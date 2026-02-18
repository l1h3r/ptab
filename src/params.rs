use core::any;
use core::fmt::Debug;
use core::fmt::Formatter;
use core::fmt::Result as FmtResult;
use core::marker::PhantomData;
use core::mem;
use core::num::NonZeroUsize;

use crate::alloc::Layout;
use crate::padded::CachePadded;
use crate::sync::atomic::AtomicUsize;

// -----------------------------------------------------------------------------
// Cache-line Properties
// -----------------------------------------------------------------------------

/// The size of a cache line in bytes.
pub const CACHE_LINE: usize = size_of::<CachePadded<u8>>();

/// The number of table slots that fit in a single cache line.
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
/// Allows compile-time customization of table parameters. The simplest approach
/// is through [`ConstParams`]:
///
/// ```
/// use ptab::{PTab, ConstParams};
///
/// type MyTable<T> = PTab<T, ConstParams<8192>>;
/// ```
///
/// [`PTab`]: crate::public::PTab
pub trait Params {
  /// The maximum number of entries the table can hold.
  ///
  /// See [`Capacity`] for more info.
  const LENGTH: Capacity = DefaultParams::LENGTH;
}

// -----------------------------------------------------------------------------
// Configurable Params - Extensions
// -----------------------------------------------------------------------------

/// Derived parameters computed from [`Params`].
///
/// Automatically implemented for all [`Params`] types. Provides computed
/// constants used internally.
///
/// # Example
///
/// ```
/// use ptab::{config::ParamsExt, ConstParams};
///
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

  #[inline]
  fn debug() -> DebugParams<Self> {
    DebugParams {
      marker: PhantomData,
    }
  }
}

mod private {
  pub trait Sealed {}
}

use private::Sealed;

impl<P> Sealed for P where P: Params + ?Sized {}
impl<P> ParamsExt for P where P: Params + ?Sized {}

// -----------------------------------------------------------------------------
// Debug Params
// -----------------------------------------------------------------------------

/// A helper type for displaying [`Params`] configuration.
///
/// Returned by [`ParamsExt::debug`]; implements [`Debug`] to show all derived
/// configuration values.
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

/// The default table configuration with [`Capacity::DEF`] slots.
///
/// Used when creating a table without specifying a custom [`Params`] type.
///
/// # Example
///
/// ```
/// use ptab::{PTab, DefaultParams};
///
/// // These are equivalent:
/// let table1: PTab<u64> = PTab::new();
/// let table2: PTab<u64, DefaultParams> = PTab::new();
/// ```
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct DefaultParams;

impl Params for DefaultParams {
  const LENGTH: Capacity = Capacity::DEF;
}

// -----------------------------------------------------------------------------
// Const-Generic Params
// -----------------------------------------------------------------------------

/// A [`Params`] implementation with compile-time configurable capacity.
///
/// The recommended way to create tables with custom capacities. The capacity
/// `N` is rounded up to the nearest power of two and clamped to
/// <code>[Capacity::MIN]..=[Capacity::MAX]</code>.
///
/// # Examples
///
/// ```
/// use ptab::{PTab, ConstParams};
///
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
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConstParams<const N: usize>;

impl<const N: usize> Params for ConstParams<N> {
  const LENGTH: Capacity = Capacity::new(N);
}

// -----------------------------------------------------------------------------
// Capacity
// -----------------------------------------------------------------------------

/// A type storing a `usize` which is a power of two in the range
/// <code>[MIN]..=[MAX]</code>, and thus represents a possible table capacity.
///
/// # Examples
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
/// [MIN]: Self::MIN
/// [MAX]: Self::MAX
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct Capacity(CapacityEnum);

const _: () = assert!(align_of::<Capacity>() == align_of::<usize>());
const _: () = assert!(size_of::<Capacity>() == size_of::<usize>());

impl Capacity {
  /// The minimum supported capacity (2⁴ entries).
  pub const MIN: Self = Self(CapacityEnum::_Capacity1Shl4);

  /// The maximum supported capacity (2²⁷ entries).
  pub const MAX: Self = {
    #[cfg(not(any(miri, all(test, not(feature = "slow")))))]
    {
      Self(CapacityEnum::_Capacity1Shl27)
    }

    #[cfg(any(miri, all(test, not(feature = "slow"))))]
    {
      Self(CapacityEnum::_Capacity1Shl16)
    }
  };

