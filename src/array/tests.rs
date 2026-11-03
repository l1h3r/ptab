use core::cell::Cell;
use core::mem::MaybeUninit;
use core::ptr::NonNull;

use crate::array::Array;
use crate::index::Concrete;
use crate::params::CACHE_LINE;
use crate::params::Params;
use crate::utils::MinParams;
use crate::utils::each_capacity;

type Slot = MaybeUninit<usize>;

fn indexed<P>() -> Array<usize, P>
where
  P: Params,
{
  Array::new(|index, slot| {
    slot.write(index);
  })
}

#[test]
fn array_new_calls_init_in_order() {
  each_capacity!({
    let calls: Cell<usize> = Cell::new(0);
    let array: Array<usize, P> = Array::new(|index, slot| {
      assert_eq!(index, calls.get());
      calls.set(index + 1);
      slot.write(index);
    });

    assert_eq!(calls.get(), S);
    drop(array);
  });
}

#[test]
fn array_new_writes_every_slot() {
  each_capacity!({
    let array: Array<usize, P> = indexed();

    for index in 0..S {
      assert_eq!(array.get(Concrete::new(index)), &index);
    }
  });
}

#[test]
fn array_new_uninit_is_cache_line_aligned() {
  each_capacity!({
    let array: Array<Slot, P> = Array::new_uninit();

    assert!(array.as_non_null().addr().get().is_multiple_of(CACHE_LINE));
  });
}

#[test]
fn array_as_ptr_is_as_non_null() {
  let array: Array<usize, MinParams> = indexed();

  assert_eq!(array.as_ptr(), array.as_non_null().as_ptr().cast_const());
}

#[test]
fn array_as_mut_ptr_is_as_non_null() {
  let array: Array<usize, MinParams> = indexed();

  assert_eq!(array.as_mut_ptr(), array.as_non_null().as_ptr());
}

#[test]
fn array_as_slice_has_length() {
  each_capacity!({
    let array: Array<usize, P> = indexed();

    assert_eq!(array.as_slice().len(), S);
  });
}

#[test]
fn array_as_slice_reads_slot() {
  each_capacity!({
    let array: Array<usize, P> = indexed();

    for (index, value) in array.as_slice().iter().enumerate() {
      assert_eq!(*value, index);
    }
  });
}

#[test]
fn array_as_mut_slice_has_length() {
  each_capacity!({
    let mut array: Array<usize, P> = indexed();

    assert_eq!(array.as_mut_slice().len(), S);
  });
}

#[test]
fn array_as_mut_slice_writes_slot() {
  each_capacity!({
    let mut array: Array<usize, P> = indexed();

    for value in array.as_mut_slice() {
      *value += 1;
    }

    for (index, value) in array.as_slice().iter().enumerate() {
      assert_eq!(*value, index + 1);
    }
  });
}

#[test]
fn array_get_returns_slot() {
  each_capacity!({
    let array: Array<usize, P> = indexed();

    for index in 0..S {
      assert_eq!(array.get(Concrete::new(index)), &index);
    }
  });
}

#[test]
fn array_get_unchecked_returns_slot() {
  each_capacity!({
    let array: Array<usize, P> = indexed();

    for index in 0..S {
      // SAFETY: `index` is below `S`, the array's `LENGTH`.
      assert_eq!(unsafe { array.get_unchecked(index) }, &index);
    }
  });
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "Array::get_unchecked requires that the index is in bounds")]
fn array_get_unchecked_rejects_out_of_bounds() {
  let array: Array<usize, MinParams> = indexed();

  // SAFETY: with debug assertions on, `get_unchecked` panics before the
  // dereference for an index at or past `LENGTH`.
  let _ = unsafe { array.get_unchecked(MinParams::LENGTH.as_usize()) };
}

#[test]
fn array_assume_init_keeps_pointer() {
  let uninit: Array<MaybeUninit<Slot>, MinParams> = Array::new_uninit();
  let nonnull: NonNull<MaybeUninit<Slot>> = uninit.as_non_null();
  // SAFETY: every `Slot` is a `MaybeUninit`, initialized whatever its bytes.
  let array: Array<Slot, MinParams> = unsafe { uninit.assume_init() };

  assert_eq!(array.as_non_null(), nonnull.cast());
}
