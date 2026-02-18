#![allow(dead_code)]

use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr;
use core::ptr::NonNull;
use core::sync::atomic::AtomicPtr;
use core::sync::atomic::Ordering;

#[cfg(test)]
#[inline]
pub(crate) fn try_reclaim() {
  // do nothing
}

// -----------------------------------------------------------------------------
// Atomic Ptr
// -----------------------------------------------------------------------------

/// An atomic pointer that can be safely shared between threads.
#[repr(transparent)]
pub(crate) struct Atomic<T> {
  inner: AtomicPtr<T>,
}

impl<T> Atomic<T> {
  /// Creates a null atomic pointer.
  #[inline]
  pub(crate) const fn null() -> Self {
    Self {
      inner: AtomicPtr::new(ptr::null_mut()),
    }
  }

  /// Loads a value from the pointer.
  #[inline]
  pub(crate) fn load<'guard>(&self, order: Ordering, _guard: &'guard Guard) -> Shared<'guard, T> {
    Shared {
      pointer: self.inner.load(order),
      phantom: PhantomData,
    }
  }

  /// Initializes and stores a value into the pointer.
  #[inline]
  pub(crate) fn write<F>(&self, order: Ordering, init: F)
  where
    F: FnOnce(&mut MaybeUninit<T>),
  {
    let mut uninit: Box<MaybeUninit<T>> = Box::new_uninit();

    init(&mut uninit);

    // SAFETY:
    // - The `init` closure is required to fully initialize `uninit`.
    // - After `init` returns, the value is assumed to be initialized.
    self
      .inner
      .store(Box::into_raw(unsafe { uninit.assume_init() }), order);
  }

  #[inline]
  pub(crate) fn evict(&self, order: Ordering) -> bool {
    // True to the name, we leak the entry `Box<T>` here
    !self.inner.swap(ptr::null_mut(), order).is_null()
  }

  #[inline]
  pub(crate) unsafe fn drop_in_place(&mut self) -> bool {
    if let Some(ptr) = NonNull::new(*self.inner.get_mut()) {
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
pub(crate) struct Shared<'guard, T> {
  pointer: *mut T,
  phantom: PhantomData<&'guard T>,
}

impl<'guard, T> Shared<'guard, T> {
  /// Returns `true` if the pointer is null.
  #[inline]
  pub(crate) const fn is_null(&self) -> bool {
    self.pointer.is_null()
  }

  /// Returns a shared reference to the value.
  #[inline]
  pub(crate) const fn as_ref(&self) -> Option<&'guard T> {
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

// -----------------------------------------------------------------------------
// Guard
// -----------------------------------------------------------------------------

/// A guard that does nothing.
#[non_exhaustive]
pub struct Guard;

impl Guard {
  /// Creates a new [`Guard`].
  #[inline]
  pub const fn new() -> Self {
    Self
  }
}

impl Default for Guard {
  #[inline]
  fn default() -> Self {
    Self::new()
  }
}
