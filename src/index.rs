//! Index types and conversions.

use core::fmt::Debug;
use core::fmt::Display;
use core::fmt::Formatter;
use core::fmt::Result;

use crate::params::Params;
use crate::params::ParamsExt;

macro_rules! internal_index {
  ($name:ident) => {
    #[repr(transparent)]
    pub(crate) struct $name<P>
    where
      P: $crate::params::Params + ?Sized,
    {
      source: usize,
      marker: ::core::marker::PhantomData<fn(P)>,
    }

    impl<P> $name<P>
    where
      P: $crate::params::Params + ?Sized,
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
      P: $crate::params::Params + ?Sized,
    {
      #[inline]
      fn clone(&self) -> Self {
        *self
      }
    }

    impl<P> Copy for $name<P> where P: $crate::params::Params + ?Sized {}

    impl<P> ::core::cmp::PartialEq for $name<P>
    where
      P: $crate::params::Params + ?Sized,
    {
      #[inline]
      fn eq(&self, other: &Self) -> bool {
        self.source == other.source
      }
    }

    impl<P> ::core::fmt::Debug for $name<P>
    where
      P: $crate::params::Params + ?Sized,
    {
      fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
        ::core::fmt::Debug::fmt(&self.source, f)
      }
    }
  };
}

// -----------------------------------------------------------------------------
// Detached Index
// -----------------------------------------------------------------------------

/// An opaque index identifying an entry in a [`PTab`].
///
/// `Detached` values are returned by [`PTab::insert`] and [`PTab::write`],
/// and can be used to access or remove entries.
///
/// # Generational Indices
///
/// Each index contains a generational component that changes when a slot is
/// reused. This helps mitigate the [ABA problem] in concurrent algorithms:
/// a stale index from a removed entry will not accidentally match a new entry
/// that happens to occupy the same slot.
///
/// # Examples
///
/// ```no_run
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
/// [`PTab`]: crate::PTab
/// [`PTab::insert`]: crate::PTab::insert
/// [`PTab::write`]: crate::PTab::write
/// [ABA problem]: https://en.wikipedia.org/wiki/ABA_problem
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Detached {
  bits: usize,
}

impl Detached {
  /// Creates a `Detached` index from its raw bit representation.
  ///
  /// This is primarily useful for serialization or interop with external
  /// systems that need to store indices as plain integers.
  ///
  /// # Warning
  ///
  /// The returned index may not correspond to any valid entry. Using an
  /// arbitrary bit pattern with table operations is safe but will likely
  /// return `None` or `false`.
  #[inline]
  pub const fn from_bits(bits: usize) -> Self {
    Self { bits }
  }

  /// Returns the raw bit representation of this index.
  ///
  /// This is primarily useful for serialization or debugging.
  #[inline]
  pub const fn into_bits(self) -> usize {
    self.bits
  }
}

impl Debug for Detached {
  fn fmt(&self, f: &mut Formatter<'_>) -> Result {
    Debug::fmt(&self.bits, f)
  }
}

impl Display for Detached {
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

  pub(crate) const fn from_detached(other: Detached) -> Self {
    detached_to_concrete(other)
  }
}

// -----------------------------------------------------------------------------
// Index Mapping
// -----------------------------------------------------------------------------

/// Extracts the abstract sequential index from a detached index.
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

/// Extracts the concrete cache-aware index from a detached index.
#[inline]
const fn detached_to_concrete<P>(detached: Detached) -> Concrete<P>
where
  P: Params + ?Sized,
{
  Concrete::new(detached.into_bits() & P::ID_MASK_ENTRY)
}

/// Converts an abstract sequential index to concrete cache-aware index.
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

/// Converts abstract sequential index to detached index.
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
