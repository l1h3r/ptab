use crate::array::Array;
use crate::params::CACHE_LINE;

#[test]
fn test_alignment() {
  each_capacity!({
    // SAFETY: `0` is a valid bit pattern for `usize`.
    let array: Array<usize, P> = unsafe { Array::new_zeroed().assume_init() };

    // TODO: ptr::is_aligned_to once stable
    assert_eq!(
      array.as_ptr().addr() & (CACHE_LINE - 1),
      0,
      "invalid array: pointer not properly aligned",
    );
  });
}
