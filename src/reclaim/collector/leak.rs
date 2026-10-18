use core::marker::PhantomData;
use core::mem;
use core::mem::MaybeUninit;
use core::ptr;
use core::ptr::NonNull;

#[allow(
  redundant_imports,
  reason = "`Box` is only in the prelude with `std`; the `alloc` build needs the import"
)]
use crate::alloc::Box;

use crate::reclaim::Atomic;
use crate::reclaim::CollectorWeak;
use crate::reclaim::Shared;
use crate::sync::atomic::AtomicPtr;
use crate::sync::atomic::Ordering;

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
/// This is a crate-owned wrapper so that the reclamation traits are only
/// implemented by `ptab`. It cannot be named outside the crate.
#[repr(transparent)]
pub struct Slot<T>(AtomicPtr<T>);

// SAFETY:
// - The slot owns the `Box<T>` behind its pointer, and dropping the table or
//   calling `clear` runs `T`'s destructor on whichever thread holds the slot.
// - So moving a `Slot<T>` across threads is sound when `T: Send`.
unsafe impl<T: Send> Send for Slot<T> {}

// SAFETY:
// - `read` hands out `&T` to any thread that holds `&Slot<T>`, so `T: Sync`
//   is required.
// - `write` takes `&self` and moves a `T` in from the calling thread, which
//   another thread may later drop through `clear`, so `T: Send` is required.
unsafe impl<T: Send + Sync> Sync for Slot<T> {}

// SAFETY:
// - Only `write` stores a non-null pointer, from `Box::into_raw`, so `read`
//   returns null or a pointer to a live `T`.
// - `evict` leaks the value, so it stays live. Only `clear` frees it, and its
//   contract excludes readers. `Ptr` borrows the guard for `'guard`.
// - `clear` nulls the slot before it frees the value.
// - The `Send` and `Sync` impls above bound `T` as the trait requires.
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
    let pointer: *mut T = self.0.load(order);

    Self::Shared {
      pointer,
      phantom: PhantomData,
    }
  }

  #[inline]
  fn write(&self, order: Ordering, init: impl FnOnce(&mut MaybeUninit<T>))
  where
    T: 'static,
  {
    // The slot must be empty. If it is not, the previous entry leaks, which is
    // still sound in release builds.
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
    // Null the slot so a repeated `clear` finds it empty instead of freeing twice.
    #[cfg(not(loom))]
    let old: *mut T = mem::replace(self.0.get_mut(), ptr::null_mut());
    #[cfg(loom)]
    let old: *mut T = self.0.with_mut(|ptr| mem::replace(ptr, ptr::null_mut()));

    if let Some(ptr) = NonNull::new(old) {
      // SAFETY:
      // - Only `write` stores a non-null pointer, and it comes from
      //   `Box::into_raw`.
      // - The slot is already null, so this `Box` is rebuilt and dropped once.
      // - The caller guarantees no reader still holds the pointer (`clear`'s
      //   `# Safety`).
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
///
/// `Ptr` holds a raw pointer, so it is neither `Send` nor `Sync`. A pointer
/// read under a guard should not cross threads.
#[repr(transparent)]
pub struct Ptr<'guard, T> {
  pointer: *mut T,
  phantom: PhantomData<&'guard T>,
}

// SAFETY:
// - `Ptr` is only built by `Slot::read`, so `pointer` is null or came from
//   `Box::into_raw` in `write`, and does not outlive the guard.
// - `evict` leaks the value and `clear` requires that no reader holds it, so
//   the pointer stays live for `'guard`.
unsafe impl<'guard, T> Shared<'guard, T> for Ptr<'guard, T> {
  #[inline]
  fn is_null(&self) -> bool {
    self.pointer.is_null()
  }

  #[inline]
  fn as_ref(&self) -> Option<&'guard T> {
    // SAFETY:
    // - `self.pointer` is either null or points to a fully initialized `T`
    //   written via `Atomic::write`.
    // - The pointer originates from `Box::into_raw`, so it is valid and
    //   properly aligned for `T`, and it stays live for `'guard` (see the
    //   `Shared` impl).
    // - Only shared references to `T` are created, so aliasing rules are not
    //   violated.
    unsafe { self.pointer.as_ref() }
  }
}

// The tests build slots outside a loom or shuttle model, which those atomics
// reject.
#[cfg(all(test, not(any(loom, shuttle))))]
mod tests {
  #[cfg(debug_assertions)]
  use core::any::Any;
  #[cfg(debug_assertions)]
  use core::panic::AssertUnwindSafe;
  #[cfg(debug_assertions)]
  use std::panic;

  use super::*;

  #[test]
  fn slot_is_send_and_sync_for_send_sync_entries() {
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
  fn write_then_read_then_clear() {
    let mut slot: Slot<u64> = Slot::null();
    let guard: () = ();

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(7);
    });
    assert_eq!(slot.read(Ordering::Relaxed, &guard).as_ref(), Some(&7));

    // SAFETY: the slot is the only owner of the entry written above, and clear
    // frees it exactly once.
    assert!(unsafe { slot.clear() });

    // SAFETY: the first clear emptied the slot, so this frees nothing.
    assert!(!unsafe { slot.clear() });
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

    // SAFETY: the slot is the only owner of the first entry, freed once.
    assert!(unsafe { slot.clear() });
  }

  #[test]
  #[cfg(not(debug_assertions))]
  fn write_twice_keeps_second_and_reclaims_first() {
    let mut slot: Slot<u64> = Slot::null();
    let guard: () = ();

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(1);
    });
    let first: *mut u64 = slot.0.load(Ordering::Relaxed);

    slot.write(Ordering::Relaxed, |uninit: &mut MaybeUninit<u64>| {
      uninit.write(2);
    });

    // SAFETY:
    // - `first` came from `Box::into_raw` in the first `write`.
    // - The second `write` replaced it, so nothing in the slot references it.
    // - It is freed here and never again.
    drop(unsafe { Box::from_raw(first) });

    assert_eq!(slot.read(Ordering::Relaxed, &guard).as_ref(), Some(&2));

    // SAFETY: the slot is the only owner of the remaining entry, freed once.
    assert!(unsafe { slot.clear() });
  }

  #[test]
  fn evict_removes_and_reclaims() {
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
}
