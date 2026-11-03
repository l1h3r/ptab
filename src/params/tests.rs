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
use crate::utils::each_capacity;
use crate::utils::hash;
use crate::utils::std::String;
use crate::utils::std::format;

const DEBUG_FIELDS: [&str; 10] = [
  "LENGTH",
  "BLOCKS",
  "LAYOUT",
  "MEMORY",
  "ID_MASK_BITS",
  "ID_MASK_ENTRY",
  "ID_MASK_BLOCK",
  "ID_MASK_INDEX",
  "ID_SHIFT_BLOCK",
  "ID_SHIFT_INDEX",
];

// -----------------------------------------------------------------------------
// Params
// -----------------------------------------------------------------------------

#[test]
fn params_length_defaults_to_def() {
  assert_eq!(NoLength::LENGTH, Capacity::DEF);
}

// -----------------------------------------------------------------------------
// ParamsExt
// -----------------------------------------------------------------------------

#[test]
fn blocks_matches_derive_blocks() {
  each_capacity!({
    assert_eq!(P::BLOCKS, derive_blocks::<P>());
  });
}

#[test]
fn blocks_is_power_of_two() {
  each_capacity!({
    assert!(P::BLOCKS.is_power_of_two());
  });
}

#[test]
fn blocks_times_slots_is_length() {
  each_capacity!({
    assert_eq!(P::BLOCKS.get() * CACHE_LINE_SLOTS, S);
  });
}

#[test]
fn layout_matches_derive_layout() {
  each_capacity!({
    assert_eq!(P::LAYOUT, derive_layout::<P>());
  });
}

#[test]
fn layout_size_is_memory() {
  each_capacity!({
    assert_eq!(P::LAYOUT.size(), P::MEMORY);
  });
}

#[test]
fn layout_align_is_cache_line() {
  each_capacity!({
    assert_eq!(P::LAYOUT.align(), CACHE_LINE);
  });
}

#[test]
fn memory_is_length_slots() {
  each_capacity!({
    assert_eq!(P::MEMORY, S * size_of::<AtomicUsize>());
  });
}

#[test]
fn memory_fits_isize() {
  each_capacity!({
    assert!(isize::try_from(P::MEMORY).is_ok());
  });
}

#[test]
fn id_mask_bits_is_sum_of_shifts() {
  each_capacity!({
    assert_eq!(P::ID_MASK_BITS, P::ID_SHIFT_BLOCK + P::ID_SHIFT_INDEX);
  });
}

#[test]
fn id_mask_entry_is_length_minus_one() {
  each_capacity!({
    assert_eq!(P::ID_MASK_ENTRY, S - 1);
  });
}

#[test]
fn id_mask_entry_joins_block_and_index() {
  each_capacity!({
    assert_eq!(
      P::ID_MASK_ENTRY,
      (P::ID_MASK_BLOCK << P::ID_SHIFT_BLOCK) | P::ID_MASK_INDEX,
    );
  });
}

#[test]
fn id_mask_block_is_blocks_minus_one() {
  each_capacity!({
    assert_eq!(P::ID_MASK_BLOCK, P::BLOCKS.get() - 1);
  });
}

#[test]
fn id_mask_index_is_slots_minus_one() {
  each_capacity!({
    assert_eq!(P::ID_MASK_INDEX, CACHE_LINE_SLOTS - 1);
  });
}

#[test]
fn id_mask_block_and_index_are_disjoint() {
  each_capacity!({
    assert_eq!(
      (P::ID_MASK_BLOCK << P::ID_SHIFT_BLOCK) & P::ID_MASK_INDEX,
      0
    );
  });
}

#[test]
fn id_shift_block_is_index_width() {
  each_capacity!({
    assert_eq!(P::ID_SHIFT_BLOCK, P::ID_MASK_INDEX.count_ones());
  });
}

#[test]
fn id_shift_index_is_block_width() {
  each_capacity!({
    assert_eq!(P::ID_SHIFT_INDEX, P::ID_MASK_BLOCK.count_ones());
  });
}

#[test]
fn params_ext_debug_lists_every_field() {
  let params: String = format!("{:?}", DefaultParams::debug());

  assert!(params.contains("DefaultParams"));

  for field in DEBUG_FIELDS {
    assert!(params.contains(&format!("{field}: ")));
  }
}

// -----------------------------------------------------------------------------
// DebugParams
// -----------------------------------------------------------------------------

