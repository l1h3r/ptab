use core::fmt::Debug;
use core::fmt::Display;
use core::fmt::Formatter;
use core::fmt::Result;
use core::marker::PhantomData;

use crate::params::Params;
use crate::params::ParamsExt as _;

macro_rules! internal_index {
  ($name:ident) => {
    #[repr(transparent)]
    pub(crate) struct $name<P>
    where
      P: ?Sized,
    {
      source: usize,
      marker: ::core::marker::PhantomData<fn(P)>,
    }

    impl<P> $name<P>
    where
      P: ?Sized,
    {
      #[inline]
      pub(crate) const fn get(self) -> usize {
        self.source
      }
    }

    impl<P> Clone for $name<P>
    where
      P: ?Sized,
    {
      #[inline]
      fn clone(&self) -> Self {
        *self
      }
    }

    impl<P> Copy for $name<P> where P: ?Sized {}

    impl<P> ::core::cmp::PartialEq for $name<P>
    where
      P: ?Sized,
    {
      #[inline]
      fn eq(&self, other: &Self) -> bool {
        self.source == other.source
      }
    }

    impl<P> ::core::fmt::Debug for $name<P>
    where
      P: ?Sized,
    {
      #[inline]
      fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
        ::core::fmt::Debug::fmt(&self.source, f)
      }
    }
  };
}

// -----------------------------------------------------------------------------
// Detached Index
// -----------------------------------------------------------------------------

/// An opaque index identifying a table entry.
///
/// Returned by [`PTab::insert`] and [`PTab::write`]. Used to access or remove
/// the entry.
///
/// # Stale Indices
///
/// Each index carries a serial that changes when a slot is reused, but lookup
/// and removal read only the slot bits. An index kept past its entry's removal
/// reaches the entry that later occupies the same slot.
///
/// # Examples
///
/// ```
/// use ptab::PTab;
///
/// let table: PTab<i32> = PTab::new();
///
/// let idx = table.insert(42).unwrap();
///
/// // Access the entry by index
/// assert_eq!(table.read(idx), Some(42));
///
/// // Indices are Copy
/// let idx2 = idx;
/// assert_eq!(table.read(idx2), Some(42));
/// ```
///
/// [`PTab`]: crate::public::PTab
/// [`PTab::insert`]: crate::public::PTab::insert
/// [`PTab::write`]: crate::public::PTab::write
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Detached {
  bits: usize,
}

impl Detached {
  /// Creates a [`Detached`] index from its raw bits.
  ///
  /// # Warning
  ///
  /// Table operations use only the slot bits of the index. An arbitrary bit
  /// pattern is safe to use, and reaches whatever entry occupies that slot, or
  /// returns [`None`] or `false` when the slot is empty.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::Detached;
  ///
  /// let index = Detached::from_bits(7);
  /// assert_eq!(index.into_bits(), 7);
  /// ```
  #[inline]
  pub const fn from_bits(bits: usize) -> Self {
    Self { bits }
  }

  /// Returns the raw bits of this index.
  ///
  /// See [`from_bits`] to reconstruct an index.
  ///
  /// [`from_bits`]: Self::from_bits
  #[inline]
  pub const fn into_bits(self) -> usize {
    self.bits
  }

  /// Returns the `(number, serial)` components of this index under `P`.
  ///
  /// `number` is the allocation counter modulo [`Params::LENGTH`]. `serial` is
  /// the counter divided by [`Params::LENGTH`], which grows each time the slot
  /// is reused.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::{ConstParams, PTab};
  ///
  /// let table: PTab<i32, ConstParams<16>> = PTab::new();
  /// let first = table.insert(1).unwrap();
  /// let second = table.insert(2).unwrap();
  ///
  /// assert_eq!(first.decompose::<ConstParams<16>>(), (0, 0));
  /// assert_eq!(second.decompose::<ConstParams<16>>(), (1, 0));
  /// ```
  ///
  /// [`Params::LENGTH`]: crate::params::Params::LENGTH
  #[inline]
  pub const fn decompose<P>(self) -> (u32, u32)
  where
    P: Params + ?Sized,
  {
    let abstract_idx: Abstract<P> = detached_to_abstract(self);

    let number: u32 = (abstract_idx.get() & P::ID_MASK_ENTRY) as u32;
    let serial: u32 = (abstract_idx.get() >> P::ID_MASK_BITS) as u32;

    (number, serial)
  }
}

