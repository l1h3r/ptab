//! Cache-aligned array allocation.
//!
//! Provides [`Array`], the backing storage for table slots.

use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::mem::MaybeUninit;
use core::ptr::NonNull;
use core::slice;

use crate::alloc::alloc;
use crate::alloc::dealloc;
use crate::alloc::handle_alloc_error;
use crate::index::Concrete;
use crate::params::Params;
use crate::params::ParamsExt;

/// A fixed-size array with cache-line-aligned allocation.
#[repr(transparent)]
pub(crate) struct Array<T, P>
where
  P: Params + ?Sized,
{
  nonnull: NonNull<T>,
  phantom: PhantomData<P>,
}

impl<T, P> Array<T, P>
where
  P: Params + ?Sized,
{
  /// Creates a new array, initializing each element with the given function.
  #[inline]
  pub(crate) fn new<F>(init: F) -> Self
  where
    F: Fn(usize, &mut MaybeUninit<T>),
  {
    let this: Array<MaybeUninit<T>, P> = Self::new_uninit();

    let mut index: usize = 0;

    while index < P::LENGTH.as_usize() {
      // SAFETY: `index < P::LENGTH` and allocation holds `P::LENGTH` elements.
      let mut ptr: NonNull<MaybeUninit<T>> = unsafe { this.as_nonnull().add(index) };

      // SAFETY: Pointer is valid and aligned; we have exclusive access.
      let uninit: &mut MaybeUninit<T> = unsafe { ptr.as_mut() };

      init(index, uninit);

      index += 1;
    }

    // SAFETY: All `P::LENGTH` elements initialized by the loop.
    unsafe { this.assume_init() }
  }

  /// Creates a new array with all bytes zeroed.
  #[allow(dead_code, reason = "not used with loom tests")]
  #[inline]
  pub(crate) fn new_zeroed() -> Array<MaybeUninit<T>, P> {
    let this: Array<MaybeUninit<T>, P> = Self::new_uninit();

    // SAFETY: Allocation holds `P::LENGTH` elements; zeroing `MaybeUninit` is valid.
    unsafe {
      this.as_nonnull().write_bytes(0, P::LENGTH.as_usize());
    }

    this
  }

  /// Creates a new array without initializing its contents.
  #[inline]
  pub(crate) fn new_uninit() -> Array<MaybeUninit<T>, P> {
    P::validate();

    // SAFETY: `P::validate()` ensures `P::LAYOUT` has non-zero size.
    let raw: *mut u8 = unsafe { alloc(P::LAYOUT) };

    Array {
      nonnull: match NonNull::new(raw.cast()) {
        Some(ptr) => ptr,
        None => handle_alloc_error(P::LAYOUT),
      },
      phantom: PhantomData,
    }
  }

  /// Returns a non-null pointer to the array.
  #[inline]
  pub(crate) const fn as_nonnull(&self) -> NonNull<T> {
    self.nonnull
  }

  /// Returns a raw pointer to the array.
  #[inline]
  pub(crate) const fn as_ptr(&self) -> *const T {
    self.as_nonnull().as_ptr()
  }

  /// Returns a raw mutable pointer to the array.
  #[inline]
  pub(crate) const fn as_mut_ptr(&self) -> *mut T {
    self.as_nonnull().as_ptr()
  }

  #[inline]
  pub(crate) const fn as_slice(&self) -> &[T] {
    // SAFETY: Contiguous allocation of `P::LENGTH` initialized elements.
    unsafe { slice::from_raw_parts(self.as_ptr(), P::LENGTH.as_usize()) }
  }

  #[inline]
  pub(crate) const fn as_mut_slice(&mut self) -> &mut [T] {
    // SAFETY: Contiguous allocation of `P::LENGTH` initialized elements.
    unsafe { slice::from_raw_parts_mut(self.as_mut_ptr(), P::LENGTH.as_usize()) }
  }

  /// Returns a reference to the element at the given index.
  #[inline]
  pub(crate) const fn get(&self, index: Concrete<P>) -> &T {
    // SAFETY: `Concrete<P>` indices are always less than `P::LENGTH`.
    unsafe { self.get_unchecked(index.get()) }
  }

  /// Returns a reference to the element at `index` without bounds checking.
  ///
  /// # Safety
  ///
  /// `index` must be less than [`P::LENGTH`].
  ///
  /// [`P::LENGTH`]: Params::LENGTH
  #[inline]
  pub(crate) const unsafe fn get_unchecked(&self, index: usize) -> &T {
    debug_assert!(
      index < P::LENGTH.as_usize(),
      "Array::get_unchecked requires that the index is in bounds",
    );

    // SAFETY: Caller guarantees `index < P::LENGTH`.
    unsafe { self.as_nonnull().add(index).as_ref() }
  }
}

impl<T, P> Array<MaybeUninit<T>, P>
where
  P: Params + ?Sized,
{
  /// Converts to an initialized array.
  ///
  /// # Safety
  ///
  /// All [`P::LENGTH`] elements must be initialized.
  ///
  /// [`P::LENGTH`]: Params::LENGTH
  #[inline]
  pub(crate) unsafe fn assume_init(self) -> Array<T, P> {
    Array {
      // Prevent drop from running on `self` (would deallocate).
      nonnull: ManuallyDrop::new(self).as_nonnull().cast(),
      phantom: PhantomData,
    }
  }
}

impl<T, P> Drop for Array<T, P>
where
  P: Params + ?Sized,
{
  fn drop(&mut self) {
    // SAFETY: Allocated with `P::LAYOUT` in `new_uninit`.
    unsafe {
      dealloc(self.as_nonnull().cast().as_ptr(), P::LAYOUT);
    }
  }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
  use crate::array::Array;
  use crate::params::CACHE_LINE;
  use crate::params::ConstParams;
  use crate::utils::each_capacity;

  #[cfg_attr(loom, ignore = "loom does not run this test")]
  #[test]
  fn alignment() {
    each_capacity!({
      // SAFETY: `0` is a valid bit pattern for `usize`.
      let array: Array<usize, P> = unsafe { Array::new_zeroed().assume_init() };

      // TODO: ptr::is_aligned_to once stable
      assert_eq!(array.as_ptr().addr() & (CACHE_LINE - 1), 0);
    });
  }

  #[cfg_attr(loom, ignore = "loom does not run this test")]
  #[test]
  fn slice_representation() {
    let mut array: Array<usize, ConstParams<64>> = Array::new(|index, uninit| {
      uninit.write(index);
    });

    assert_eq!(array.as_slice().len(), 64);
    assert_eq!(array.as_mut_slice().len(), 64);

    for (index, value) in array.as_slice().iter().enumerate() {
      assert_eq!(*value, index);
    }

    for value in array.as_mut_slice() {
      *value += 1;
    }

    for (index, value) in array.as_slice().iter().enumerate() {
      assert_eq!(*value, index + 1);
    }
  }
}
