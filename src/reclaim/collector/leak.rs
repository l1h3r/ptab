use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr;
use core::ptr::NonNull;
use core::sync::atomic::AtomicPtr;
use core::sync::atomic::Ordering;

use crate::reclaim::Atomic;
use crate::reclaim::CollectorWeak;
use crate::reclaim::Shared;

/// A reclamation strategy that leaks evicted entries.
pub enum Leak {}

impl CollectorWeak for Leak {
  type Guard = ();
  type Atomic<T> = AtomicPtr<T>;

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

impl<T> Atomic<T> for AtomicPtr<T> {
  type Guard = ();

  #[rustfmt::skip]
  type Shared<'guard> = Ptr<'guard, T>
  where
    T: 'guard;

  #[inline]
  fn null() -> Self {
    Self::new(ptr::null_mut())
  }

  #[inline]
  fn read<'guard>(&self, order: Ordering, _guard: &'guard Self::Guard) -> Self::Shared<'guard> {
    Self::Shared {
      pointer: self.load(order),
      phantom: PhantomData,
    }
  }

  #[inline]
  fn write(&self, order: Ordering, init: impl FnOnce(&mut MaybeUninit<T>))
  where
    T: 'static,
  {
    let mut uninit: Box<MaybeUninit<T>> = Box::new_uninit();

    init(&mut uninit);

    // SAFETY:
    // - The `init` closure is required to fully initialize `uninit`.
    // - After `init` returns, the value is assumed to be initialized.
    self.store(Box::into_raw(unsafe { uninit.assume_init() }), order);
  }

  #[inline]
  fn evict(&self, order: Ordering) -> bool {
    // True to the name, we leak the entry `Box<T>` here
    !self.swap(ptr::null_mut(), order).is_null()
  }

  #[inline]
  unsafe fn clear(&mut self) -> bool {
    if let Some(ptr) = NonNull::new(*self.get_mut()) {
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

impl<'guard, T> Shared<'guard, T> for Ptr<'guard, T> {
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
    //   properly aligned for `T`.
    // - Only shared references to `T` are created, so aliasing rules are not
    //   violated.
    unsafe { self.pointer.as_ref() }
  }
}