impl Debug for Detached {
  #[inline]
  fn fmt(&self, f: &mut Formatter<'_>) -> Result {
    Debug::fmt(&self.bits, f)
  }
}

impl Display for Detached {
  #[inline]
  fn fmt(&self, f: &mut Formatter<'_>) -> Result {
    Display::fmt(&self.bits, f)
  }
}

impl Detached {
  #[inline]
  pub(crate) const fn from_abstract<P>(other: Abstract<P>) -> Self
  where
    P: Params + ?Sized,
  {
    abstract_to_detached(other)
  }
}

// -----------------------------------------------------------------------------
// Abstract Index
// -----------------------------------------------------------------------------

internal_index!(Abstract);

impl<P> Abstract<P>
where
  P: ?Sized,
{
  #[inline]
  pub(crate) const fn new(source: usize) -> Self {
    Self {
      source,
      marker: PhantomData,
    }
  }
}

impl<P> Abstract<P>
where
  P: Params + ?Sized,
{
  #[inline]
  pub(crate) const fn from_detached(other: Detached) -> Self {
    detached_to_abstract(other)
  }
}

// -----------------------------------------------------------------------------
// Concrete Index
// -----------------------------------------------------------------------------

internal_index!(Concrete);

impl<P> Concrete<P>
where
  P: Params + ?Sized,
{
  /// Creates a [`Concrete`] index from a raw value.
  ///
  /// # Safety
  ///
  /// `source` must be strictly less than `P::LENGTH`.
  #[inline]
  pub(crate) const unsafe fn new(source: usize) -> Self {
    debug_assert!(source < P::LENGTH.as_usize());

    Self {
      source,
      marker: PhantomData,
    }
  }

  #[inline]
  pub(crate) const fn from_abstract(other: Abstract<P>) -> Self {
    abstract_to_concrete(other)
  }

  #[inline]
  pub(crate) const fn from_detached(other: Detached) -> Self {
    detached_to_concrete(other)
  }
}

// -----------------------------------------------------------------------------
// Index Assertions
// -----------------------------------------------------------------------------

/// Post-monomorphization checks on the index masks of `P`.
struct AssertIndexMasks<P>(PhantomData<fn(P)>)
where
  P: ?Sized;

impl<P> AssertIndexMasks<P>
where
  P: Params + ?Sized,
{
  /// ID_MASK_COVERS_LENGTH: the block and slot masks together address exactly
  /// `0..LENGTH`.
  const COVERS: () = assert!(
    Self::covers(P::ID_MASK_BLOCK, P::ID_MASK_INDEX, P::LENGTH.as_usize()),
    "invalid params: index masks must address exactly `0..LENGTH`",
  );

  /// Returns `true` if a block mask and a slot mask address exactly `length` slots.
  const fn covers(mask_block: usize, mask_index: usize, length: usize) -> bool {
    match (mask_block.checked_add(1), mask_index.checked_add(1)) {
      (Some(blocks), Some(slots)) => match blocks.checked_mul(slots) {
        Some(total) => total == length,
        None => false,
      },
      _ => false,
    }
  }
}

// -----------------------------------------------------------------------------
// Index Mapping
// -----------------------------------------------------------------------------

