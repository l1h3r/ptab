use core::alloc::Layout;
use core::any;
use core::fmt::Debug;
use core::fmt::Formatter;
use core::fmt::Result as FmtResult;
use core::marker::PhantomData;
use core::mem;
use core::num::NonZeroUsize;

use sdd::AtomicOwned;

use crate::padded::CachePadded;
use crate::sync::atomic::AtomicUsize;

// -----------------------------------------------------------------------------
// SDD Sanity Check
// -----------------------------------------------------------------------------

const _: () = assert!(
  align_of::<AtomicOwned<()>>() == align_of::<AtomicUsize>(),
  "invalid system: atomic owned align != atomic usize align",
);

const _: () = assert!(
  size_of::<AtomicOwned<()>>() == size_of::<AtomicUsize>(),
  "invalid system: atomic owned width != atomic usize width",
);

// -----------------------------------------------------------------------------
// Cache-line Properties
// -----------------------------------------------------------------------------

/// The size of a cache line in bytes.
///
/// This value is used to align table data structures to minimize false sharing
/// between threads. On most modern x86-64 systems, this is 64 bytes.
///
/// The table distributes consecutive allocations across different cache lines
/// to reduce contention when multiple threads operate on recently-allocated
/// entries.
pub const CACHE_LINE: usize = size_of::<CachePadded<u8>>();

/// The number of table slots that fit in a single cache line.
///
/// This determines the stride used when distributing allocations across
/// cache lines. See [`CACHE_LINE`] for more details.
pub const CACHE_LINE_SLOTS: usize = CACHE_LINE / size_of::<AtomicUsize>();

const _: () = assert!(
  CACHE_LINE.is_multiple_of(size_of::<AtomicUsize>()),
  "invalid params: `CACHE_LINE` must be a multiple of pointer width",
);

const _: () = assert!(
  CACHE_LINE.is_power_of_two(),
  "invalid params: `CACHE_LINE` must be a power of two",
);

const _: () = assert!(
  CACHE_LINE_SLOTS.is_power_of_two(),
  "invalid params: `CACHE_LINE_SLOTS` must be a power of two",
);

// -----------------------------------------------------------------------------
// Configurable Params
// -----------------------------------------------------------------------------

/// Configuration parameters for a [`PTab`].
///
/// This trait allows customizing table capacity at compile time. The simplest
/// way to use custom parameters is through [`ConstParams`]:
///
/// ```
/// use ptab::{PTab, ConstParams};
///
/// // Table with 8,192 slots
/// type MyTable<T> = PTab<T, ConstParams<8192>>;
/// ```
///
/// # Implementing `Params`
///
/// For advanced use cases, you can implement `Params` directly:
///
/// ```
/// use ptab::{Params, Capacity, PTab};
///
/// struct LargeParams;
///
/// impl Params for LargeParams {
///   const LENGTH: Capacity = Capacity::new(1 << 20);
/// }
///
/// let table: PTab<u64, LargeParams> = PTab::new();
/// assert_eq!(table.capacity(), 1 << 20);
/// ```
///
/// Note that [`Capacity::new`] clamps values to the valid range and rounds
/// up to the nearest power of two.
///
/// [`PTab`]: crate::PTab
pub trait Params {
  /// The maximum number of entries the table can hold.
  ///
  /// This value is rounded up to the nearest power of two and clamped to
  /// [`Capacity::MIN`]`..=`[`Capacity::MAX`].
  const LENGTH: Capacity = DefaultParams::LENGTH;
}

// -----------------------------------------------------------------------------
// Configurable Params - Extensions
// -----------------------------------------------------------------------------

/// Derived parameters computed from [`Params`].
///
/// This trait is automatically implemented for all types that implement
/// [`Params`]. It provides computed constants used internally by the table
/// implementation.
///
/// Users generally do not need to interact with this trait directly, but
/// it is exposed for advanced use cases such as debugging configuration.
///
/// # Example
///
/// ```
/// use ptab::{Params, ParamsExt, ConstParams};
///
/// // View derived parameters for a configuration
/// println!("{:#?}", <ConstParams<1024> as ParamsExt>::debug());
/// ```
pub trait ParamsExt: Params + Sealed {
  const BLOCKS: NonZeroUsize = derive_blocks::<Self>();
  const LAYOUT: Layout = derive_layout::<Self>();
  const MEMORY: usize = Self::BLOCKS.get().strict_mul(CACHE_LINE);