#[expect(clippy::clone_on_copy, reason = "exercises the `Clone` impl")]
#[test]
fn debug_params_clone_copy() {
  let a = DefaultParams::debug();
  let b = a.clone();
  let c = b;

  assert_eq!(format!("{a:?}"), format!("{b:?}"));
  assert_eq!(format!("{b:?}"), format!("{c:?}"));
}

// -----------------------------------------------------------------------------
// DefaultParams
// -----------------------------------------------------------------------------

#[test]
fn default_params_length() {
  assert_eq!(DefaultParams::LENGTH, Capacity::DEF);
}

#[expect(clippy::clone_on_copy, reason = "exercises the `Clone` impl")]
#[test]
fn default_params_clone_copy() {
  let a: DefaultParams = DefaultParams;
  let b: DefaultParams = a.clone();
  let _: DefaultParams = b;
}

// -----------------------------------------------------------------------------
// ConstParams
// -----------------------------------------------------------------------------

#[test]
fn const_params_length_rounds_up() {
  assert_eq!(ConstParams::<1000>::LENGTH.as_usize(), 1024);
}

#[test]
fn const_params_length_clamps_below_min() {
  assert_eq!(ConstParams::<0>::LENGTH, Capacity::MIN);
}

#[test]
fn const_params_length_clamps_above_max() {
  assert_eq!(ConstParams::<{ usize::MAX }>::LENGTH, Capacity::MAX);
}

#[expect(clippy::clone_on_copy, reason = "exercises the `Clone` impl")]
#[test]
fn const_params_clone_copy() {
  let a: ConstParams<1024> = ConstParams;
  let b: ConstParams<1024> = a.clone();
  let c: ConstParams<1024> = b;

  assert_eq!(a, b);
  assert_eq!(b, c);
  assert_eq!(c, a);
}

#[test]
fn const_params_debug() {
  assert_eq!(format!("{:?}", ConstParams::<1024>), "ConstParams");
}

#[test]
fn const_params_hash() {
  assert_eq!(hash(ConstParams::<1024>), hash(ConstParams::<1024>));
}

#[test]
fn const_params_eq() {
  assert_eq!(ConstParams::<1024>, ConstParams::<1024>);
}

// -----------------------------------------------------------------------------
// Capacity
// -----------------------------------------------------------------------------

#[test]
fn capacity_new_clamps_below_min() {
  assert_eq!(Capacity::new(1), Capacity::MIN);
}

#[test]
fn capacity_new_clamps_above_max() {
  assert_eq!(Capacity::new(Capacity::MAX.as_usize() + 1), Capacity::MAX);
}

#[test]
fn capacity_new_clamps_usize_max() {
  assert_eq!(Capacity::new(usize::MAX), Capacity::MAX);
}

#[test]
fn capacity_new_keeps_power_of_two() {
  each_capacity!({
    assert_eq!(Capacity::new(S).as_usize(), S);
  });
}

#[test]
fn capacity_new_rounds_up() {
  assert_eq!(Capacity::new((1 << 10) + 1).as_usize(), 1 << 11);
}

#[test]
fn capacity_new_unchecked() {
  each_capacity!({
    // SAFETY: `S` is a power of two in `MIN..=MAX`.
    assert_eq!(unsafe { Capacity::new_unchecked(S) }, P::LENGTH);
  });
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "Capacity::new_unchecked requires a valid value")]
fn capacity_new_unchecked_rejects_invalid() {
  // SAFETY: with debug assertions on, `new_unchecked` panics before the
  // transmute for a value that is not a power of two.
  let _ = unsafe { Capacity::new_unchecked(3) };
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
  each_capacity!({
    assert_eq!(
      format!("{:?}", P::LENGTH),
      format!("{S} (1 << {})", S.ilog2())
    );
  });
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

#[expect(clippy::clone_on_copy, reason = "exercises the `Clone` impl")]
#[test]
fn capacity_clone_copy() {
  let a: Capacity = Capacity::MIN;
  let b: Capacity = a.clone();
  let c: Capacity = b;

  assert_eq!(a, b);
  assert_eq!(b, c);
  assert_eq!(c, a);
}

#[test]
fn capacity_eq() {
  assert_eq!(Capacity::MIN, Capacity::MIN);
  assert_ne!(Capacity::MIN, Capacity::MAX);
}
