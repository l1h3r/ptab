use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr;
use core::ptr::NonNull;
use core::sync::atomic::AtomicPtr;
use core::sync::atomic::Ordering;

#[allow(
  redundant_imports,
  reason = "`Box` is only in the prelude with `std`; the `alloc` build needs the import"
)]
use crate::alloc::Box;

use crate::reclaim::Atomic;
use crate::reclaim::CollectorWeak;
use crate::reclaim::Shared;

/// A reclamation strategy that leaks evicted entries.
pub enum Leak {}

impl CollectorWeak for Leak {
  type Guard = ();
  type Atomic<T> = Slot<T>;

  #[inline]
  fn guard() -> Self::Guard {
    // do nothing
  }

  #[inline]
  fn flush() {
    // do nothing
  }
}

// -----------------------------------------------------------------------------
// Atomic Ptr
// -----------------------------------------------------------------------------

/// An atomic pointer to a leaked entry.
///
/// A crate-private wrapper, so the reclamation traits are implemented only on
/// types this crate builds.
#[repr(transparent)]
pub struct Slot<T>(AtomicPtr<T>);

// SAFETY:
// - `read` loads the `AtomicPtr`, which holds null or a `Box::into_raw` pointer
//   from `write`; `evict` never frees it, so it stays live past any guard.
// - `Slot` is `repr(transparent)` over `AtomicPtr`, the size and alignment of
//   `usize`.
// - `evict` swaps the pointer out and leaks the entry, so no reader can see it
//   freed.
// - `AtomicPtr` operations are thread-safe, so the `&self` methods are sound to
//   call from several threads.
// - `AtomicPtr<T>` is `Send` for every `T`, but `Slot` also owns the `T` that
//   `clear` frees. That `T` moves and drops as a `Box<T>` does, so `Slot` is
//   sound to move to and drop on another thread when `T: Send`.
unsafe impl<T> Atomic<T> for Slot<T> {
  type Guard = ();

  #[rustfmt::skip]
  type Shared<'guard> = Ptr<'guard, T>
  where
    T: 'guard;

  #[inline]
  fn null() -> Self {
    Self(AtomicPtr::new(ptr::null_mut()))
  }

  #[inline]
  fn read<'guard>(&self, order: Ordering, _guard: &'guard Self::Guard) -> Self::Shared<'guard> {
    Self::Shared {
      pointer: self.0.load(order),
      phantom: PhantomData,
    }
  }

  #[inline]
  fn write(&self, order: Ordering, init: impl FnOnce(&mut MaybeUninit<T>))
  where
    T: 'static,
  {
    debug_assert!(
      self.0.load(Ordering::Relaxed).is_null(),
      "Atomic<T> is occupied!"
    );

    let mut uninit: Box<MaybeUninit<T>> = Box::new_uninit();

    init(&mut uninit);

    // SAFETY:
    // - The `init` closure is required to fully initialize `uninit`.
    // - After `init` returns, the value is assumed to be initialized.
    self
      .0
      .store(Box::into_raw(unsafe { uninit.assume_init() }), order);
  }

  #[inline]
  fn evict(&self, order: Ordering) -> bool {
    // True to the name, we leak the entry `Box<T>` here
    !self.0.swap(ptr::null_mut(), order).is_null()
  }

  #[inline]
  unsafe fn clear(&mut self) -> bool {
    if let Some(ptr) = NonNull::new(*self.0.get_mut()) {
      // SAFETY:
      // - `ptr` was previously created by `Box::into_raw`, so it originated
      //   from a valid `Box<T>` allocation.
      // - We have exclusive access via `&mut self`, so no concurrent access to
      //   the pointer can occur.
      // - Reconstructing the `Box<T>` transfers ownership back and will drop
      //   the value and free the allocation exactly once.
      drop(unsafe { Box::from_raw(ptr.as_ptr()) });
      true
    } else {
      false
    }
  }
}

// -----------------------------------------------------------------------------
// Shared Ptr
// -----------------------------------------------------------------------------

/// A pointer to an unprotected object.
#[repr(transparent)]
pub struct Ptr<'guard, T> {
  pointer: *mut T,
  phantom: PhantomData<&'guard T>,
}

