use crate::index::Abstract;
use crate::index::Concrete;
use crate::index::Detached;
use crate::index::RESERVED;
use crate::index::abstract_next_serial;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::ParamsExt as _;
use crate::utils::any_index;
use crate::utils::each_capacity;

// -----------------------------------------------------------------------------
// Abstract
// -----------------------------------------------------------------------------

#[kani::proof]
fn abstract_get() {
  let value: usize = kani::any();

  assert!(Abstract::<()>::new(value).get() == value);
}

#[kani::proof]
fn abstract_clone_copy() {
  let a: Abstract<()> = Abstract::new(kani::any());
  let b: Abstract<()> = a.clone();
  let c: Abstract<()> = b;

  assert!(a == b);
  assert!(b == c);
  assert!(c == a);
}

#[kani::proof]
fn abstract_eq() {
  let a: usize = kani::any();
  let b: usize = kani::any();

  assert!((Abstract::<()>::new(a) == Abstract::new(b)) == (a == b));
}

#[kani::proof]
fn abstract_round_trips_through_detached() {
  each_capacity!({
    let index: Abstract<P> = Abstract::new(kani::any());

    assert!(Abstract::from_detached(Detached::from_abstract(index)) == index);
  });
}

// -----------------------------------------------------------------------------
// Concrete
// -----------------------------------------------------------------------------

#[kani::proof]
fn concrete_get() {
  let value: usize = kani::any();

  assert!(Concrete::<()>::new(value).get() == value);
}

#[kani::proof]
fn concrete_clone_copy() {
  let a: Concrete<()> = Concrete::new(kani::any());
  let b: Concrete<()> = a.clone();
  let c: Concrete<()> = b;

  assert!(a == b);
  assert!(b == c);
  assert!(c == a);
}

#[kani::proof]
fn concrete_eq() {
  let a: usize = kani::any();
  let b: usize = kani::any();

  assert!((Concrete::<()>::new(a) == Concrete::new(b)) == (a == b));
}

#[kani::proof]
fn concrete_from_abstract_is_in_bounds() {
  each_capacity!({
    let index: Abstract<P> = Abstract::new(kani::any());

    assert!(Concrete::from_abstract(index).get() < S);
  });
}

#[kani::proof]
fn concrete_from_abstract_is_injective() {
  each_capacity!({
    let a: Abstract<P> = Abstract::new(any_index::<P>());
    let b: Abstract<P> = Abstract::new(any_index::<P>());

    if Concrete::from_abstract(a) == Concrete::from_abstract(b) {
      assert!(a == b);
    }
  });
}

#[kani::proof]
fn concrete_from_abstract_is_surjective() {
  each_capacity!({
    let slot: usize = any_index::<P>();
    let index: Abstract<P> = Abstract::from_detached(Detached::from_bits(slot));

    assert!(index.get() < S);
    assert!(Concrete::from_abstract(index).get() == slot);
  });
}

#[kani::proof]
fn concrete_from_abstract_ignores_serial() {
  each_capacity!({
    let value: usize = kani::any();

    assert!(
      Concrete::from_abstract(Abstract::<P>::new(value))
        == Concrete::from_abstract(Abstract::new(value & P::ID_MASK_ENTRY))
    );
  });
}

#[kani::proof]
fn concrete_from_abstract_block_is_index_mod_blocks() {
  each_capacity!({
    let value: usize = kani::any();
    let slot: usize = Concrete::from_abstract(Abstract::<P>::new(value)).get();

    assert!(slot / CACHE_LINE_SLOTS == value % P::BLOCKS.get());
  });
}

#[kani::proof]
fn concrete_from_abstract_separates_neighbours() {
  each_capacity!({
    let value: usize = kani::any();
    let this: usize = Concrete::from_abstract(Abstract::<P>::new(value)).get();
    let next: usize = Concrete::from_abstract(Abstract::<P>::new(value.wrapping_add(1))).get();

    assert!(this / CACHE_LINE_SLOTS != next / CACHE_LINE_SLOTS);
  });
}

