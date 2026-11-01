use core::num::NonZeroUsize;

use crate::params::CACHE_LINE_SLOTS;
use crate::params::Capacity;
use crate::params::DefaultParams;
use crate::params::Params as _;
use crate::params::ParamsExt as _;
use crate::params::derive_blocks;
use crate::params::derive_layout;
use crate::utils::each_capacity;

#[test]
fn capacity_new_clamp_under_min() {
  assert_eq!(Capacity::new(1), Capacity::MIN);
}

#[test]
fn capacity_new_clamp_over_max() {
  assert_eq!(Capacity::new(Capacity::MAX.as_usize() + 1), Capacity::MAX);
}

#[test]
fn capacity_new_clamp_usize_max() {
  assert_eq!(Capacity::new(usize::MAX), Capacity::MAX);
}

#[test]
fn capacity_new_exact() {
  assert_eq!(Capacity::new(1 << 10).as_usize(), 1 << 10);
}

#[test]
fn capacity_new_round_up() {
  assert_eq!(Capacity::new((1 << 10) - 1).as_usize(), 1 << 10);
}

#[test]
fn capacity_as_u32() {
  each_capacity!({
    assert_eq!(P::LENGTH.as_u32(), S as u32);
  });
}

#[test]
fn capacity_as_usize() {
  each_capacity!({
    assert_eq!(P::LENGTH.as_usize(), S);
  });
}

#[test]
fn capacity_as_nonzero() {
  each_capacity!({
    assert_eq!(P::LENGTH.as_nonzero(), NonZeroUsize::new(S).unwrap());
  });
}

#[test]
fn capacity_log2() {
  each_capacity!({
    assert_eq!(P::LENGTH.log2(), S.ilog2());
  });
}

#[test]
fn capacity_debug() {
  assert_eq!(format!("{:?}", Capacity::MIN), "16 (1 << 4)");
}

#[test]
fn capacity_default() {
  assert_eq!(Capacity::default(), Capacity::DEF);
}

#[test]
fn capacity_into_nonzero() {
  each_capacity!({
    assert_eq!(NonZeroUsize::from(P::LENGTH), NonZeroUsize::new(S).unwrap());
  });
}

#[test]
fn capacity_into_usize() {
  each_capacity!({
    assert_eq!(usize::from(P::LENGTH), S);
  });
}

#[test]
fn default_params() {
  assert_eq!(DefaultParams::LENGTH, Capacity::DEF);
}

#[test]
fn debug_params() {
  let params: String = format!("{:?}", DefaultParams::debug());

  assert!(params.contains("DefaultParams"));
  assert!(params.contains("LENGTH:"));
  assert!(params.contains("BLOCKS:"));
  assert!(params.contains("LAYOUT:"));
  assert!(params.contains("MEMORY:"));
}

#[test]
fn params_length_vs_memory() {
  each_capacity!({
    let expected_size: usize = P::LENGTH.as_usize() * size_of::<usize>();
    let received_size: usize = P::MEMORY;

    assert!(received_size >= expected_size);
  });
}

#[test]
fn params_length_vs_blocks() {
  each_capacity!({
    let expected: usize = P::LENGTH.as_usize();
    let received: usize = P::BLOCKS.get() * CACHE_LINE_SLOTS;

    assert_eq!(expected, received);
  });
}

#[test]
fn params_derive_blocks_at_runtime() {
  each_capacity!({
    assert_eq!(P::BLOCKS, derive_blocks::<P>());
  });
}

#[test]
fn params_derive_layout_at_runtime() {
  each_capacity!({
    assert_eq!(P::LAYOUT, derive_layout::<P>());
  });
}

#[test]
fn params_blocks_power_of_two() {
  each_capacity!({
    assert!(P::BLOCKS.is_power_of_two());
  });
}

#[test]
fn id_mask_bits_composition() {
  each_capacity!({
    assert_eq!(P::ID_MASK_BITS, P::ID_SHIFT_BLOCK + P::ID_SHIFT_INDEX);
  });
}

#[test]
fn id_mask_entry_composition() {
  each_capacity!({
    assert_eq!(
      P::ID_MASK_ENTRY,
      (P::ID_MASK_BLOCK << P::ID_SHIFT_BLOCK) ^ P::ID_MASK_INDEX,
    );
  });
}

#[test]
fn id_mask_entry_covers_all_indices() {
  each_capacity!({
    for index in 0..P::LENGTH.as_usize() {
      assert!(index <= P::ID_MASK_ENTRY);
    }
  });
}

#[test]
fn id_shift_block_composition() {
  each_capacity!({
    assert_eq!(P::ID_SHIFT_BLOCK, P::ID_MASK_INDEX.count_ones());
  });
}

#[test]
fn id_shift_index_composition() {
  each_capacity!({
    assert_eq!(P::ID_SHIFT_INDEX, P::ID_MASK_BLOCK.count_ones());
  });
}
