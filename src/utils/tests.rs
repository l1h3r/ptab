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
// Macros
// -----------------------------------------------------------------------------

#[cfg(test)]
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

pub(crate) use each_capacity;
