//! Built-in memory reclamation strategies.

mod leak;

pub use self::leak::Leak;

#[cfg(feature = "sdd")]
mod sdd;

#[cfg(feature = "sdd")]
pub use self::sdd::Sdd;
