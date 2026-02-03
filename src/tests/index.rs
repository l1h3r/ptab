use std::collections::HashSet;

use crate::index::Abstract;
use crate::index::Concrete;
use crate::index::Detached;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::Params;
use crate::params::ParamsExt;

#[cfg_attr(not(feature = "slow"), ignore = "enable the 'slow' feature to run this test.")]
#[test]
fn test_abstract_to_concrete_covers_all_slots() {
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
      "invalid id mapping: abstract fails to cover all concrete slots",
    );
  });
}

#[cfg_attr(not(feature = "slow"), ignore = "enable the 'slow' feature to run this test.")]
#[test]
fn test_abstract_to_detached_roundtrip() {
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

#[cfg_attr(not(feature = "slow"), ignore = "enable the 'slow' feature to run this test.")]
#[test]
fn test_detached_to_concrete_matches_direct_conversion() {
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
fn test_cache_line_distribution() {
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

#[cfg_attr(not(feature = "slow"), ignore = "enable the 'slow' feature to run this test.")]
#[test]
fn test_serial_number_preservation() {
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
