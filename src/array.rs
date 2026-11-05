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

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(all(test, not(any(loom, shuttle))))]
mod tests;

// -----------------------------------------------------------------------------
// Kani proofs
// -----------------------------------------------------------------------------

#[cfg(kani)]
mod proofs;

// -----------------------------------------------------------------------------
// Implementation
// -----------------------------------------------------------------------------

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
  /// Creates an array, calling `init` with each index in ascending order.
  #[inline]
  pub(crate) fn new<F>(init: F) -> Self
  where
    F: Fn(usize, &mut MaybeUninit<T>),
  {
    let this: Array<MaybeUninit<T>, P> = Self::new_uninit();

    for index in 0..P::LENGTH.as_usize() {
      // SAFETY:
      // - `index` is strictly less than `P::LENGTH`
      // - the allocation performed by `new_uninit` reserves space for exactly
      //   `P::LENGTH` contiguous elements
      // - the pointer returned by `as_non_null` is properly aligned for `T`
      // - this function has exclusive access to the allocation, so creating a
      //   unique mutable reference is sound
      init(index, unsafe { this.as_non_null().add(index).as_mut() });
    }

    // SAFETY: the loop in `new` initializes all `P::LENGTH` elements of the
    //         allocation exactly once.
    unsafe { this.assume_init() }
  }

  /// Constructs a new array with uninitialized contents.
  #[inline]
  pub(crate) fn new_uninit() -> Array<MaybeUninit<T>, P> {
    const {
      assert!(
        size_of::<T>() == size_of::<usize>() && align_of::<T>() == align_of::<usize>(),
        "`Atomic<T>` must have the size and alignment of `usize`"
      );
    }

    // SAFETY:
    // - `P::LAYOUT` describes a non-zero-sized allocation
    // - `P::LAYOUT` fits `P::LENGTH` values of `usize`, and `T` has the size
    //   and alignment of `usize`
    let raw: *mut u8 = unsafe { alloc(P::LAYOUT) };

    Array {
      nonnull: match NonNull::new(raw.cast()) {
        Some(ptr) => ptr,
        None => handle_alloc_error(P::LAYOUT),
      },
      phantom: PhantomData,
    }
  }

  /// Returns a `NonNull` pointer to the array's buffer.
  #[inline]
  pub(crate) const fn as_non_null(&self) -> NonNull<T> {
    self.nonnull
  }

  /// Returns a raw pointer to the array's buffer.
  #[cfg(all(any(test, kani), not(any(loom, shuttle))))]
  #[inline]
  pub(crate) const fn as_ptr(&self) -> *const T {
    self.as_non_null().as_ptr()
  }

  /// Returns a raw mutable pointer to the array's buffer.
  #[inline]
  pub(crate) const fn as_mut_ptr(&self) -> *mut T {
    self.as_non_null().as_ptr()
  }

  /// Extracts a slice containing the entire array.
  #[cfg(all(any(test, kani), not(any(loom, shuttle))))]
  #[inline]
  pub(crate) const fn as_slice(&self) -> &[T] {
    // SAFETY:
    // - the allocation contains `P::LENGTH` contiguous elements of `T`
    // - `Array<T, P>` contains only initialized elements
    // - the pointer is valid for reads for the entire range
    unsafe { slice::from_raw_parts(self.as_ptr(), P::LENGTH.as_usize()) }
  }

  /// Extracts a mutable slice of the entire array.
  #[inline]
  pub(crate) const fn as_mut_slice(&mut self) -> &mut [T] {
    // SAFETY:
    // - the allocation contains `P::LENGTH` contiguous elements of `T`
    // - `Array<T, P>` contains only initialized elements
    // - `&mut self` gives unique access to the allocation
    unsafe { slice::from_raw_parts_mut(self.as_mut_ptr(), P::LENGTH.as_usize()) }
  }

  /// Returns a reference to the element at `index`.
  #[inline]
  pub(crate) const fn get(&self, index: Concrete<P>) -> &T {
    // SAFETY: `Concrete<P>` bounds the index by `P::LENGTH`.
    unsafe { self.get_unchecked(index.get()) }
  }

  /// Returns a reference to the element at `index`, without doing bounds
  /// checking.
  ///
  /// # Safety
  ///
  /// `index` must be strictly less than `P::LENGTH`. Passing an out-of-bounds
  /// index results in undefined behavior, even if nothing uses the returned
  /// reference.
  #[inline]
  pub(crate) const unsafe fn get_unchecked(&self, index: usize) -> &T {
    debug_assert!(
      index < P::LENGTH.as_usize(),
      "Array::get_unchecked requires that the index is in bounds",
    );

    // SAFETY:
    // - the caller promises `index < P::LENGTH`
    // - the allocation has room for `P::LENGTH` contiguous elements
    // - the pointer is properly aligned and valid for reads
    unsafe { self.as_non_null().add(index).as_ref() }
  }
}

impl<T, P> Array<MaybeUninit<T>, P>
where
  P: Params + ?Sized,
{
  /// Converts to `Array<T, P>`.
  ///
  /// # Safety
  ///
  /// Every element in the allocation must be fully initialized. Converting
  /// with an uninitialized element results in immediate undefined behavior.
  #[inline]
  pub(crate) unsafe fn assume_init(self) -> Array<T, P> {
    // SAFETY:
    // - the caller promises `self` contains only initialized elements
    // - `Array<MaybeUninit<T>, P>` and `Array<T, P>` have identical layout
    // - `ManuallyDrop` prevents `self` from dropping, so nothing frees the
    //   allocation during the conversion
    Array {
      nonnull: ManuallyDrop::new(self).as_non_null().cast(),
      phantom: PhantomData,
    }
  }
}

impl<T, P> Drop for Array<T, P>
where
  P: Params + ?Sized,
{
  fn drop(&mut self) {
    // SAFETY:
    // - `new_uninit` made the allocation with `alloc(P::LAYOUT)`
    // - `P::LAYOUT` is the exact layout used for allocation
    // - `self.nonnull` still points to the original allocation
    unsafe {
      dealloc(self.as_non_null().cast().as_ptr(), P::LAYOUT);
    }
  }
}
