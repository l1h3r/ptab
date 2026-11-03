use core::num::NonZeroUsize;

use crate::params::CACHE_LINE;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::Capacity;
use crate::params::ConstParams;
use crate::params::DefaultParams;
use crate::params::Params;
use crate::params::ParamsExt as _;
use crate::params::derive_blocks;
use crate::params::derive_layout;
use crate::sync::atomic::AtomicUsize;
use crate::utils::NoLength;
use crate::utils::any_capacity;
use crate::utils::any_valid_capacity;
use crate::utils::each_capacity;

// -----------------------------------------------------------------------------
// Params
// -----------------------------------------------------------------------------

#[kani::proof]
fn params_length_defaults_to_def() {
  assert!(NoLength::LENGTH == Capacity::DEF);
}

// -----------------------------------------------------------------------------
// ParamsExt
// -----------------------------------------------------------------------------

#[kani::proof]
fn blocks_matches_derive_blocks() {
  each_capacity!({
    assert!(P::BLOCKS == derive_blocks::<P>());
  });
}

#[kani::proof]
fn blocks_is_power_of_two() {
  each_capacity!({
    assert!(P::BLOCKS.is_power_of_two());
  });
}

#[kani::proof]
fn blocks_times_slots_is_length() {
  each_capacity!({
    assert!(P::BLOCKS.get().checked_mul(CACHE_LINE_SLOTS) == Some(S));
  });
}

#[kani::proof]
fn layout_matches_derive_layout() {
  each_capacity!({
    assert!(P::LAYOUT == derive_layout::<P>());
  });
}

#[kani::proof]
fn layout_size_is_memory() {
  each_capacity!({
    assert!(P::LAYOUT.size() == P::MEMORY);
  });
}

#[kani::proof]
fn layout_align_is_cache_line() {
  each_capacity!({
    assert!(P::LAYOUT.align() == CACHE_LINE);
  });
}

#[kani::proof]
fn memory_is_length_slots() {
  each_capacity!({
    assert!(P::MEMORY == S * size_of::<AtomicUsize>());
  });
}

#[kani::proof]
fn memory_fits_isize() {
  each_capacity!({
    assert!(P::MEMORY <= isize::MAX as usize);
  });
}

#[kani::proof]
fn id_mask_bits_is_sum_of_shifts() {
  each_capacity!({
    assert!(P::ID_MASK_BITS == P::ID_SHIFT_BLOCK + P::ID_SHIFT_INDEX);
  });
}

#[kani::proof]
fn id_mask_entry_is_length_minus_one() {
  each_capacity!({
    assert!(P::ID_MASK_ENTRY == S - 1);
  });
}

#[kani::proof]
fn id_mask_entry_joins_block_and_index() {
  each_capacity!({
    assert!(P::ID_MASK_ENTRY == (P::ID_MASK_BLOCK << P::ID_SHIFT_BLOCK) | P::ID_MASK_INDEX);
  });
}

#[kani::proof]
fn id_mask_block_is_blocks_minus_one() {
  each_capacity!({
    assert!(P::ID_MASK_BLOCK == P::BLOCKS.get() - 1);
  });
}

#[kani::proof]
fn id_mask_index_is_slots_minus_one() {
  each_capacity!({
    assert!(P::ID_MASK_INDEX == CACHE_LINE_SLOTS - 1);
  });
}

#[kani::proof]
fn id_mask_block_and_index_are_disjoint() {
  each_capacity!({
    assert!((P::ID_MASK_BLOCK << P::ID_SHIFT_BLOCK) & P::ID_MASK_INDEX == 0);
  });
}

#[kani::proof]
fn id_shift_block_is_index_width() {
  each_capacity!({
    assert!(P::ID_SHIFT_BLOCK == P::ID_MASK_INDEX.count_ones());
  });
}

#[kani::proof]
fn id_shift_index_is_block_width() {
  each_capacity!({
    assert!(P::ID_SHIFT_INDEX == P::ID_MASK_BLOCK.count_ones());
  });
}

// -----------------------------------------------------------------------------
// DefaultParams
// -----------------------------------------------------------------------------

#[kani::proof]
fn default_params_length() {
  assert!(DefaultParams::LENGTH == Capacity::DEF);
}