  const ID_MASK_BITS: u32 = Self::LENGTH.log2();
  const ID_MASK_ENTRY: usize = 1_usize.strict_shl(Self::ID_MASK_BITS).strict_sub(1);
  const ID_MASK_BLOCK: usize = Self::BLOCKS.get().strict_sub(1);
  const ID_MASK_INDEX: usize = CACHE_LINE_SLOTS.strict_sub(1);
  const ID_SHIFT_BLOCK: u32 = Self::ID_MASK_INDEX.trailing_ones();
  const ID_SHIFT_INDEX: u32 = Self::ID_MASK_BLOCK.trailing_ones();

  fn validate() {
    assert_ne!(Self::LAYOUT.size(), 0, "invalid params: layout size is `0`");
  }

  #[inline]
  fn debug() -> DebugParams<Self> {
    DebugParams {
      marker: PhantomData,
    }
  }
}

// -----------------------------------------------------------------------------
// Debug Params
// -----------------------------------------------------------------------------

/// A helper type for displaying [`Params`] configuration.
///
/// This type is returned by [`ParamsExt::debug`] and implements [`Debug`]
/// to display all derived configuration values.
///
/// # Example
///
/// ```
/// use ptab::{ParamsExt, DefaultParams};
///
/// let debug = <DefaultParams as ParamsExt>::debug();
/// println!("{:#?}", debug);
/// ```
#[derive(Clone, Copy)]
pub struct DebugParams<P>
where
  P: ?Sized,
{
  marker: PhantomData<fn(P)>,
}

impl<P> Debug for DebugParams<P>
where
  P: Params + ?Sized,
{
  fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
    f.debug_struct(any::type_name::<P>())
      .field("LENGTH", &P::LENGTH)
      .field("BLOCKS", &P::BLOCKS)
      .field("LAYOUT", &P::LAYOUT)
      .field("MEMORY", &P::MEMORY)
      .field("ID_MASK_BITS", &P::ID_MASK_BITS)
      .field("ID_MASK_ENTRY", &format_args!("{:0>32b}", P::ID_MASK_ENTRY))
      .field("ID_MASK_BLOCK", &format_args!("{:0>32b}", P::ID_MASK_BLOCK))
      .field("ID_MASK_INDEX", &format_args!("{:0>32b}", P::ID_MASK_INDEX))
      .field("ID_SHIFT_BLOCK", &P::ID_SHIFT_BLOCK)
      .field("ID_SHIFT_INDEX", &P::ID_SHIFT_INDEX)
      .finish()
  }
}

// -----------------------------------------------------------------------------
// Default Params
// -----------------------------------------------------------------------------

/// The default table configuration with 1,048,576 slots.
///
/// This is the configuration used when creating a [`PTab`] without
/// specifying a custom [`Params`] type.
///
/// # Example
///
/// ```
/// use ptab::{PTab, DefaultParams};
///
/// // These are equivalent:
/// let table1: PTab<u64> = PTab::new();
/// let table2: PTab<u64, DefaultParams> = PTab::new();
///
/// assert_eq!(table1.capacity(), 1_048_576);
/// ```
///
/// [`PTab`]: crate::PTab
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct DefaultParams;

impl Debug for DefaultParams {
  fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
    Debug::fmt(&<Self as ParamsExt>::debug(), f)
  }
}

impl Params for DefaultParams {
  const LENGTH: Capacity = Capacity::DEF;
}

// -----------------------------------------------------------------------------
// Const-Generic Params
// -----------------------------------------------------------------------------

