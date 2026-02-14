use crate::array::Array;
use crate::params::CACHE_LINE;
use crate::params::ConstParams;

#[test]
fn alignment() {
  each_capacity!({
    // SAFETY: `0` is a valid bit pattern for `usize`.
    let array: Array<usize, P> = unsafe { Array::new_zeroed().assume_init() };

    // TODO: ptr::is_aligned_to once stable
    assert_eq!(array.as_ptr().addr() & (CACHE_LINE - 1), 0);
  });
}

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