  /// The default capacity (2²⁰ entries).
  pub const DEF: Self = {
    #[cfg(not(any(miri, all(test, not(feature = "slow")))))]
    {
      Self(CapacityEnum::_Capacity1Shl20)
    }

    #[cfg(any(miri, all(test, not(feature = "slow"))))]
    {
      Self(CapacityEnum::_Capacity1Shl10)
    }
  };

  /// Creates a `Capacity` from a `usize`.
  ///
  /// Rounds up to the nearest power of two and clamps to
  /// <code>[MIN]..=[MAX]</code>.
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
  /// [MIN]: Self::MIN
  /// [MAX]: Self::MAX
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
      // SAFETY:
      // - `capacity` was produced by `checked_next_power_of_two`, so it is a
      //   power of two.
      // - The bounds checks above ensure `Self::MIN <= capacity <= Self::MAX`.
      unsafe { Self::new_unchecked(capacity) }
    }
  }

  /// Creates a `Capacity` from a power-of-two `usize`.
  ///
  /// # Safety
  ///
  /// `capacity` must be a power of two in the range <code>[MIN]..=[MAX]</code>.
  ///
  /// [MIN]: Self::MIN
  /// [MAX]: Self::MAX
  #[inline]
  pub const unsafe fn new_unchecked(capacity: usize) -> Self {
    debug_assert!(
      Self::is_valid(capacity),
      "Capacity::new_unchecked requires a valid value",
    );

    // SAFETY:
    // - The caller guarantees that `capacity` is a power of two in the range
    //   `[Self::MIN, Self::MAX]`.
    // - `Capacity` has the same size and alignment as `usize`.
    unsafe { mem::transmute::<usize, Self>(capacity) }
  }

  /// Returns the capacity as a [`u32`].
  #[inline]
  pub const fn as_u32(self) -> u32 {
    self.0 as u32
  }

  /// Returns the capacity as a [`usize`].
  #[inline]
  pub const fn as_usize(self) -> usize {
    self.0 as usize
  }

  /// Returns the capacity as a [`NonZeroUsize`].
  #[inline]
  pub const fn as_nonzero(self) -> NonZeroUsize {
    // SAFETY:
    // - All valid `Capacity` values are non-zero powers of two.
    // - `Capacity` has the same size and alignment as `usize`.
    unsafe { mem::transmute::<Self, NonZeroUsize>(self) }
  }

  /// Returns the base-2 logarithm of the capacity.
  ///
  /// This is always exact, as `self` represents a power of two.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::Capacity;
  ///
  /// assert_eq!(Capacity::new(16).log2(), 4);
  /// assert_eq!(Capacity::new(1024).log2(), 10);
  /// ```
  #[inline]
  pub const fn log2(self) -> u32 {
    self.as_nonzero().trailing_zeros()
  }

  #[inline]
  const fn is_valid(value: usize) -> bool {
    value.is_power_of_two() && value >= Self::MIN.as_usize() && value <= Self::MAX.as_usize()
  }
}

impl Debug for Capacity {
  fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
    write!(f, "{:?} (1 << {:?})", self.as_nonzero(), self.log2())
  }
}

impl Default for Capacity {
  #[inline]
  fn default() -> Self {
    Self::DEF
  }
}

impl From<Capacity> for NonZeroUsize {
  #[inline]
  fn from(other: Capacity) -> Self {
    other.as_nonzero()
  }
}

