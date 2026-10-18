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
/// This is a crate-owned wrapper so that the reclamation traits are only
/// implemented by `ptab`. Its `Send` and `Sync` impls come from
/// `sdd::AtomicOwned<T>`, which requires `T: Send + Sync`.
#[repr(transparent)]
pub struct SddAtomic<T>(sdd::AtomicOwned<T>);

// SAFETY:
// - `SddAtomic<T>` has no impl of its own; it is `Send` and `Sync` exactly when
//   `sdd::AtomicOwned<T>` is, which sdd implements for `T: Send + Sync`.
// - The `Atomic` impl below never adds a way to share or move the entry.
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

// -----------------------------------------------------------------------------
// Shared Ptr
// -----------------------------------------------------------------------------

/// A pointer to an entry owned by [`sdd`], valid for the guard lifetime.
#[repr(transparent)]
pub struct SddPtr<'guard, T>(sdd::Ptr<'guard, T>);

// SAFETY:
// - `SddPtr` is only built by `SddAtomic::read`, from an entry written with no
//   tag bits set. Tags are never set through this crate's API.
// - `as_ref` only returns `&'guard T` for a pointer that sdd keeps alive for the
//   guard.
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
    // - The pointer originates from `Box::into_raw`, so it is valid and
    //   properly aligned for `T`.
    // - Only shared references to `T` are created, so aliasing rules are not
    //   violated.
    // - `self` does not have any tag bits set.
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
