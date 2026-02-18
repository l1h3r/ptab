use core::hint;
use core::mem;
use core::mem::MaybeUninit;
use core::sync::atomic::Ordering;

#[cfg(test)]
#[inline]
pub(crate) fn try_reclaim() {
  // sdd triggers reclamation after we've observed three new epochs
  sdd::Guard::new().accelerate();
  sdd::Guard::new().accelerate();
  sdd::Guard::new().accelerate();

  drop(sdd::Guard::new());
}

// -----------------------------------------------------------------------------
// Atomic Ptr
// -----------------------------------------------------------------------------

/// An atomic pointer that can be safely shared between threads.
#[repr(transparent)]
pub(crate) struct Atomic<T> {
  inner: sdd::AtomicOwned<T>,
}

impl<T> Atomic<T> {
  /// Creates a null atomic pointer.
  #[inline]
  pub(crate) const fn null() -> Self {
    Self {
      inner: sdd::AtomicOwned::null(),
    }
  }

  /// Loads a value from the pointer.
  #[inline]
  pub(crate) fn load<'guard>(&self, order: Ordering, guard: &'guard Guard) -> Shared<'guard, T> {
    Shared {
      inner: self.inner.load(order, &guard.inner),
    }
  }

  /// Initializes and stores a value into the pointer.
  #[inline]
  pub(crate) fn write<F>(&self, order: Ordering, init: F)
  where
    F: FnOnce(&mut MaybeUninit<T>),
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

    let old: (Option<sdd::Owned<T>>, sdd::Tag) =
      self.inner.swap((Some(value), sdd::Tag::None), order);

    debug_assert!(old.0.is_none(), "Atomic<T> is occupied!");
    debug_assert!(old.1 == sdd::Tag::None, "Atomic<T> is tagged!");

    // SAFETY:
    // - Slot allocation guarantees the slot is empty before calling `write`.
    // - Therefore `old.0` must be `None` and `old.1` must be `Tag::None`.
    unsafe {
      hint::assert_unchecked(old.0.is_none());
      hint::assert_unchecked(old.1 == sdd::Tag::None);
    }
  }

  #[inline]
  pub(crate) fn evict(&self, order: Ordering) -> bool {
    self.inner.swap((None, sdd::Tag::None), order).0.is_some()
  }

  #[inline]
  pub(crate) unsafe fn drop_in_place(&mut self) -> bool {
    let entry: sdd::AtomicOwned<T> = mem::take(&mut self.inner);

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

/// A pointer to an object protected by the epoch GC.
///
/// The pointer is valid for use only during the lifetime `'guard`.
#[repr(transparent)]
pub(crate) struct Shared<'guard, T> {
  inner: sdd::Ptr<'guard, T>,
}

impl<'guard, T> Shared<'guard, T> {
  /// Returns `true` if the pointer is null.
  #[inline]
  pub(crate) fn is_null(&self) -> bool {
    self.inner.is_null()
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
    // - `self.pointer` does not have any tag bits set.
    unsafe { self.inner.as_ref_unchecked() }
  }
}

// -----------------------------------------------------------------------------
// Guard
// -----------------------------------------------------------------------------

/// A guard that keeps the current thread pinned.
#[repr(transparent)]
pub struct Guard {
  inner: sdd::Guard,
}

impl Guard {
  /// Creates a new [`Guard`].
  #[inline]
  pub fn new() -> Self {
    Self {
      inner: sdd::Guard::new(),
    }
  }
}

impl Default for Guard {
  #[inline]
  fn default() -> Self {
    Self::new()
  }
}