impl From<Capacity> for usize {
  #[inline]
  fn from(other: Capacity) -> Self {
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

#[inline]
const fn derive_blocks<P>() -> NonZeroUsize
where
  P: Params + ?Sized,
{
  // Determine the minimum valid size of table arrays.
  let Some(mem_bytes) = P::LENGTH.as_usize().checked_mul(size_of::<AtomicUsize>()) else {
    panic_for_blocks();
  };

  // Round up so we have an even number of slots.
  let Some(mem_align) = mem_bytes.checked_next_multiple_of(CACHE_LINE) else {
    panic_for_blocks();
  };

  // Make sure we don't overflow `isize::MAX`, otherwise the layout is invalid.
  if mem_align > isize::MAX as usize {
    panic_for_blocks();
  }

  // Finally, compute the block count.
  let Some(blocks) = NonZeroUsize::new(mem_align / CACHE_LINE) else {
    panic_for_blocks();
  };

  blocks
}

#[inline]
const fn derive_layout<P>() -> Layout
where
  P: Params + ?Sized,
{
  assert!(
    P::MEMORY != 0,
    "derive_layout requires a non-zero table size",
  );

  // SAFETY:
  // - `P::MEMORY != 0` (asserted above).
  // - `CACHE_LINE` is a power of two, so it is a valid alignment.
  // - `P::MEMORY <= isize::MAX`, guaranteed by `derive_blocks`.
  unsafe { Layout::from_size_align_unchecked(P::MEMORY, CACHE_LINE) }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[cold]
#[cfg_attr(panic = "abort", inline)]
#[cfg_attr(not(panic = "abort"), inline(never))]
const fn panic_for_blocks() -> ! {
  panic!("invalid params: `BLOCKS` must be representable");
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(test)]
mod tests {
  use core::num::NonZeroUsize;

  use crate::params::CACHE_LINE_SLOTS;
  use crate::params::Capacity;
  use crate::params::DefaultParams;
  use crate::params::Params;
  use crate::params::ParamsExt;
  use crate::params::derive_blocks;
  use crate::params::derive_layout;
  use crate::utils::each_capacity;

  #[test]
  fn capacity_new_clamp_under_min() {
    assert_eq!(Capacity::new(1), Capacity::MIN);
  }

  #[test]
  fn capacity_new_clamp_over_max() {
    assert_eq!(Capacity::new(Capacity::MAX.as_usize() + 1), Capacity::MAX);
  }

  #[test]
  fn capacity_new_clamp_usize_max() {
    assert_eq!(Capacity::new(usize::MAX), Capacity::MAX);
  }

  #[test]
  fn capacity_new_exact() {
    assert_eq!(Capacity::new(1 << 10).as_usize(), 1 << 10);
  }

  #[test]
  fn capacity_new_round_up() {
    assert_eq!(Capacity::new((1 << 10) - 1).as_usize(), 1 << 10);
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
    assert_eq!(format!("{:?}", Capacity::MIN), "16 (1 << 4)");
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

  #[test]
  fn default_params() {
    assert_eq!(DefaultParams::LENGTH, Capacity::DEF);
  }

  #[test]
  fn debug_params() {
    let params: String = format!("{:?}", DefaultParams::debug());

    assert!(params.contains("DefaultParams"));
    assert!(params.contains("LENGTH:"));
    assert!(params.contains("BLOCKS:"));
    assert!(params.contains("LAYOUT:"));
    assert!(params.contains("MEMORY:"));
  }

  #[test]
  fn params_length_vs_memory() {
    each_capacity!({
      let expected_size: usize = P::LENGTH.as_usize() * size_of::<usize>();
      let received_size: usize = P::MEMORY;

      assert!(received_size >= expected_size);
    });
  }

  #[test]
  fn params_length_vs_blocks() {
    each_capacity!({
      let expected: usize = P::LENGTH.as_usize();
      let received: usize = P::BLOCKS.get() * CACHE_LINE_SLOTS;

      assert_eq!(expected, received);
    });
  }

  #[test]
  fn params_derive_blocks_at_runtime() {
    each_capacity!({
      assert_eq!(P::BLOCKS, derive_blocks::<P>());
    });
  }

  #[test]
  fn params_derive_layout_at_runtime() {
    each_capacity!({
      assert_eq!(P::LAYOUT, derive_layout::<P>());
    });
  }

  #[test]
  fn params_blocks_power_of_two() {
    each_capacity!({
      assert!(P::BLOCKS.is_power_of_two());
    });
  }

  #[test]
  fn id_mask_bits_composition() {
    each_capacity!({
      assert_eq!(P::ID_MASK_BITS, P::ID_SHIFT_BLOCK + P::ID_SHIFT_INDEX);
    });
  }

  #[test]
  fn id_mask_entry_composition() {
    each_capacity!({
      assert_eq!(
        P::ID_MASK_ENTRY,
        (P::ID_MASK_BLOCK << P::ID_SHIFT_BLOCK) ^ P::ID_MASK_INDEX,
      );
    });
  }

  #[test]
  fn id_mask_entry_covers_all_indices() {
    each_capacity!({
      for index in 0..P::LENGTH.as_usize() {
        assert!(index <= P::ID_MASK_ENTRY);
      }
    });
  }

  #[test]
  fn id_shift_block_composition() {
    each_capacity!({
      assert_eq!(P::ID_SHIFT_BLOCK, P::ID_MASK_INDEX.count_ones());
    });
  }

  #[test]
  fn id_shift_index_composition() {
    each_capacity!({
      assert_eq!(P::ID_SHIFT_INDEX, P::ID_MASK_BLOCK.count_ones());
    });
  }
}