/// A [`Params`] implementation with compile-time configurable capacity.
///
/// This is the recommended way to create tables with custom capacities.
/// The capacity `N` is rounded up to the nearest power of two and clamped
/// to [`Capacity::MIN`]`..=`[`Capacity::MAX`].
///
/// # Examples
///
/// ```
/// use ptab::{PTab, ConstParams};
///
/// // Create a table with 4,096 slots
/// let table: PTab<String, ConstParams<4096>> = PTab::new();
/// assert_eq!(table.capacity(), 4096);
/// ```
///
/// ```
/// use ptab::{PTab, ConstParams};
///
/// // Values are rounded up to powers of two
/// let table: PTab<String, ConstParams<1000>> = PTab::new();
/// assert_eq!(table.capacity(), 1024);
/// ```
///
/// # Type Aliases
///
/// For frequently-used configurations, consider defining a type alias:
///
/// ```
/// use ptab::{PTab, ConstParams};
///
/// type SmallTable<T> = PTab<T, ConstParams<64>>;
/// type LargeTable<T> = PTab<T, ConstParams<{ 1 << 20 }>>;
/// ```
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConstParams<const N: usize>;

impl<const N: usize> Params for ConstParams<N> {
  const LENGTH: Capacity = Capacity::new(N);
}

// -----------------------------------------------------------------------------
// Auto-implement Derive
// -----------------------------------------------------------------------------

mod private {
  pub trait Sealed {}
}

use private::Sealed;

impl<P> Sealed for P where P: Params + ?Sized {}
impl<P> ParamsExt for P where P: Params + ?Sized {}

// -----------------------------------------------------------------------------
// Capacity
// -----------------------------------------------------------------------------

/// A validated table capacity value.
///
/// `Capacity` represents a power-of-two value in the range [`MIN`]`..=`[`MAX`].
/// It is used by [`Params::LENGTH`] to specify how many entries a table can hold.
///
/// # Construction
///
/// Use [`Capacity::new`] to create a capacity from an arbitrary value. The
/// value is automatically rounded up to the nearest power of two and clamped
/// to the valid range.
///
/// ```
/// use ptab::Capacity;
///
/// // Exact power of two
/// let cap = Capacity::new(256);
/// assert_eq!(cap.as_usize(), 256);
///
/// // Rounded up
/// let cap = Capacity::new(100);
/// assert_eq!(cap.as_usize(), 128);
///
/// // Clamped to minimum
/// let cap = Capacity::new(1);
/// assert_eq!(cap, Capacity::MIN);
///
/// // Clamped to maximum
/// let cap = Capacity::new(usize::MAX);
/// assert_eq!(cap, Capacity::MAX);
/// ```
///
/// [`MIN`]: Self::MIN
/// [`MAX`]: Self::MAX
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct Capacity(CapacityEnum);

impl Capacity {
  /// The minimum supported capacity: 16 entries.
  pub const MIN: Self = Self(CapacityEnum::_Capacity1Shl4);

  /// The maximum supported capacity: 134,217,728 entries (2²⁷).
  pub const MAX: Self = Self(CapacityEnum::_Capacity1Shl27);

  /// The default capacity: 1,048,576 entries (2²⁰).
  pub const DEF: Self = Self(CapacityEnum::_Capacity1Shl20);

  /// Creates a new `Capacity` from an arbitrary value.
  ///
  /// The value is rounded up to the nearest power of two and clamped to
  /// [`MIN`]`..=`[`MAX`].
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::Capacity;
  ///
  /// assert_eq!(Capacity::new(100).as_usize(), 128);
  /// assert_eq!(Capacity::new(0), Capacity::MIN);
  /// ```
  ///
  /// [`MIN`]: Self::MIN
  /// [`MAX`]: Self::MAX
  #[inline]
  pub const fn new(value: usize) -> Self {
    let Some(capacity) = value.checked_next_power_of_two() else {
      return Self::MAX;
    };

    if capacity < Self::MIN.as_usize() {
      Self::MIN
    } else if capacity > Self::MAX.as_usize() {
      Self::MAX
    } else {
      // SAFETY: `capacity` is non-zero because values below `Self::MIN` take
      // the earlier branch.
      unsafe { Self::new_unchecked(capacity) }
    }
  }

  /// Creates a new `Capacity` without validation.
  ///
  /// # Safety
  ///
  /// `value` must be a power of two in the range [`MIN`]`..=`[`MAX`].
  ///
  /// [`MIN`]: Self::MIN
  /// [`MAX`]: Self::MAX
  #[inline]
  pub const unsafe fn new_unchecked(value: usize) -> Self {
    // SAFETY: Caller guarantees `value` is a valid `Capacity`.
    unsafe { mem::transmute::<usize, Self>(value) }
  }

