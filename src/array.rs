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
  /// Creates an array where each element is produced by calling `init` with
  /// that element’s index while walking forward through the array.
  #[inline]
  pub(crate) fn new<F>(init: F) -> Self
  where
    F: Fn(usize, &mut MaybeUninit<T>),
  {
    let this: Array<MaybeUninit<T>, P> = Self::new_uninit();

    for index in 0..P::LENGTH.as_usize() {
      // SAFETY:
      // - `index` is strictly less than `P::LENGTH`.
      // - The allocation performed by `new_uninit` reserves space for exactly
      //   `P::LENGTH` contiguous elements.
      // - The pointer returned by `as_non_null` is properly aligned for `T`.
      // - We have exclusive access to the allocation, so creating a unique
      //   mutable reference is sound.
      init(index, unsafe { this.as_non_null().add(index).as_mut() });
    }

    // SAFETY: The loop above initializes every element in the allocation
    //         exactly once, so all `P::LENGTH` elements are initialized.
    unsafe { this.assume_init() }
  }

  /// Constructs a new array with uninitialized contents.
  #[inline]
  pub(crate) fn new_uninit() -> Array<MaybeUninit<T>, P> {
    // SAFETY:
    // - `P::LAYOUT` describes a non-zero-sized allocation.
    // - Its size and alignment have been validated when constructing the
    //   associated `Params` implementation.
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
  #[cfg(test)]
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
  #[cfg(test)]
  #[inline]
  pub(crate) const fn as_slice(&self) -> &[T] {
    // SAFETY:
    // - The allocation contains `P::LENGTH` contiguous elements of `T`.
    // - For `Array<T, P>`, all elements are guaranteed to be initialized.
    // - The pointer is valid for reads for the entire range.
    unsafe { slice::from_raw_parts(self.as_ptr(), P::LENGTH.as_usize()) }
  }

  /// Extracts a mutable slice of the entire array.
  #[inline]
  pub(crate) const fn as_mut_slice(&mut self) -> &mut [T] {
    // SAFETY:
    // - The allocation contains `P::LENGTH` contiguous elements of `T`.
    // - For `Array<T, P>`, all elements are guaranteed to be initialized.
    // - `&mut self` guarantees unique access to the allocation.
    unsafe { slice::from_raw_parts_mut(self.as_mut_ptr(), P::LENGTH.as_usize()) }
  }

  /// Returns a reference to the element at `index`.
  #[inline]
  pub(crate) const fn get(&self, index: Concrete<P>) -> &T {
    // SAFETY: `Concrete<P>` ensures that the underlying index is strictly less
    //         than `P::LENGTH`, so it is within bounds.
    unsafe { self.get_unchecked(index.get()) }
  }

  /// Returns a reference to the element at `index`, without doing bounds
  /// checking.
  ///
  /// # Safety
  ///
  /// `index` must be strictly less than `P::LENGTH`. Passing an out-of-bounds
  /// index results in undefined behavior, even if the returned reference is not
  /// used.
  #[inline]
  pub(crate) const unsafe fn get_unchecked(&self, index: usize) -> &T {
    debug_assert!(
      index < P::LENGTH.as_usize(),
      "Array::get_unchecked requires that the index is in bounds",
    );

    // SAFETY:
    // - The caller guarantees `index < P::LENGTH`.
    // - The allocation holds `P::LENGTH` contiguous elements.
    // - The pointer is properly aligned and valid for reads.
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
  /// The caller must guarantee that all elements in the allocation are fully
  /// initialized. If any element is uninitialized, converting to `Array<T, P>`
  /// results in immediate undefined behavior.
  #[inline]
  pub(crate) unsafe fn assume_init(self) -> Array<T, P> {
    // SAFETY:
    // - The caller guarantees that all elements are initialized.
    // - `Array<MaybeUninit<T>, P>` and `Array<T, P>` have identical layout.
    // - `ManuallyDrop` prevents `self` from being dropped, so the allocation is
    //   not freed during the conversion.
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
    // - The allocation was created with `alloc(P::LAYOUT)` in `new_uninit`.
    // - `P::LAYOUT` is the exact layout used for allocation.
    // - `self.nonnull` still points to the original allocation.
    unsafe {
      dealloc(self.as_non_null().cast().as_ptr(), P::LAYOUT);
    }
  }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(test)]
mod tests {
  use crate::array::Array;
  use crate::index::Concrete;
  use crate::params::CACHE_LINE;
  use crate::params::Params;
  use crate::utils::each_capacity;

  #[cfg_attr(loom, ignore = "loom does not run this test")]
  #[test]
  fn alignment() {
    each_capacity!({
      let array: Array<usize, P> = Array::new(|_, slot| {
        slot.write(0);
      });

      // TODO: ptr::is_aligned_to once stable
      assert_eq!(array.as_ptr().addr() & (CACHE_LINE - 1), 0);
    });
  }

  #[cfg_attr(loom, ignore = "loom does not run this test")]
  #[test]
  fn get() {
    each_capacity!({
      let array: Array<usize, P> = Array::new(|index, slot| {
        slot.write(index);
      });

      for index in 0..P::LENGTH.as_usize() {
        assert_eq!(array.get(Concrete::<P>::new(index)), &index);
      }
    });
  }

  #[cfg_attr(loom, ignore = "loom does not run this test")]
  #[test]
  fn as_slice() {
    each_capacity!({
      let array: Array<usize, P> = Array::new(|index, slot| {
        slot.write(index);
      });

      assert_eq!(array.as_slice().len(), P::LENGTH.as_usize());

      for (index, value) in array.as_slice().iter().enumerate() {
        assert_eq!(*value, index);
      }
    });
  }

  #[cfg_attr(loom, ignore = "loom does not run this test")]
  #[test]
  fn as_mut_slice() {
    each_capacity!({
      let mut array: Array<usize, P> = Array::new(|index, slot| {
        slot.write(index);
      });

      assert_eq!(array.as_mut_slice().len(), P::LENGTH.as_usize());

      for (index, value) in array.as_mut_slice().iter_mut().enumerate() {
        assert_eq!(*value, index);
        *value += 1;
      }

      for (index, value) in array.as_slice().iter().enumerate() {
        assert_eq!(*value, index + 1);
      }
    });
  }
}
