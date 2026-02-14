use std::collections::HashSet;

use crate::index::Abstract;
use crate::index::Concrete;
use crate::index::Detached;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::Params;
use crate::params::ParamsExt;

#[test]
fn abstract_clone_copy() {
  let a: Abstract<()> = Abstract::new(123);
  let b: Abstract<()> = a.clone();
  let c: Abstract<()> = *&b;

  assert_eq!(a, b);
  assert_eq!(b, c);
  assert_eq!(c, a);
}

#[test]
fn concrete_clone_copy() {
  let a: Concrete<()> = Concrete::new(123);
  let b: Concrete<()> = a.clone();
  let c: Concrete<()> = *&b;

  assert_eq!(a, b);
  assert_eq!(b, c);
  assert_eq!(c, a);
}

#[test]
fn abstract_debug_transparency() {
  let value: usize = 123;
  let index: Abstract<()> = Abstract::new(value);

  assert_eq!(format!("{index:?}"), format!("{value:?}"));
}

#[test]
fn concrete_debug_transparency() {
  let value: usize = 123;
  let index: Concrete<()> = Concrete::new(value);

  assert_eq!(format!("{index:?}"), format!("{value:?}"));
}

#[test]
fn detached_debug_transparency() {
  let value: usize = 123;
  let index: Detached = Detached::from_bits(value);

  assert_eq!(format!("{index:?}"), format!("{value:?}"));
}

#[test]
fn detached_display_transparency() {
  let value: usize = 123;
  let index: Detached = Detached::from_bits(value);

  assert_eq!(format!("{index}"), format!("{value}"));
}

#[cfg_attr(
  not(feature = "slow"),
  ignore = "enable the 'slow' feature to run this test."
)]
#[test]
fn abstract_to_concrete_covers_all_slots() {
  each_capacity!({
    let mut used: HashSet<usize> = HashSet::with_capacity(P::LENGTH.as_usize());

    for index in 0..P::LENGTH.as_usize() {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let concrete_idx: Concrete<P> = Concrete::from_abstract(abstract_idx);

      used.insert(concrete_idx.get());
    }

    assert_eq!(
      used.len(),
      P::LENGTH.as_usize(),
      "invalid id mapping: abstract fails to cover all concrete slots - {:?}",
      P::debug(),
    );
  });
}

#[cfg_attr(
  not(feature = "slow"),
  ignore = "enable the 'slow' feature to run this test."
)]
#[test]
fn abstract_to_detached_roundtrip() {
  each_capacity!({
    for index in 0..P::LENGTH.as_usize() {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let detached_idx: Detached = Detached::from_abstract(abstract_idx);
      let recovery_idx: Abstract<P> = Abstract::from_detached(detached_idx);

      assert_eq!(
        abstract_idx,
        recovery_idx,
        "invalid id mapping: abstract-to-detached conversion not recoverable - {:?}",
        P::debug(),
      );
    }
  });
}

#[cfg_attr(
  not(feature = "slow"),
  ignore = "enable the 'slow' feature to run this test."
)]
#[test]
fn detached_to_concrete_matches_direct_conversion() {
  each_capacity!({
    for index in 0..P::LENGTH.as_usize() {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let detached_idx: Detached = Detached::from_abstract(abstract_idx);
      let concrete_idx: Concrete<P> = Concrete::from_abstract(abstract_idx);
      let recovery_idx: Concrete<P> = Concrete::from_detached(detached_idx);

      assert_eq!(
        concrete_idx,
        recovery_idx,
        "invalid id mapping: detached-to-concrete conversion mismatch - {:?}",
        P::debug(),
      );
    }
  });
}

#[test]
fn cache_line_distribution() {
  // Verify that consecutive base indices are distributed across cache lines
  each_capacity!({
    if P::BLOCKS.get() <= 1 {
      return; // Skip solo blocks
    }

    // First `CACHE_LINE_SLOTS` indices should map to different blocks
    let mut blocks: HashSet<usize> = HashSet::with_capacity(CACHE_LINE_SLOTS);

    for index in 0..CACHE_LINE_SLOTS {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let concrete_idx: Concrete<P> = Concrete::from_abstract(abstract_idx);

      blocks.insert(concrete_idx.get() / CACHE_LINE_SLOTS);
    }

    assert_eq!(
      blocks.len(),
      P::BLOCKS.get(),
      "invalid id mapping: corrupted cache-line distribution - {:?}",
      P::debug(),
    );
  });
}

#[cfg_attr(
  not(feature = "slow"),
  ignore = "enable the 'slow' feature to run this test."
)]
#[test]
fn serial_number_preservation() {
  each_capacity!({
    for generation in 0..16 {
      let serial: usize = generation * P::LENGTH.as_usize();

      for index in 0..P::LENGTH.as_usize() {
        let abstract_idx: Abstract<P> = Abstract::new(serial + index);
        let detached_idx: Detached = Detached::from_abstract(abstract_idx);
        let recovery_idx: Abstract<P> = Abstract::from_detached(detached_idx);

        assert_eq!(
          abstract_idx,
          recovery_idx,
          "invalid id mapping: serial number not preserved - {:?}",
          P::debug(),
        );
      }
    }
  });
}
