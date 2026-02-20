//! A lock-free, cache-line-aware concurrent table.
//!
//! `ptab` provides [`PTab`], a fixed-capacity table for storing entries and
//! accessing them by opaque indices. It is optimized for read-heavy workloads
//! where entries are looked up frequently from multiple threads simultaneously.
//!
//! # Overview
//!
//! The table assigns each inserted value a [`Detached`] index that uniquely
//! identifies that entry. This index can be used to look up, access, or remove
//! the entry. Indices include a generational component to mitigate the
//! [ABA problem] in concurrent algorithms.
//!
//! # Usage
//!
//! ```
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
//! ```
//!
//! # Configuration
//!
//! Table capacity is configured at compile time through the [`Params`] trait.
//! The default configuration ([`DefaultParams`]) provides [`Capacity::DEF`]
//! slots:
//!
//! ```
//! use ptab::{PTab, DefaultParams};
//!
//! // These are equivalent:
//! let table1: PTab<u64> = PTab::new();
//! let table2: PTab<u64, DefaultParams> = PTab::new();
//! ```
//!
//! For custom capacities, use [`ConstParams`]:
//!
//! ```
//! use ptab::{PTab, ConstParams};
//!
//! let table: PTab<u64, ConstParams<512>> = PTab::new();
//! assert_eq!(table.capacity(), 512);
//! ```
//!
//! Capacity is always rounded up to the nearest power of two and clamped
//! to the range <code>[Capacity::MIN]..=[Capacity::MAX]</code>.
//!
//! # Concurrency
//!
//! All operations on [`PTab`] are thread-safe and lock-free. Multiple threads
//! can concurrently insert, remove, and access entries without blocking.
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
//! Removed entries are reclaimed using epoch-based memory management via
//! [`sdd`]. This ensures concurrent readers can safely access entries even
//! while other threads are removing them.
//!
//! # Memory Layout
//!
//! The table uses a cache-line-aware memory layout to minimize false sharing
//! between threads. Consecutive allocations are distributed across different
//! cache lines, reducing contention when multiple threads operate on
//! recently-allocated entries. See [`CACHE_LINE_SLOTS`] for the distribution
//! stride.
//!
//! # Capacity Limits
//!
//! Capacity is bounded by [`Capacity::MIN`] and [`Capacity::MAX`]. The default
//! is [`Capacity::DEF`]. When full, [`PTab::insert()`] returns [`None`].
//!
//! [Capacity::MAX]: crate::config::Capacity::MAX
//! [Capacity::MIN]: crate::config::Capacity::MIN
//! [`CACHE_LINE_SLOTS`]: crate::config::CACHE_LINE_SLOTS
//! [`Capacity::DEF`]: crate::config::Capacity::DEF
//! [`Capacity::MAX`]: crate::config::Capacity::MAX
//! [`Capacity::MIN`]: crate::config::Capacity::MIN
//! [`ConstParams`]: crate::config::ConstParams
//! [`DefaultParams`]: crate::config::DefaultParams
//! [`Params`]: crate::config::Params
//! [`PTab::insert()`]: crate::public::PTab::insert
//!
//! [ABA problem]: https://en.wikipedia.org/wiki/ABA_problem
//! [`sdd`]: https://docs.rs/sdd
//!

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

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
  //! Configuration parameters which can be used to override the default table
  //! settings.

  pub use crate::params::CACHE_LINE;
  pub use crate::params::CACHE_LINE_SLOTS;
  pub use crate::params::Capacity;
  pub use crate::params::ConstParams;
  pub use crate::params::DebugParams;
  pub use crate::params::DefaultParams;
  pub use crate::params::Params;
  pub use crate::params::ParamsExt;
}

pub mod garbage {
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
