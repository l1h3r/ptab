use core::cmp::Ordering;

use crate::index::Abstract;
use crate::index::Concrete;
use crate::index::Detached;
use crate::index::RESERVED;
use crate::index::abstract_next_serial;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::Params;
use crate::params::ParamsExt as _;
use crate::utils::each_capacity;
use crate::utils::hash;
use crate::utils::std::BTreeSet;
use crate::utils::std::Vec;
use crate::utils::std::format;
use crate::utils::std::vec;

fn samples<P>() -> [usize; 9]
where
  P: Params,
{
  let length: usize = P::LENGTH.as_usize();

  [
    0,
    1,
    length - 1,
    length,
    2 * length - 1,
    RESERVED - length,
    RESERVED - 1,
    RESERVED,
    usize::MAX,
  ]
}

// -----------------------------------------------------------------------------
// Abstract
// -----------------------------------------------------------------------------

#[test]
fn abstract_get() {
  assert_eq!(Abstract::<()>::new(123).get(), 123);
}

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

#[test]
fn abstract_eq() {
  assert_eq!(Abstract::<()>::new(123), Abstract::new(123));
  assert_ne!(Abstract::<()>::new(123), Abstract::new(456));
}

#[test]
fn abstract_debug() {
  assert_eq!(format!("{:?}", Abstract::<()>::new(123)), "123");
}

#[test]
fn abstract_round_trips_through_detached() {
  each_capacity!({
    for value in samples::<P>() {
      let index: Abstract<P> = Abstract::new(value);

      assert_eq!(
        Abstract::from_detached(Detached::from_abstract(index)),
        index
      );
    }
  });
}

// -----------------------------------------------------------------------------
// Concrete
// -----------------------------------------------------------------------------

