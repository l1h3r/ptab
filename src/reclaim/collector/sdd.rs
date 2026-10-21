use core::mem;
use core::mem::MaybeUninit;

use crate::reclaim::Atomic;
use crate::reclaim::Collector;
use crate::reclaim::CollectorWeak;
use crate::reclaim::Shared;
use crate::sync::atomic::Ordering;

const NO_TAG: sdd::Tag = sdd::Tag::None;

/// Retires an entry that `write` found in a slot it expected to be empty.
///
/// Kept out of line so the hot path pays one branch and nothing else: dropping
/// an `Owned` pulls sdd's thread-local collector into the caller otherwise.
#[cold]
#[inline(never)]
fn retire_occupied<T>(old: Option<sdd::Owned<T>>)
where
  T: 'static,
{
  drop(old);
}

/// A reclamation strategy based on [`sdd`].
///
/// [`sdd`]: https://crates.io/crates/sdd
pub enum Sdd {}

// SAFETY: `evict` drops the swapped-out `sdd::Owned`, which hands the entry to
// `sdd`'s collector. `sdd` runs its destructor once no guard can still observe
// it, as the collector advances, so evicted entries are not permanently leaked.
unsafe impl Collector for Sdd {}

impl CollectorWeak for Sdd {
  type Guard = sdd::Guard;
  type Atomic<T> = SddAtomic<T>;

  #[inline]
  fn guard() -> Self::Guard {
    sdd::Guard::new()
  }

  #[inline]
  fn flush() {
    // sdd triggers reclamation after we've observed three new epochs; we
    // observe one extra since we might lag behind the global value.
    const EPOCH: usize = 4;

    for _ in 0..EPOCH {
      Self::guard().accelerate();
    }
  }
}

// -----------------------------------------------------------------------------
// Atomic Ptr
// -----------------------------------------------------------------------------

/// An atomic pointer to an entry owned by [`sdd`].
///
/// A crate-private wrapper, so the reclamation traits are implemented only on
/// types this crate builds. A bare `sdd::AtomicOwned` is not an [`Atomic`]; the
/// doctests for that are in the `sdd_atomic` module below.
#[repr(transparent)]
pub struct SddAtomic<T>(sdd::AtomicOwned<T>);

// SAFETY:
// - `read` loads the `sdd::AtomicOwned` under a `sdd::Guard`, giving a pointer
//   that is null or a valid `T` that sdd keeps live while the guard is.
// - `SddAtomic` is `repr(transparent)` over `sdd::AtomicOwned`, the size and
//   alignment of `usize` (const-asserted in `Array::new_uninit`).
// - `evict` swaps out the `sdd::Owned` and drops it, which hands the entry to
//   sdd's collector for deferred destruction.
// - Every `&self` method goes through `sdd::AtomicOwned`, which is `Sync` for
//   `T: Send + Sync`, so they are sound to call from several threads then.
// - `sdd::AtomicOwned` is `Send` only for `T: Send + Sync`, so the move
//   obligation for `T: Send` rests on ownership instead: a move needs `&mut`
//   access, so no guard borrows the slot. A retired entry is owned by sdd and
//   dropped on any thread, which needs only `T: Send`.
unsafe impl<T> Atomic<T> for SddAtomic<T> {
  type Guard = sdd::Guard;

  #[rustfmt::skip]
  type Shared<'guard> = SddPtr<'guard, T>
  where
    T: 'guard;

  #[inline]
  fn null() -> Self {
    Self(sdd::AtomicOwned::null())
  }

  #[inline]
  fn read<'guard>(&self, order: Ordering, guard: &'guard Self::Guard) -> Self::Shared<'guard> {
    SddPtr(self.0.load(order, guard))
  }

  #[inline]
  fn write(&self, order: Ordering, init: impl FnOnce(&mut MaybeUninit<T>))
  where
    T: 'static,
  {
    let value: sdd::Owned<T> = sdd::Owned::new_with(|| {
      let mut uninit: MaybeUninit<T> = MaybeUninit::uninit();

      init(&mut uninit);

      // SAFETY:
      // - The `init` closure is required to fully initialize `uninit`.
      // - After `init` returns, the value is assumed to be initialized.
      unsafe { uninit.assume_init() }
    });

    let old: (Option<sdd::Owned<T>>, sdd::Tag) = self.0.swap((Some(value), NO_TAG), order);

    // The slot must be empty. If it is not, sdd retires the previous entry
    // through its epoch, so this is still sound in release builds.
    debug_assert!(old.0.is_none(), "Atomic<T> is occupied!");
    debug_assert!(old.1 == NO_TAG, "Atomic<T> is tagged!");

    if old.0.is_some() {
      retire_occupied(old.0);
    }
  }

  #[inline]
  fn evict(&self, order: Ordering) -> bool {
    self.0.swap((None, NO_TAG), order).0.is_some()
  }

  #[inline]
  unsafe fn clear(&mut self) -> bool {
    let entry: sdd::AtomicOwned<T> = mem::take(&mut self.0);

    if let Some(value) = entry.into_owned(Ordering::Relaxed) {
      // SAFETY:
      // - `entry.into_owned` transfers exclusive ownership of the value.
      // - If `Some(value)` is returned, we are the unique owner.
      // - Calling `drop_in_place` drops the contained `T` exactly once and
      //   reclaims the associated allocation.
      unsafe {
        value.drop_in_place();
      }

      true
    } else {
      false
    }
  }
}