/// Extracts the [`Abstract`] sequential index from a [`Detached`] index.
#[inline]
const fn detached_to_abstract<P>(detached: Detached) -> Abstract<P>
where
  P: Params + ?Sized,
{
  let mut value: usize = detached.into_bits() & !P::ID_MASK_ENTRY;
  value |= (detached.into_bits() >> P::ID_SHIFT_BLOCK) & P::ID_MASK_BLOCK;
  value |= (detached.into_bits() & P::ID_MASK_INDEX) << P::ID_SHIFT_INDEX;
  Abstract::new(value)
}

/// Extracts the [`Concrete`] cache-aware index from a [`Detached`] index.
#[inline]
const fn detached_to_concrete<P>(detached: Detached) -> Concrete<P>
where
  P: Params + ?Sized,
{
  // SAFETY: `ID_MASK_ENTRY == LENGTH - 1`, so the masked value is strictly
  //         less than `LENGTH`.
  unsafe { Concrete::new(detached.into_bits() & P::ID_MASK_ENTRY) }
}

/// Converts an [`Abstract`] sequential index to a [`Concrete`] cache-aware index.
#[inline]
const fn abstract_to_concrete<P>(abstract_idx: Abstract<P>) -> Concrete<P>
where
  P: Params + ?Sized,
{
  let () = AssertIndexMasks::<P>::COVERS;
  let mut value: usize = 0;
  value += (abstract_idx.get() & P::ID_MASK_BLOCK) << P::ID_SHIFT_BLOCK;
  value += (abstract_idx.get() >> P::ID_SHIFT_INDEX) & P::ID_MASK_INDEX;
  // SAFETY: `AssertIndexMasks::<P>::COVERS` fails the build unless the block and
  //         slot masks address exactly `0..LENGTH`, so `value` is less than `LENGTH`.
  unsafe { Concrete::new(value) }
}

/// Converts an [`Abstract`] sequential index to a [`Detached`] index.
#[inline]
const fn abstract_to_detached<P>(abstract_idx: Abstract<P>) -> Detached
where
  P: Params + ?Sized,
{
  let index: usize = abstract_idx.get() & !P::ID_MASK_ENTRY;
  let index: usize = index | abstract_to_concrete(abstract_idx).get();
  let value: Detached = Detached::from_bits(index);
  debug_assert!(detached_to_abstract::<P>(value).get() == abstract_idx.get());
  value
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(test)]
mod tests {
  use std::collections::HashSet;

  use crate::index::Abstract;
  use crate::index::AssertIndexMasks;
  use crate::index::Concrete;
  use crate::index::Detached;
  use crate::params::CACHE_LINE_SLOTS;
  use crate::params::DefaultParams;
  use crate::params::Params as _;
  use crate::params::ParamsExt as _;
  use crate::utils::each_capacity;

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
    // SAFETY: 7 is below the minimum capacity of 16.
    let a: Concrete<DefaultParams> = unsafe { Concrete::new(7) };
    let b: Concrete<DefaultParams> = a.clone();
    let c: Concrete<DefaultParams> = b;

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
    let value: usize = 7;
    // SAFETY: 7 is below the minimum capacity of 16.
    let index: Concrete<DefaultParams> = unsafe { Concrete::new(value) };

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
      let mut used: HashSet<usize> = HashSet::with_capacity(P::LENGTH.as_usize());

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
    // Verify that consecutive base indices are distributed across cache lines
    each_capacity!('block: {
      if P::BLOCKS.get() <= 1 {
        break 'block; // Skip solo blocks
      }

      // First `CACHE_LINE_SLOTS` indices should map to different blocks
      let mut blocks: HashSet<usize> = HashSet::with_capacity(CACHE_LINE_SLOTS);

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

  #[test]
  fn covers_rejects_masks_past_length() {
    assert!(!AssertIndexMasks::<DefaultParams>::covers(0, 31, 16));
    assert!(AssertIndexMasks::<DefaultParams>::covers(0, 15, 16));
    assert!(AssertIndexMasks::<DefaultParams>::covers(1, 7, 16));
    assert!(!AssertIndexMasks::<DefaultParams>::covers(
      usize::MAX,
      0,
      16
    ));
  }
}