#[test]
fn concrete_get() {
  assert_eq!(Concrete::<()>::new(123).get(), 123);
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
fn concrete_eq() {
  assert_eq!(Concrete::<()>::new(123), Concrete::new(123));
  assert_ne!(Concrete::<()>::new(123), Concrete::new(456));
}

#[test]
fn concrete_debug() {
  assert_eq!(format!("{:?}", Concrete::<()>::new(123)), "123");
}

#[test]
fn concrete_from_abstract_is_in_bounds() {
  each_capacity!({
    for value in samples::<P>() {
      assert!(Concrete::from_abstract(Abstract::<P>::new(value)).get() < S);
    }
  });
}

#[test]
fn concrete_from_abstract_is_injective() {
  each_capacity!({
    let mut seen: Vec<bool> = vec![false; S];

    for value in 0..S {
      let slot: usize = Concrete::from_abstract(Abstract::<P>::new(value)).get();

      assert!(!seen[slot]);
      seen[slot] = true;
    }
  });
}

#[test]
fn concrete_from_abstract_is_surjective() {
  each_capacity!({
    let mut seen: Vec<bool> = vec![false; S];

    for value in 0..S {
      seen[Concrete::from_abstract(Abstract::<P>::new(value)).get()] = true;
    }

    assert!(seen.iter().all(|slot| *slot));
  });
}

#[test]
fn concrete_from_abstract_ignores_serial() {
  each_capacity!({
    for value in samples::<P>() {
      assert_eq!(
        Concrete::from_abstract(Abstract::<P>::new(value)),
        Concrete::from_abstract(Abstract::new(value & P::ID_MASK_ENTRY)),
      );
    }
  });
}

#[test]
fn concrete_from_abstract_block_is_index_mod_blocks() {
  each_capacity!({
    for value in 0..S {
      let slot: usize = Concrete::from_abstract(Abstract::<P>::new(value)).get();

      assert_eq!(slot / CACHE_LINE_SLOTS, value % P::BLOCKS);
    }
  });
}

#[test]
fn concrete_from_abstract_separates_neighbours() {
  each_capacity!({
    for value in samples::<P>() {
      let this: usize = Concrete::from_abstract(Abstract::<P>::new(value)).get();
      let next: usize = Concrete::from_abstract(Abstract::<P>::new(value.wrapping_add(1))).get();

      assert_ne!(this / CACHE_LINE_SLOTS, next / CACHE_LINE_SLOTS);
    }
  });
}

#[test]
fn concrete_from_detached_is_in_bounds() {
  each_capacity!({
    for bits in samples::<P>() {
      assert!(Concrete::<P>::from_detached(Detached::from_bits(bits)).get() < S);
    }
  });
}

#[test]
fn concrete_from_detached_matches_from_abstract() {
  each_capacity!({
    for bits in samples::<P>() {
      let index: Detached = Detached::from_bits(bits);

      assert_eq!(
        Concrete::<P>::from_detached(index),
        Concrete::from_abstract(Abstract::from_detached(index)),
      );
    }
  });
}

// -----------------------------------------------------------------------------
// Detached
// -----------------------------------------------------------------------------

#[test]
fn detached_into_bits() {
  assert_eq!(Detached::from_bits(123).into_bits(), 123);
}

#[expect(clippy::clone_on_copy, reason = "exercises the `Clone` impl")]
#[test]
fn detached_clone_copy() {
  let a: Detached = Detached::from_bits(123);
  let b: Detached = a.clone();
  let c: Detached = b;

  assert_eq!(a, b);
  assert_eq!(b, c);
  assert_eq!(c, a);
}

#[test]
fn detached_eq() {
  assert_eq!(Detached::from_bits(123), Detached::from_bits(123));
  assert_ne!(Detached::from_bits(123), Detached::from_bits(456));
}

#[test]
fn detached_hash() {
  assert_eq!(
    hash(Detached::from_bits(123)),
    hash(Detached::from_bits(123))
  );
}

#[test]
fn detached_ord() {
  assert_eq!(
    Detached::from_bits(123).cmp(&Detached::from_bits(456)),
    Ordering::Less
  );
  assert!(Detached::from_bits(123) < Detached::from_bits(456));
}

#[test]
fn detached_debug() {
  assert_eq!(format!("{:?}", Detached::from_bits(123)), "123");
}

#[test]
fn detached_display() {
  assert_eq!(format!("{}", Detached::from_bits(123)), "123");
}

#[test]
fn detached_decompose_number() {
  each_capacity!({
    for value in samples::<P>() {
      let index: Detached = Detached::from_abstract(Abstract::<P>::new(value));

      assert_eq!(index.decompose::<P>().0, value & P::ID_MASK_ENTRY);
    }
  });
}

#[test]
fn detached_decompose_serial() {
  each_capacity!({
    for value in samples::<P>() {
      let index: Detached = Detached::from_abstract(Abstract::<P>::new(value));

      assert_eq!(index.decompose::<P>().1, value >> P::ID_MASK_BITS);
    }
  });
}

#[test]
fn detached_decompose_round_trips() {
  each_capacity!({
    for value in samples::<P>() {
      let index: Detached = Detached::from_abstract(Abstract::<P>::new(value));
      let (number, serial): (usize, usize) = index.decompose::<P>();

      assert_eq!(
        Abstract::<P>::new((serial << P::ID_MASK_BITS) | number),
        Abstract::from_detached(index),
      );
    }
  });
}

#[test]
fn detached_round_trips_through_abstract() {
  each_capacity!({
    for bits in samples::<P>() {
      let index: Detached = Detached::from_bits(bits);

      assert_eq!(
        Detached::from_abstract(Abstract::<P>::from_detached(index)),
        index
      );
    }
  });
}

// -----------------------------------------------------------------------------
// Next Serial
// -----------------------------------------------------------------------------

#[test]
fn reserved_is_all_ones_in_width() {
  assert_eq!(RESERVED, (1 << (usize::BITS - 4)) - 1);
}

#[test]
fn abstract_next_serial_is_below_reserved() {
  each_capacity!({
    for value in samples::<P>().into_iter().filter(|value| *value < RESERVED) {
      assert!(abstract_next_serial(Abstract::<P>::new(value)).get() < RESERVED);
    }
  });
}

#[test]
fn abstract_next_serial_keeps_slot() {
  each_capacity!({
    for value in 0..S {
      let index: Abstract<P> = Abstract::new(value);

      assert_eq!(
        Concrete::from_abstract(abstract_next_serial(index)),
        Concrete::from_abstract(index),
      );
    }
  });
}

#[test]
fn abstract_next_serial_adds_length() {
  each_capacity!({
    let index: Abstract<P> = Abstract::new(RESERVED - S - 1);

    assert_eq!(abstract_next_serial(index).get(), RESERVED - 1);
  });
}

#[test]
fn abstract_next_serial_skips_reserved() {
  each_capacity!({
    let index: Abstract<P> = Abstract::new(RESERVED - S);

    assert_eq!(abstract_next_serial(index).get(), S - 1);
  });
}

#[test]
fn abstract_next_serial_wraps_at_width() {
  each_capacity!({
    let index: Abstract<P> = Abstract::new(RESERVED - (S - 1));

    assert_eq!(abstract_next_serial(index).get(), 0);
  });
}

#[test]
fn abstract_next_serial_is_injective() {
  each_capacity!({
    let mut seen: BTreeSet<usize> = BTreeSet::new();

    for value in (0..S).chain(RESERVED - S..RESERVED) {
      assert!(seen.insert(abstract_next_serial(Abstract::<P>::new(value)).get()));
    }
  });
}
