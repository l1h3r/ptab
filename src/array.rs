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
      // SAFETY: `index < P::LENGTH` and the allocation holds `P::LENGTH`
      // elements, so this offset is within bounds.
      let ptr: NonNull<MaybeUninit<T>> = unsafe { this.nonnull.add(index) };

      // SAFETY: The pointer is valid (within our allocation), properly aligned
      // (allocation uses `P::LAYOUT`), and we have exclusive access.
      let uninit: &mut MaybeUninit<T> = unsafe { &mut *ptr.as_ptr() };

      init(index, uninit);

      index += 1;
    }

    // SAFETY: The loop above initialized all `P::LENGTH` elements.
    unsafe { this.assume_init() }
  }

  /// Creates a new array with all bytes zeroed.
  #[inline]
  pub(crate) fn new_zeroed() -> Array<MaybeUninit<T>, P> {
    let this: Array<MaybeUninit<T>, P> = Self::new_uninit();

    // SAFETY: The allocation is valid for writes of `P::LENGTH` elements.
    // Writing zeros to `MaybeUninit<T>` is always safe.
    unsafe {
      this.nonnull.write_bytes(0, P::LENGTH.as_usize());
    }

    this
  }

  /// Creates a new array without initializing its contents.
  #[inline]
  pub(crate) fn new_uninit() -> Array<MaybeUninit<T>, P> {
    P::validate();

    // SAFETY: `P::validate()` asserts that `P::LAYOUT` has non-zero size.
    let raw: *mut u8 = unsafe { alloc(P::LAYOUT) };

    Array {
      nonnull: match NonNull::new(raw.cast()) {
        Some(ptr) => ptr,
        None => handle_alloc_error(P::LAYOUT),
      },
      phantom: PhantomData,
    }
  }

  /// Returns a raw pointer to the array.
  #[inline]
  pub(crate) const fn as_ptr(&self) -> *const T {
    self.nonnull.as_ptr()
  }

  #[inline]
  pub(crate) fn as_slice(&self) -> &[T] {
    // SAFETY: `self.nonnull` returns a pointer to the start of a contiguous
    // allocation of `P::LENGTH` elements, all of which are valid `T` values
    unsafe { slice::from_raw_parts(self.as_ptr(), P::LENGTH.as_usize()) }
  }

  /// Returns a reference to the element at the given index.
  #[inline]
  pub(crate) fn get(&self, index: Concrete<P>) -> &T {
    // SAFETY: `Concrete<P>` values are constructed from `P` parameters and
    // are guaranteed to be less than `P::LENGTH`.
    unsafe { self.get_unchecked(index.get()) }
  }

  /// Returns a reference to the element at `index` without bounds checking.
  ///
  /// # Safety
  ///
  /// `index` must be less than `P::LENGTH`.
  #[inline]
  pub(crate) const unsafe fn get_unchecked(&self, index: usize) -> &T {
    debug_assert!(
      index < P::LENGTH.as_usize(),
      "Array::get_unchecked requires that the index is in bounds",
    );

    // SAFETY: Caller guarantees `index < P::LENGTH`. The allocation holds
    // `P::LENGTH` elements, so the offset is within bounds.
    unsafe { self.nonnull.add(index).as_ref() }
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
  /// All elements must be initialized.
  #[inline]
  pub(crate) unsafe fn assume_init(self) -> Array<T, P> {
    Array {
      // Prevent drop from running on `self` (which would deallocate).
      nonnull: ManuallyDrop::new(self).nonnull.cast(),
      phantom: PhantomData,
    }
  }
}

impl<T, P> Drop for Array<T, P>
where
  P: Params + ?Sized,
{
  fn drop(&mut self) {
    // SAFETY: The pointer was allocated with `P::LAYOUT` in `new_uninit`.
    // Drop provides exclusive access via `&mut self`.
    unsafe {
      dealloc(self.nonnull.cast().as_ptr(), P::LAYOUT);
    }
  }
}