// The doctests live here because private items can't carry doctests under
// `rustdoc::private_doc_tests`.
#[cfg(doctest)]
mod sdd_atomic {
  /// A bare `sdd::AtomicOwned` does not implement `ptab::memory::Atomic`.
  ///
  /// ```compile_fail,E0277
  /// use core::sync::atomic::Ordering;
  /// use ptab::memory::Atomic;
  ///
  /// let ptr: sdd::AtomicOwned<u8> = Atomic::null();
  /// let guard: sdd::Guard = sdd::Guard::new();
  /// let _ = Atomic::read(&ptr, Ordering::Acquire, &guard);
  /// ```
  ///
  /// ```
  /// use core::sync::atomic::Ordering;
  /// use ptab::memory::Atomic;
  /// use ptab::memory::CollectorWeak;
  /// use ptab::memory::collector::Sdd;
  ///
  /// let ptr: <Sdd as CollectorWeak>::Atomic<u8> = Atomic::null();
  /// let guard: sdd::Guard = sdd::Guard::new();
  /// let _ = Atomic::read(&ptr, Ordering::Acquire, &guard);
  /// ```
  const NOT_ATOMIC_OWNED: () = ();
}

// -----------------------------------------------------------------------------
// Shared Ptr
// -----------------------------------------------------------------------------

/// A pointer to an entry owned by [`sdd`], valid for the guard lifetime.
#[repr(transparent)]
pub struct SddPtr<'guard, T>(sdd::Ptr<'guard, T>);

// SAFETY: `SddPtr` is only built by `SddAtomic::read`, so it is null or points to
// a `T` that sdd keeps live for `'guard`. `is_null` and `as_ref` both test the
// same inner pointer, so they agree.
unsafe impl<'guard, T> Shared<'guard, T> for SddPtr<'guard, T> {
  #[inline]
  fn is_null(&self) -> bool {
    self.0.is_null()
  }

  #[inline]
  fn as_ref(&self) -> Option<&'guard T> {
    // SAFETY:
    // - `self` is either null or points to a fully initialized `T` written via
    //   `Atomic::write`.
    // - The pointer comes from the `sdd::Owned` that `write` stored, so it is
    //   valid and aligned for `T`, and sdd keeps it alive for `'guard`.
    // - Only shared references to `T` are created, so aliasing rules are not
    //   violated.
    // - `self` has no tag bits set: `SddPtr` is only built by `SddAtomic::read`,
    //   and every store into `SddAtomic` uses `NO_TAG`.
    unsafe { self.0.as_ref_unchecked() }
  }
}
#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn atomic_is_send_and_sync_for_send_sync_entries() {
    fn assert_send_sync<S: Send + Sync>() {}

    assert_send_sync::<SddAtomic<u64>>();
  }

  #[test]
  fn null_reads_as_none() {
    let slot: SddAtomic<u64> = SddAtomic::null();
    let guard: sdd::Guard = sdd::Guard::new();

    assert!(slot.read(Ordering::Relaxed, &guard).is_null());
    assert!(slot.read(Ordering::Relaxed, &guard).as_ref().is_none());
  }

  #[test]
  fn write_read_evict_clear() {
    let mut slot: SddAtomic<u64> = SddAtomic::null();
    let guard: sdd::Guard = sdd::Guard::new();

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(5);
    });
    assert_eq!(slot.read(Ordering::Relaxed, &guard).as_ref(), Some(&5));

    assert!(slot.evict(Ordering::Relaxed));
    assert!(!slot.evict(Ordering::Relaxed));

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(6);
    });

    // SAFETY: the slot is the only owner of the entry, cleared once.
    assert!(unsafe { slot.clear() });
    // SAFETY: the first `clear` left the slot empty, so this drops nothing.
    assert!(!unsafe { slot.clear() });
  }

  #[test]
  #[cfg(debug_assertions)]
  #[should_panic(expected = "occupied")]
  fn write_twice() {
    let slot: SddAtomic<u64> = SddAtomic::null();

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(1);
    });
    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(2);
    });
  }

  #[test]
  #[cfg(not(debug_assertions))]
  fn write_twice() {
    let slot: SddAtomic<u64> = SddAtomic::null();
    let guard: sdd::Guard = sdd::Guard::new();

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(1);
    });
    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(2);
    });

    assert_eq!(slot.read(Ordering::Relaxed, &guard).as_ref(), Some(&2));
  }
}