// -----------------------------------------------------------------------------
// ConstParams
// -----------------------------------------------------------------------------

#[kani::proof]
fn const_params_length_rounds_up() {
  assert!(ConstParams::<1000>::LENGTH.as_usize() == 1024);
}

#[kani::proof]
fn const_params_length_clamps_below_min() {
  assert!(ConstParams::<0>::LENGTH == Capacity::MIN);
}

#[kani::proof]
fn const_params_length_clamps_above_max() {
  assert!(ConstParams::<{ usize::MAX }>::LENGTH == Capacity::MAX);
}

// -----------------------------------------------------------------------------
// Capacity
// -----------------------------------------------------------------------------

#[kani::proof]
fn capacity_new_clamps_below_min() {
  let value: usize = kani::any_where(|value: &usize| *value < Capacity::MIN.as_usize());

  assert!(Capacity::new(value) == Capacity::MIN);
}

#[kani::proof]
fn capacity_new_clamps_above_max() {
  let value: usize = kani::any_where(|value: &usize| *value > Capacity::MAX.as_usize());

  assert!(Capacity::new(value) == Capacity::MAX);
}

#[kani::proof]
fn capacity_new_clamps_usize_max() {
  let value: usize = kani::any_where(|value: &usize| value.checked_next_power_of_two().is_none());

  assert!(Capacity::new(value) == Capacity::MAX);
}

#[kani::proof]
fn capacity_new_keeps_power_of_two() {
  let capacity: Capacity = any_capacity();

  assert!(Capacity::new(capacity.as_usize()) == capacity);
}

#[kani::proof]
fn capacity_new_rounds_up() {
  let value: usize = kani::any_where(|value: &usize| {
    *value > Capacity::MIN.as_usize() && *value <= Capacity::MAX.as_usize()
  });
  let capacity: usize = Capacity::new(value).as_usize();

  assert!(capacity >= value);
  assert!(capacity / 2 < value);
  kani::cover!(!value.is_power_of_two(), "the input rounds up");
}

#[kani::proof]
fn capacity_new_is_power_of_two() {
  assert!(any_capacity().as_usize().is_power_of_two());
}

#[kani::proof]
fn capacity_new_is_at_least_min() {
  assert!(any_capacity().as_usize() >= Capacity::MIN.as_usize());
}

#[kani::proof]
fn capacity_new_is_at_most_max() {
  assert!(any_capacity().as_usize() <= Capacity::MAX.as_usize());
}

#[kani::proof]
fn capacity_new_unchecked() {
  let value: usize = any_valid_capacity();

  // SAFETY: `value` satisfies `Capacity::is_valid`.
  assert!(unsafe { Capacity::new_unchecked(value) }.as_usize() == value);
}

#[kani::proof]
#[kani::should_panic]
fn capacity_new_unchecked_rejects_invalid() {
  let value: usize = kani::any_where(|value: &usize| !Capacity::is_valid(*value));

  // SAFETY: with debug assertions on, `new_unchecked` panics before the
  // transmute for a value `is_valid` rejects.
  let _ = unsafe { Capacity::new_unchecked(value) };
}

#[kani::proof]
fn capacity_as_u32() {
  let capacity: Capacity = any_capacity();

  assert!(capacity.as_u32() as usize == capacity.as_usize());
}

#[kani::proof]
fn capacity_as_usize() {
  let value: usize = any_valid_capacity();

  assert!(Capacity::new(value).as_usize() == value);
}

#[kani::proof]
fn capacity_as_nonzero() {
  let capacity: Capacity = any_capacity();

  assert!(capacity.as_nonzero().get() == capacity.as_usize());
}

#[kani::proof]
fn capacity_log2() {
  let capacity: Capacity = any_capacity();

  assert!(1_usize << capacity.log2() == capacity.as_usize());
}

#[kani::proof]
fn capacity_default() {
  assert!(Capacity::default() == Capacity::DEF);
}

#[kani::proof]
fn capacity_into_nonzero() {
  let capacity: Capacity = any_capacity();

  assert!(NonZeroUsize::from(capacity).get() == capacity.as_usize());
}

#[kani::proof]
fn capacity_into_usize() {
  let capacity: Capacity = any_capacity();

  assert!(usize::from(capacity) == capacity.as_usize());
}
