//! A lock-free, cache-line-aware concurrent table.
//!
//! [`PTab`] is a fixed-capacity table that stores entries under opaque indices.
//!
//! # Overview
//!
//! Each inserted value gets a generational [`Detached`] index.
//!
//! # Usage
//!
//! ```
//! use ptab::PTab;
//!
//! let table: PTab<String> = PTab::new();
//! let index = table.insert("hello".to_string()).unwrap();
//!
//! let greeting = table.with(index, |s| s.to_uppercase());
//! assert_eq!(greeting, Some("HELLO".to_string()));
//!
//! # #[cfg(feature = "sdd")]
//! # {
//! assert!(table.remove(index));
//! assert!(!table.exists(index));
//! # }
//! ```
//!
//! # Configuration
//!
//! [`Params`] fixes the capacity and collector at compile time.
//! [`ConstParams`] sets the capacity, rounded by [`Capacity::new`]:
//!
//! ```
//! use ptab::{PTab, ConstParams};
//!
//! let table: PTab<u64, ConstParams<512>> = PTab::new();
//! assert_eq!(table.capacity(), 512);
//! ```
//!
//! # Concurrency
//!
//! All [`PTab`] operations are lock-free.
//!
//! ```no_run
//! use ptab::{PTab, ConstParams};
//! use std::sync::Arc;
//! use std::thread;
//!
//! let table: Arc<PTab<u64, ConstParams<1024>>> = Arc::new(PTab::new());
//!
//! let handles: Vec<_> = (0..4)
//!   .map(|thread_id| {
//!     let table = Arc::clone(&table);
//!     thread::spawn(move || {
//!       for i in 0..100 {
//!         if let Some(idx) = table.insert(thread_id * 1000 + i) {
//!           # #[cfg(feature = "sdd")]
//!           table.remove(idx);
//!         }
//!       }
//!     })
//!   })
//!   .collect();
//!
//! for handle in handles {
//!   handle.join().unwrap();
//! }
//! ```
//!
//! ## Memory Reclamation
//!
//! The [`Params`] collector handles removed entries. See [`DefaultParams`] for
//! the default and [`memory`] for the traits.
//!
//! # Memory Layout
//!
//! Consecutive inserts land on different cache lines when the table spans more
//! than one cache line. See [`implementation`].
//!
//! [`Capacity::new`]: crate::params::Capacity::new
//! [`ConstParams`]: crate::params::ConstParams
//! [`DefaultParams`]: crate::params::DefaultParams
//! [`Params`]: crate::params::Params
//! [`implementation`]: crate::implementation
//! [`memory`]: crate::memory

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(all(feature = "alloc", not(feature = "std")))]
extern crate alloc as rust_alloc;

#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!("expected either `std` or `alloc` feature to be enabled");

mod array;
mod index;
mod padded;
mod params;
mod public;
mod reclaim;
mod table;
mod utils;

pub(crate) use crate::utils::alloc;
pub(crate) use crate::utils::sync;

pub mod implementation {
  #![doc = include_str!("../IMPLEMENTATION.md")]
}

pub mod config {
  //! Compile-time table configuration.

  pub use crate::params::CACHE_LINE;
  pub use crate::params::CACHE_LINE_SLOTS;
  pub use crate::params::Capacity;
  pub use crate::params::ConstParams;
  pub use crate::params::DebugParams;
  pub use crate::params::DefaultParams;
  pub use crate::params::Params;
  pub use crate::params::ParamsExt;
}

pub mod memory {
  //! The collector traits and built-in collectors.
  //!
  //! Needed only to implement [`Params`] or a collector.
  //!
  //! [`Params`]: crate::params::Params

  pub use crate::reclaim::Atomic;
  pub use crate::reclaim::Collector;
  pub use crate::reclaim::CollectorWeak;
  pub use crate::reclaim::Shared;
  pub use crate::reclaim::collector;
}

#[doc(inline)]
pub use self::config::Capacity;

#[doc(inline)]
pub use self::config::ConstParams;

#[doc(inline)]
pub use self::config::DefaultParams;

#[doc(inline)]
pub use self::config::Params;

pub use self::index::Detached;

pub use self::public::PTab;
pub use self::public::WeakKeys;
