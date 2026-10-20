//! A lock-free, cache-line-aware concurrent table.
//!
//! [`PTab`] is a fixed-capacity table. Entries are stored and accessed by opaque
//! indices. It targets read-heavy workloads with lookups from many threads.
//!
//! # Overview
//!
//! The table assigns each inserted value a [`Detached`] index, which looks up
//! or removes the entry.
//!
//! Lookup and removal use only the slot bits of an index. A slot is reused after
//! removal, so an index kept past its entry's removal reaches whatever entry
//! occupies that slot later.
//!
//! # Usage
//!
//! ```
//! # #[cfg(feature = "sdd")]
//! # fn main() {
//! use ptab::PTab;
//!
//! // Create a table with default capacity
//! let table: PTab<String> = PTab::new();
//!
//! // Insert an entry and get its index
//! let index = table.insert("hello".to_string()).unwrap();
//!
//! // Access the entry by index
//! let greeting = table.with(index, |s| s.to_uppercase());
//! assert_eq!(greeting, Some("HELLO".to_string()));
//!
//! // Remove the entry
//! assert!(table.remove(index));
//!
//! // The entry is gone
//! assert!(!table.exists(index));
//! # }
//! # #[cfg(not(feature = "sdd"))]
//! # fn main() {}
//! ```
//!
//! # Configuration
//!
//! Capacity is set at compile time through the [`Params`] trait.
//! [`DefaultParams`] provides [`Capacity::DEF`] slots:
//!
//! ```
//! use ptab::{PTab, DefaultParams};
//!
//! // These are equivalent:
//! let table1: PTab<u64> = PTab::new();
//! let table2: PTab<u64, DefaultParams> = PTab::new();
//! ```
//!
//! [`ConstParams`] sets a custom capacity:
//!
//! ```
//! use ptab::{PTab, ConstParams};
//!
//! let table: PTab<u64, ConstParams<512>> = PTab::new();
//! assert_eq!(table.capacity(), 512);
//! ```
//!
//! Capacity rounds up to a power of two and clamps to
//! <code>[Capacity::MIN]..=[Capacity::MAX]</code>.
//!
//! # Concurrency
//!
//! All operations on [`PTab`] are thread-safe and lock-free.
//!
//! ```no_run
//! # #[cfg(feature = "sdd")]
//! # fn main() {
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
//! # }
//! # #[cfg(not(feature = "sdd"))]
//! # fn main() {}
//! ```
//!
//! ## Memory Reclamation
//!
//! Removed entries are reclaimed by the collector that `Params::Collector`
//! selects, deferred while any reader may still hold a reference. With the
//! `sdd` feature (the default) `DefaultParams` uses a collector that
//! eventually reclaims every removed entry. Without it the default is a leaking
//! collector, which never reclaims and does not support `remove`.
//!
//! See [`memory`] for reclamation trait details and custom implementations.
//!
//! # Memory Layout
//!
//! Consecutive allocations land in different cache lines, so threads that insert
//! together do not share a line. See [`CACHE_LINE_SLOTS`] for the slots per
//! line and [`implementation`] for the index layout.
//!
//! # Capacity Limits
//!
//! Capacity is bounded by [`Capacity::MIN`] and [`Capacity::MAX`]. The default
//! is [`Capacity::DEF`]. When full, [`PTab::insert()`] returns [`None`]. At
//! [`Capacity::MAX`] one slot is reserved, so the table holds one fewer entry.
//!
//! [Capacity::MAX]: crate::params::Capacity::MAX
//! [Capacity::MIN]: crate::params::Capacity::MIN
//! [`CACHE_LINE_SLOTS`]: crate::params::CACHE_LINE_SLOTS
//! [`Capacity::DEF`]: crate::params::Capacity::DEF
//! [`Capacity::MAX`]: crate::params::Capacity::MAX
//! [`Capacity::MIN`]: crate::params::Capacity::MIN
//! [`ConstParams`]: crate::params::ConstParams
//! [`DefaultParams`]: crate::params::DefaultParams
//! [`Params`]: crate::params::Params
//! [`PTab::insert()`]: crate::public::PTab::insert
//! [`memory`]: crate::memory
//! [`implementation`]: crate::implementation

#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]
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
  #![allow(
    clippy::doc_include_without_cfg,
    reason = "the module needs its docs in every build, not only rustdoc"
  )]
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
  //! Memory reclamation traits and built-in strategies.
  //!
  //! This module exposes the internal reclamation API used by `ptab`. You do
  //! not need to interact with these traits directly unless implementing
  //! [`Params`] or your own custom collector.
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
