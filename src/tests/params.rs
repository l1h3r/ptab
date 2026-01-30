use crate::params::CACHE_LINE;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::Capacity;
use crate::params::Params;
use crate::params::ParamsExt;

#[test]
fn test_capacity_min() {
  assert_eq!(
    Capacity::new(1).as_usize(),
    Capacity::MIN.as_usize(),
    "invalid capacity: expected clamp to MIN",
  );
}

#[test]
fn test_capacity_max() {
  assert_eq!(
    Capacity::new(1 << 30).as_usize(),
    Capacity::MAX.as_usize(),
    "invalid capacity: expected clamp to MAX",
  );
}

#[test]
fn test_capacity_round_up() {
  assert_eq!(
    Capacity::new((1 << 7) - 25).as_usize(),
    1 << 7,
    "invalid capacity: expected round up",
  );
}

#[test]
fn test_capacity_exact() {
  assert_eq!(
    Capacity::new(1 << 8).as_usize(),
    1 << 8,
    "invalid capacity: expected no change",
  );
}

#[test]
fn test_blocks() {
  each_capacity!({
    let expected: usize = P::LENGTH.as_usize() * size_of::<usize>();
    let received: usize = P::BLOCKS.get() * CACHE_LINE;

    assert!(
      received >= expected,
      "invalid params: not enough block capacity - {:?}",
      P::debug(),
    );
  });
}

#[test]
fn test_blocks_vs_length() {
  each_capacity!({
    let expected: usize = P::BLOCKS.get() * CACHE_LINE_SLOTS;
    let received: usize = P::LENGTH.as_usize();

    assert_eq!(
      expected,
      received,
      "invalid params: corrupted block calculation - {:?}",
      P::debug(),
    );
  });
}

#[test]
fn test_masks_and_shifts() {
  each_capacity!({
    assert!(
      P::LENGTH.as_usize().is_power_of_two(),
      "invalid params: `LENGTH` must be a power of two - {:?}",
      P::debug(),
    );

    assert!(
      P::BLOCKS.is_power_of_two(),
      "invalid params: `BLOCKS` must be a power of two - {:?}",
      P::debug(),
    );

    assert_eq!(
      P::ID_SHIFT_BLOCK,
      P::ID_MASK_INDEX.count_ones(),
      "invalid params: corrupted bit shift (ID_SHIFT_BLOCK) - {:?}",
      P::debug(),
    );

    assert_eq!(
      P::ID_SHIFT_INDEX,
      P::ID_MASK_BLOCK.count_ones(),
      "invalid params: corrupted bit shift (ID_SHIFT_INDEX) - {:?}",
      P::debug(),
    );

    assert_eq!(
      P::ID_MASK_BITS,
      P::ID_SHIFT_BLOCK + P::ID_SHIFT_INDEX,
      "invalid params: corrupted bit width relationship - {:?}",
      P::debug(),
    );

    assert_eq!(
      P::ID_MASK_ENTRY,
      (P::ID_MASK_BLOCK << P::ID_SHIFT_BLOCK) ^ P::ID_MASK_INDEX,
      "invalid params: corrupted mask composition - {:?}",
      P::debug(),
    );
  });
}

#[test]
fn test_entry_mask_covers_all_indices() {
  each_capacity!({
    for index in 0..P::LENGTH.as_usize() {
      assert!(
        index <= P::ID_MASK_ENTRY,
        "invalid params: index[{}] escapes entry mask - {:?}",
        index,
        P::debug(),
      );
    }
  });
}