// SAFETY: `Ptr` is only built by `Slot::read`, so `pointer` is null or points to
// a leaked `Box<T>` that is never freed while the table is shared. `is_null`
// and `as_ref` agree because both test `pointer`.
unsafe impl<'guard, T> Shared<'guard, T> for Ptr<'guard, T> {
  #[inline]
  fn is_null(&self) -> bool {
    self.pointer.is_null()
  }

  #[inline]
  fn as_ref(&self) -> Option<&'guard T> {
    // SAFETY:
    // - `Ptr` is only built by `Slot::read`, so `self.pointer` is null or
    //   came from `Box::into_raw` in `Slot::write` and points to a fully
    //   initialized `T`.
    // - `evict` leaks the entry and `clear` requires that no reader holds the
    //   pointer, so the `T` stays live for `'guard`.
    // - Only shared references to `T` are created, so aliasing rules are not
    //   violated.
    unsafe { self.pointer.as_ref() }
  }
}

#[cfg(test)]
mod tests {
  #[cfg(debug_assertions)]
  use core::any::Any;
  #[cfg(debug_assertions)]
  use core::panic::AssertUnwindSafe;
  #[cfg(debug_assertions)]
  use std::panic;

  use super::*;

  #[test]
  fn slot_is_send_and_sync() {
    fn assert_send_sync<S: Send + Sync>() {}

    assert_send_sync::<Slot<u64>>();
  }

  #[test]
  fn null_reads_as_none() {
    let slot: Slot<u64> = Slot::null();
    let guard: () = ();

    assert!(slot.read(Ordering::Relaxed, &guard).is_null());
    assert!(slot.read(Ordering::Relaxed, &guard).as_ref().is_none());
  }

  #[test]
  fn write_read_clear() {
    let mut slot: Slot<u64> = Slot::null();
    let guard: () = ();

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(7);
    });
    assert_eq!(slot.read(Ordering::Relaxed, &guard).as_ref(), Some(&7));

    // SAFETY: the slot is the only owner of the entry written above, and it is
    // cleared once.
    assert!(unsafe { slot.clear() });
  }

  #[test]
  fn clear_empty() {
    let mut slot: Slot<u64> = Slot::null();

    // SAFETY: the slot is empty, so this frees nothing.
    assert!(!unsafe { slot.clear() });
  }

  #[test]
  fn evict_leaks_the_entry() {
    let slot: Slot<u64> = Slot::null();
    let guard: () = ();

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(3);
    });
    let first: *mut u64 = slot.0.load(Ordering::Relaxed);

    assert!(slot.evict(Ordering::Relaxed));
    assert!(!slot.evict(Ordering::Relaxed));
    assert!(slot.read(Ordering::Relaxed, &guard).is_null());

    // SAFETY:
    // - `first` came from `Box::into_raw` in `write`.
    // - `evict` swapped it out, so the slot no longer references it.
    // - It is freed here and never again.
    drop(unsafe { Box::from_raw(first) });
  }

  #[test]
  #[cfg(debug_assertions)]
  fn write_twice() {
    let mut slot: Slot<u64> = Slot::null();
    let guard: () = ();

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(1);
    });

    let result: Result<(), Box<dyn Any + Send>> = panic::catch_unwind(AssertUnwindSafe(|| {
      slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
        uninit.write(2);
      });
    }));

    let message: &str = result.unwrap_err().downcast_ref::<&str>().copied().unwrap();

    assert!(message.contains("occupied"));
    assert_eq!(slot.read(Ordering::Relaxed, &guard).as_ref(), Some(&1));

    // SAFETY: the slot is the only owner of the first entry, cleared once.
    assert!(unsafe { slot.clear() });
  }

  #[test]
  #[cfg(not(debug_assertions))]
  fn write_twice() {
    let mut slot: Slot<u64> = Slot::null();
    let guard: () = ();

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(1);
    });
    let first: *mut u64 = slot.0.load(Ordering::Relaxed);

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(2);
    });
    assert_eq!(slot.read(Ordering::Relaxed, &guard).as_ref(), Some(&2));

    // SAFETY:
    // - `first` came from `Box::into_raw` in the first `write`.
    // - The second `write` replaced it, so nothing in the slot references it.
    drop(unsafe { Box::from_raw(first) });

    // SAFETY: the slot is the only owner of the second entry, cleared once.
    assert!(unsafe { slot.clear() });
  }
}
