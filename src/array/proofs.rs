use core::mem::MaybeUninit;
use core::ptr::NonNull;

use crate::array::Array;
use crate::index::Concrete;
use crate::params::Params;
use crate::utils::MinParams;
use crate::utils::any_index;
use crate::utils::each_capacity;

type Slot = MaybeUninit<usize>;

fn empty<P>() -> Array<Slot, P>
where
  P: Params,
{
  let array: Array<MaybeUninit<Slot>, P> = Array::new_uninit();
  // SAFETY: every `Slot` is a `MaybeUninit`, initialized whatever its bytes.
  unsafe { array.assume_init() }
}

fn written<P>(index: usize, value: usize) -> Array<Slot, P>
where
  P: Params,
{
  let array: Array<Slot, P> = empty();
  // SAFETY: `index` is below `P::LENGTH`, and the allocation holds `P::LENGTH`
  // slots.
  unsafe { array.as_mut_ptr().add(index).write(MaybeUninit::new(value)) };
  array
}

// #[kani::proof]
// #[kani::unwind(1025)]
// fn array_new_calls_init_in_order() {
//   let calls: Cell<usize> = Cell::new(0);
//   let array: Array<usize, MinParams> = Array::new(|index, slot| {
//     assert!(index == calls.get());
//     calls.set(index + 1);
//     slot.write(index);
//   });

//   assert!(calls.get() == MinParams::LENGTH.as_usize());
//   drop(array);
// }

// #[kani::proof]
// #[kani::unwind(1025)]
// fn array_new_writes_every_slot() {
//   let array: Array<usize, MinParams> = Array::new(|index, slot| {
//     slot.write(index);
//   });
//   let index: usize = any_index::<MinParams>();

//   assert!(*array.get(Concrete::new(index)) == index);
// }

#[kani::proof]
fn array_as_ptr_is_as_non_null() {
  let array: Array<Slot, MinParams> = empty();

  assert!(array.as_ptr() == array.as_non_null().as_ptr().cast_const());
}

#[kani::proof]
fn array_as_mut_ptr_is_as_non_null() {
  let array: Array<Slot, MinParams> = empty();

  assert!(array.as_mut_ptr() == array.as_non_null().as_ptr());
}

#[kani::proof]
fn array_as_slice_has_length() {
  each_capacity!({
    let array: Array<Slot, P> = empty();

    assert!(array.as_slice().len() == S);
  });
}

#[kani::proof]
fn array_as_slice_reads_slot() {
  let index: usize = any_index::<MinParams>();
  let value: usize = kani::any();
  let array: Array<Slot, MinParams> = written(index, value);

  // SAFETY: `written` initialized slot `index`.
  assert!(unsafe { array.as_slice()[index].assume_init_read() } == value);
}

#[kani::proof]
fn array_as_mut_slice_has_length() {
  each_capacity!({
    let mut array: Array<Slot, P> = empty();

    assert!(array.as_mut_slice().len() == S);
  });
}

#[kani::proof]
fn array_as_mut_slice_writes_slot() {
  let index: usize = any_index::<MinParams>();
  let value: usize = kani::any();
  let mut array: Array<Slot, MinParams> = empty();

  array.as_mut_slice()[index].write(value);

  // SAFETY: the write above initialized slot `index`.
  assert!(unsafe { array.get(Concrete::new(index)).assume_init_read() } == value);
}

#[kani::proof]
fn array_get_returns_slot() {
  let index: usize = any_index::<MinParams>();
  let value: usize = kani::any();
  let array: Array<Slot, MinParams> = written(index, value);

  // SAFETY: `written` initialized slot `index`.
  assert!(unsafe { array.get(Concrete::new(index)).assume_init_read() } == value);
}

#[kani::proof]
fn array_get_unchecked_returns_slot() {
  let index: usize = any_index::<MinParams>();
  let value: usize = kani::any();
  let array: Array<Slot, MinParams> = written(index, value);

  // SAFETY: `index` is below `MinParams::LENGTH`, and `written` initialized
  // slot `index`.
  assert!(unsafe { array.get_unchecked(index).assume_init_read() } == value);
}

#[kani::proof]
#[kani::should_panic]
fn array_get_unchecked_rejects_out_of_bounds() {
  let array: Array<Slot, MinParams> = empty();
  let index: usize = kani::any_where(|index: &usize| *index >= MinParams::LENGTH.as_usize());

  // SAFETY: with debug assertions on, `get_unchecked` panics before the
  // dereference for an index at or past `LENGTH`.
  let _ = unsafe { array.get_unchecked(index) };
}

#[kani::proof]
fn array_assume_init_keeps_pointer() {
  let uninit: Array<MaybeUninit<Slot>, MinParams> = Array::new_uninit();
  let nonnull: NonNull<MaybeUninit<Slot>> = uninit.as_non_null();
  // SAFETY: every `Slot` is a `MaybeUninit`, initialized whatever its bytes.
  let array: Array<Slot, MinParams> = unsafe { uninit.assume_init() };

  assert!(array.as_non_null() == nonnull.cast());
}
