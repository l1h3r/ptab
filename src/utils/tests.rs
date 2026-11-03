use core::hash::Hash;
use core::hash::Hasher;

use crate::params::Capacity;
use crate::params::ConstParams;
use crate::params::DefaultParams;
use crate::params::Params;

// -----------------------------------------------------------------------------
// Params
// -----------------------------------------------------------------------------

#[allow(dead_code, reason = "conditional")]
pub(crate) type DefParams = ConstParams<{ Capacity::DEF.as_usize() }>;

#[allow(dead_code, reason = "conditional")]
pub(crate) type MaxParams = ConstParams<{ Capacity::MAX.as_usize() }>;

#[allow(dead_code, reason = "conditional")]
pub(crate) type MinParams = ConstParams<{ Capacity::MIN.as_usize() }>;

pub(crate) struct NoLength;

impl Params for NoLength {
  type Collector = <DefaultParams as Params>::Collector;
}

// -----------------------------------------------------------------------------
// Hash
// -----------------------------------------------------------------------------

#[cfg(test)]
struct Fnv(u64);

#[cfg(test)]
impl Hasher for Fnv {
  fn finish(&self) -> u64 {
    self.0
  }

  fn write(&mut self, bytes: &[u8]) {
    for byte in bytes {
      self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
  }
}

#[cfg(test)]
pub(crate) fn hash<T: Hash>(value: T) -> u64 {
  let mut hasher: Fnv = Fnv(0xcbf2_9ce4_8422_2325);
  value.hash(&mut hasher);
  hasher.finish()
}

// -----------------------------------------------------------------------------
// Kani
// -----------------------------------------------------------------------------

#[cfg(kani)]
pub(crate) fn any_capacity() -> Capacity {
  Capacity::new(::kani::any())
}

#[cfg(kani)]
pub(crate) fn any_valid_capacity() -> usize {
  ::kani::any_where(|value: &usize| Capacity::is_valid(*value))
}

#[cfg(kani)]
pub(crate) fn any_index<P>() -> usize
where
  P: Params,
{
  ::kani::any_where(|index: &usize| *index < P::LENGTH.as_usize())
}

// -----------------------------------------------------------------------------
// Macros
// -----------------------------------------------------------------------------

#[cfg(all(test, not(kani)))]
macro_rules! each_capacity {
  ($expr:expr) => {
    $crate::utils::each_capacity!(
      @each $expr;
      $crate::params::Capacity::DEF.log2(),
      $crate::params::Capacity::MAX.log2(),
      $crate::params::Capacity::MIN.log2(),
    );
  };
  (@each $expr:expr; $($bits:expr),+ $(,)?) => {
    $({
      const S: usize = 1 << $bits;
      #[allow(dead_code, reason = "not every body names `P`")]
      type P = $crate::params::ConstParams<{ S }>;
      $expr
    })+
  };
}

#[cfg(kani)]
macro_rules! each_capacity {
  ($expr:expr) => {
    $crate::utils::each_capacity!(
      @each $expr;
      10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    );
  };
  (@each $expr:expr; $($bits:literal),+ $(,)?) => {
    $({
      const S: usize = 1 << $bits;
      type P = $crate::params::ConstParams<{ S }>;
      ::kani::assert(
        <P as $crate::params::Params>::LENGTH.as_usize() == S,
        concat!("LENGTH is 1 << ", stringify!($bits)),
      );
      ::kani::cover(true, concat!("capacity 1 << ", stringify!($bits)));
      $expr
    })+
  };
}

pub(crate) use each_capacity;
