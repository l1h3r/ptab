mod traits;

pub mod collector;

pub use self::traits::Atomic;
pub use self::traits::Collector;
pub use self::traits::CollectorWeak;
pub use self::traits::Shared;

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(test)]
mod tests {
  use crate::reclaim::CollectorWeak;
  use crate::reclaim::collector;
  use crate::sync::atomic::AtomicUsize;

  #[test]
  fn leak_atomic_is_usize_sized() {
    assert_eq!(
      size_of::<<collector::Leak as CollectorWeak>::Atomic<u64>>(),
      size_of::<AtomicUsize>(),
    );
    assert_eq!(
      size_of::<<collector::Leak as CollectorWeak>::Atomic<[u8; 64]>>(),
      size_of::<AtomicUsize>(),
    );
  }

  #[cfg(feature = "sdd")]
  #[test]
  fn sdd_atomic_is_usize_sized() {
    assert_eq!(
      size_of::<<collector::Sdd as CollectorWeak>::Atomic<u64>>(),
      size_of::<AtomicUsize>(),
    );
    assert_eq!(
      size_of::<<collector::Sdd as CollectorWeak>::Atomic<[u8; 64]>>(),
      size_of::<AtomicUsize>(),
    );
  }
}
