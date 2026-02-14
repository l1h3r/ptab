use crate::params;
use crate::params::CACHE_LINE;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::Capacity;
use crate::params::DefaultParams;
use crate::params::Params;
use crate::params::ParamsExt;

#[test]
fn capacity_min() {
  assert_eq!(Capacity::new(1).as_usize(), Capacity::MIN.as_usize());
}

#[test]
fn capacity_max() {
  const MAX: usize = Capacity::MAX.as_usize();

  assert_eq!(Capacity::new(MAX + 1).as_usize(), MAX);
  assert_eq!(Capacity::new(usize::MAX).as_usize(), MAX);
}

#[test]
fn capacity_round_up() {
  assert_eq!(Capacity::new((1 << 7) - 25).as_usize(), 1 << 7);
}

#[test]
fn capacity_exact() {
  assert_eq!(Capacity::new(1 << 8).as_usize(), 1 << 8);
}

#[test]
fn capacity_default() {
  assert_eq!(Capacity::DEF, Default::default());
}

#[test]
fn capacity_as_nonzero() {
  each_capacity!({
    assert_eq!(P::LENGTH.as_nonzero().get(), P::LENGTH.as_usize());
  });
}

#[test]
fn capacity_log2() {
  each_capacity!({
    assert_eq!(P::LENGTH.log2(), P::LENGTH.as_usize().ilog2());
  });
}

#[test]
fn capacity_into_nonzero() {
  each_capacity!({
    assert_eq!(P::LENGTH.as_nonzero(), P::LENGTH.into());
  });
}

#[test]
fn capacity_into_usize() {
  each_capacity!({
    assert_eq!(P::LENGTH.as_usize(), P::LENGTH.into());
  });
}

#[test]
fn blocks() {
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
fn blocks_vs_length() {
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
fn masks_and_shifts() {
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
fn entry_mask_covers_all_indices() {
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

#[test]
fn derive_blocks() {
  each_capacity!({
    assert_eq!(P::BLOCKS, params::derive_blocks::<P>());
  })
}

#[test]
fn derive_layout() {
  each_capacity!({
    assert_eq!(P::LAYOUT, params::derive_layout::<P>());
  })
}