#[kani::proof]
fn concrete_from_detached_is_in_bounds() {
  each_capacity!({
    let index: Detached = Detached::from_bits(kani::any());

    assert!(Concrete::<P>::from_detached(index).get() < S);
  });
}

#[kani::proof]
fn concrete_from_detached_matches_from_abstract() {
  each_capacity!({
    let index: Detached = Detached::from_bits(kani::any());

    assert!(
      Concrete::<P>::from_detached(index)
        == Concrete::from_abstract(Abstract::from_detached(index))
    );
  });
}

// -----------------------------------------------------------------------------
// Detached
// -----------------------------------------------------------------------------

#[kani::proof]
fn detached_into_bits() {
  let bits: usize = kani::any();

  assert!(Detached::from_bits(bits).into_bits() == bits);
}

#[kani::proof]
fn detached_decompose_number() {
  each_capacity!({
    let value: usize = kani::any();
    let index: Detached = Detached::from_abstract(Abstract::<P>::new(value));

    assert!(index.decompose::<P>().0 as usize == value & P::ID_MASK_ENTRY);
  });
}

#[kani::proof]
fn detached_decompose_serial() {
  each_capacity!({
    let value: usize = kani::any();
    let index: Detached = Detached::from_abstract(Abstract::<P>::new(value));

    assert!(index.decompose::<P>().1 == (value >> P::ID_MASK_BITS) as u32);
  });
}

#[kani::proof]
fn detached_round_trips_through_abstract() {
  each_capacity!({
    let index: Detached = Detached::from_bits(kani::any());

    assert!(Detached::from_abstract(Abstract::<P>::from_detached(index)) == index);
  });
}

// -----------------------------------------------------------------------------
// Next Serial
// -----------------------------------------------------------------------------

#[kani::proof]
fn reserved_is_all_ones_in_width() {
  assert!(RESERVED == (1 << (usize::BITS - 4)) - 1);
}

#[kani::proof]
fn abstract_next_serial_is_below_reserved() {
  each_capacity!({
    let value: usize = kani::any_where(|value: &usize| *value < RESERVED);

    assert!(abstract_next_serial(Abstract::<P>::new(value)).get() < RESERVED);
  });
}

#[kani::proof]
fn abstract_next_serial_keeps_slot() {
  each_capacity!({
    let index: Abstract<P> = Abstract::new(kani::any_where(|value: &usize| *value < RESERVED));

    assert!(Concrete::from_abstract(abstract_next_serial(index)) == Concrete::from_abstract(index));
  });
}

#[kani::proof]
fn abstract_next_serial_adds_length() {
  each_capacity!({
    let value: usize = kani::any_where(|value: &usize| *value < RESERVED - S);

    assert!(abstract_next_serial(Abstract::<P>::new(value)).get() == value + S);
  });
}

#[kani::proof]
fn abstract_next_serial_skips_reserved() {
  each_capacity!({
    let index: Abstract<P> = Abstract::new(RESERVED - S);

    assert!(abstract_next_serial(index).get() == S - 1);
  });
}

#[kani::proof]
fn abstract_next_serial_wraps_at_width() {
  each_capacity!({
    let value: usize = kani::any_where(|value: &usize| *value > RESERVED - S && *value < RESERVED);

    assert!(abstract_next_serial(Abstract::<P>::new(value)).get() == value - (RESERVED + 1 - S));
  });
}

#[kani::proof]
fn abstract_next_serial_is_injective() {
  each_capacity!({
    let a: usize = kani::any_where(|value: &usize| *value < RESERVED);
    let b: usize = kani::any_where(|value: &usize| *value < RESERVED);

    if abstract_next_serial(Abstract::<P>::new(a)) == abstract_next_serial(Abstract::new(b)) {
      assert!(a == b);
    }
  });
}
