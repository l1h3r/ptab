use core::fmt::Debug;
use core::fmt::Display;
use core::fmt::Formatter;
use core::fmt::Result;

use crate::params::Params;
use crate::params::ParamsExt;

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(test)]
mod tests;

// -----------------------------------------------------------------------------
// Implementation
// -----------------------------------------------------------------------------

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
      pub(crate) const fn new(source: usize) -> Self {
        Self {
          source,
          marker: ::core::marker::PhantomData,
        }
      }

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

/// An opaque index naming a table entry, returned by [`PTab::insert`] and
/// [`PTab::write`].
///
/// # Generational Indices
///
/// An index carries a serial that changes when its slot is reused.
///
/// # Examples
///
/// ```
/// use ptab::PTab;
///
/// let table: PTab<i32> = PTab::new();
///
/// let idx = table.insert(42).unwrap();
/// assert_eq!(table.read(idx), Some(42));
/// ```
///
/// [`PTab::insert`]: crate::public::PTab::insert
/// [`PTab::write`]: crate::public::PTab::write
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Detached {
  bits: usize,
}

impl Detached {
  /// Creates a [`Detached`] index from its raw bit representation.
  ///
  /// Table operations accept any `bits` and return [`None`] or `false` if it
  /// names no entry.
  #[inline]
  pub const fn from_bits(bits: usize) -> Self {
    Self { bits }
  }

  /// Returns the raw bit representation of this index.
  ///
  /// See [`from_bits`] to reconstruct an index.
  ///
  /// [`from_bits`]: Self::from_bits
  #[inline]
  pub const fn into_bits(self) -> usize {
    self.bits
  }

  /// Translates this index into its `(number, serial)` components.
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
  Concrete::new(detached.into_bits() & P::ID_MASK_ENTRY)
}

/// Converts an [`Abstract`] sequential index to a [`Concrete`] cache-aware index.
#[inline]
const fn abstract_to_concrete<P>(abstract_idx: Abstract<P>) -> Concrete<P>
where
  P: Params + ?Sized,
{
  let mut value: usize = 0;
  value += (abstract_idx.get() & P::ID_MASK_BLOCK) << P::ID_SHIFT_BLOCK;
  value += (abstract_idx.get() >> P::ID_SHIFT_INDEX) & P::ID_MASK_INDEX;
  Concrete::new(value)
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

/// Marks a free-list entry whose index a thread holds, all ones in the data
/// width of a [`Detached`] index.
pub(crate) const RESERVED: usize = usize::MAX >> 4;

/// Returns the [`Abstract`] index that follows `abstract_idx` in its slot.
#[inline]
pub(crate) const fn abstract_next_serial<P>(abstract_idx: Abstract<P>) -> Abstract<P>
where
  P: Params + ?Sized,
{
  let length: usize = P::LENGTH.as_usize();
  let mut value: usize = abstract_idx.get().wrapping_add(length) & RESERVED;

  if value == RESERVED {
    value = value.wrapping_add(length) & RESERVED;
  }

  Abstract::new(value)
}
