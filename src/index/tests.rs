use crate::index::Abstract;
use crate::index::Concrete;
use crate::index::Detached;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::Params as _;
use crate::params::ParamsExt as _;
use crate::utils::each_capacity;
use crate::utils::std::BTreeSet;
use crate::utils::std::format;

#[expect(clippy::clone_on_copy, reason = "exercises the `Clone` impl")]
#[test]
fn abstract_clone_copy() {
  let a: Abstract<()> = Abstract::new(123);
  let b: Abstract<()> = a.clone();
  let c: Abstract<()> = b;

  assert_eq!(a, b);
  assert_eq!(b, c);
  assert_eq!(c, a);
}

#[expect(clippy::clone_on_copy, reason = "exercises the `Clone` impl")]
#[test]
fn concrete_clone_copy() {
  let a: Concrete<()> = Concrete::new(123);
  let b: Concrete<()> = a.clone();
  let c: Concrete<()> = b;

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

#[test]
fn detached_bits_roundtrip() {
  let data: usize = usize::MAX >> 2;
  let bits: usize = Detached::from_bits(data).into_bits();

  assert_eq!(data, bits);
}

#[test]
fn detached_decompose() {
  each_capacity!({
    for generation in 0..3 {
      let offset: usize = generation * P::LENGTH.as_usize();

      for index in 0..P::LENGTH.as_usize() {
        let abstract_idx: Abstract<P> = Abstract::new(offset + index);
        let detached_idx: Detached = Detached::from_abstract(abstract_idx);

        let components: (u32, u32) = detached_idx.decompose::<P>();

        assert_eq!(components.0 as usize, index);
        assert_eq!(components.1 as usize, generation);
      }
    }
  });
}

#[test]
fn abstract_to_concrete_covers_all_slots() {
  each_capacity!({
    let mut used: BTreeSet<usize> = BTreeSet::new();

    for index in 0..P::LENGTH.as_usize() {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let concrete_idx: Concrete<P> = Concrete::from_abstract(abstract_idx);

      used.insert(concrete_idx.get());
    }

    assert_eq!(used.len(), P::LENGTH.as_usize());
    assert_eq!(used.iter().min(), Some(&0));
    assert_eq!(used.iter().max(), Some(&(P::LENGTH.as_usize() - 1)));
  });
}

#[test]
fn abstract_to_detached_roundtrip() {
  each_capacity!({
    for index in 0..P::LENGTH.as_usize() {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let detached_idx: Detached = Detached::from_abstract(abstract_idx);
      let recovery_idx: Abstract<P> = Abstract::from_detached(detached_idx);

      assert_eq!(abstract_idx, recovery_idx);
    }
  });
}

#[test]
fn concrete_from_detached_matches_from_abstract() {
  each_capacity!({
    for index in 0..P::LENGTH.as_usize() {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let detached_idx: Detached = Detached::from_abstract(abstract_idx);

      let from_abstract: Concrete<P> = Concrete::from_abstract(abstract_idx);
      let from_detached: Concrete<P> = Concrete::from_detached(detached_idx);

      assert_eq!(from_abstract, from_detached);
    }
  });
}

#[test]
fn cache_line_distribution() {
  each_capacity!('block: {
    if P::BLOCKS.get() <= 1 {
      break 'block;
    }

    let mut blocks: BTreeSet<usize> = BTreeSet::new();

    for index in 0..P::LENGTH.as_usize() {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let concrete_idx: Concrete<P> = Concrete::from_abstract(abstract_idx);

      blocks.insert(concrete_idx.get() / CACHE_LINE_SLOTS);
    }

    assert_eq!(blocks.len(), P::BLOCKS.get());
    assert_eq!(blocks.iter().min(), Some(&0));
    assert_eq!(blocks.iter().max(), Some(&(P::BLOCKS.get() - 1)));
  });
}

#[test]
fn serial_number_preservation() {
  each_capacity!({
    for generation in 0..16 {
      let serial: usize = generation * P::LENGTH.as_usize();

      for index in 0..P::LENGTH.as_usize() {
        let abstract_idx: Abstract<P> = Abstract::new(serial + index);
        let detached_idx: Detached = Detached::from_abstract(abstract_idx);
        let recovery_idx: Abstract<P> = Abstract::from_detached(detached_idx);

        assert_eq!(abstract_idx.get(), serial + index);
        assert_eq!(abstract_idx, recovery_idx);
      }
    }
  });
}
