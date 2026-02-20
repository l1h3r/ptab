mod traits;

pub mod collector;

pub use self::traits::Atomic;
pub use self::traits::Collector;
pub use self::traits::CollectorWeak;
pub use self::traits::Shared;

// -----------------------------------------------------------------------------
// Sanity Check
// -----------------------------------------------------------------------------

const _: () = <collector::Leak as CollectorWeak>::ASSERT_ATOMIC;

#[cfg(feature = "sdd")]
const _: () = <collector::Sdd as CollectorWeak>::ASSERT_ATOMIC;