  /// Returns the capacity as a [`usize`].
  #[inline]
  pub const fn as_usize(self) -> usize {
    self.0 as usize
  }

  /// Returns the capacity as a [`NonZeroUsize`].
  #[inline]
  pub const fn as_nonzero(self) -> NonZeroUsize {
    // SAFETY: All `Capacity` values are non-zero by construction.
    unsafe { mem::transmute::<Self, NonZeroUsize>(self) }
  }

  /// Returns the base-2 logarithm of the capacity.
  ///
  /// Since capacity is always a power of two, this is equivalent to
  /// the bit position of the single set bit.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::Capacity;
  ///
  /// assert_eq!(Capacity::new(1024).log2(), 10);
  /// assert_eq!(Capacity::MIN.log2(), 4);
  /// ```
  #[inline]
  pub const fn log2(self) -> u32 {
    self.as_nonzero().trailing_zeros()
  }
}

impl Debug for Capacity {
  fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
    write!(f, "{:?} (1 << {:?})", self.as_nonzero(), self.log2())
  }
}

impl Default for Capacity {
  #[inline]
  fn default() -> Capacity {
    Capacity::DEF
  }
}

impl From<Capacity> for NonZeroUsize {
  #[inline]
  fn from(other: Capacity) -> NonZeroUsize {
    other.as_nonzero()
  }
}

impl From<Capacity> for usize {
  #[inline]
  fn from(other: Capacity) -> usize {
    other.as_usize()
  }
}

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(usize)]
enum CapacityEnum {
  _Capacity1Shl4 = 1 << 4,
  _Capacity1Shl5 = 1 << 5,
  _Capacity1Shl6 = 1 << 6,
  _Capacity1Shl7 = 1 << 7,
  _Capacity1Shl8 = 1 << 8,
  _Capacity1Shl9 = 1 << 9,
  _Capacity1Shl10 = 1 << 10,
  _Capacity1Shl11 = 1 << 11,
  _Capacity1Shl12 = 1 << 12,
  _Capacity1Shl13 = 1 << 13,
  _Capacity1Shl14 = 1 << 14,
  _Capacity1Shl15 = 1 << 15,
  _Capacity1Shl16 = 1 << 16,
  _Capacity1Shl17 = 1 << 17,
  _Capacity1Shl18 = 1 << 18,
  _Capacity1Shl19 = 1 << 19,
  _Capacity1Shl20 = 1 << 20,
  _Capacity1Shl21 = 1 << 21,
  _Capacity1Shl22 = 1 << 22,
  _Capacity1Shl23 = 1 << 23,
  _Capacity1Shl24 = 1 << 24,
  _Capacity1Shl25 = 1 << 25,
  _Capacity1Shl26 = 1 << 26,
  _Capacity1Shl27 = 1 << 27,
}

// -----------------------------------------------------------------------------
// Misc. Utilities
// -----------------------------------------------------------------------------

const fn derive_blocks<P>() -> NonZeroUsize
where
  P: Params + ?Sized,
{
  // Determine the minimum number of bytes needed
  // to represent an array of pointer-sized slots.
  let Some(mem_bytes) = P::LENGTH.as_usize().checked_mul(size_of::<AtomicUsize>()) else {
    panic!("invalid params: `BLOCKS` must be representable");
  };

  // Round up so we have an even number of cache lines.
  let Some(mem_align) = mem_bytes.checked_next_multiple_of(CACHE_LINE) else {
    panic!("invalid params: `BLOCKS` must be representable");
  };

  // Make sure we don't overflow `isize::MAX`, otherwise the layout is invalid.
  assert!(
    mem_align <= isize::MAX as usize,
    "invalid params: `BLOCKS` must be representable",
  );

  // Finally, compute the block count.
  let Some(blocks) = NonZeroUsize::new(mem_align / CACHE_LINE) else {
    panic!("invalid params: `BLOCKS` must be representable");
  };

  blocks
}

const fn derive_layout<P>() -> Layout
where
  P: Params + ?Sized,
{
  // SAFETY: `CACHE_LINE` is a power of two. The size does not overflow
  // `isize::MAX` as verified by `derive_blocks`.
  unsafe { Layout::from_size_align_unchecked(P::MEMORY, CACHE_LINE) }
}
