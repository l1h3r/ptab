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
//! let table: PTab<u64, ConstParams<4096>> = PTab::new();
//! assert_eq!(table.capacity(), 4096);
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
//! is [`Capacity::DEF`]. When full, [`PTab::insert`] returns [`None`].
//!
//! [ABA problem]: https://en.wikipedia.org/wiki/ABA_problem
//! [`sdd`]: https://docs.rs/sdd
//!

mod array;
mod index;
mod padded;
mod params;
mod public;
mod table;

#[cfg(all(test, not(any(loom, shuttle))))]
mod tests;

pub mod implementation {
  #![doc = include_str!("../IMPLEMENTATION.md")]
}

pub use self::index::Detached;
pub use self::params::CACHE_LINE;
pub use self::params::CACHE_LINE_SLOTS;
pub use self::params::Capacity;
pub use self::params::ConstParams;
pub use self::params::DebugParams;
pub use self::params::DefaultParams;
pub use self::params::Params;
pub use self::params::ParamsExt;
pub use self::public::PTab;
pub use self::table::WeakKeys;

mod alloc {
  #[cfg(loom)]
  mod exports {
    pub(crate) use ::loom::alloc::alloc;
    pub(crate) use ::loom::alloc::dealloc;
    pub(crate) use ::std::alloc::handle_alloc_error;
  }

  #[cfg(not(loom))]
  mod exports {
    pub(crate) use ::std::alloc::alloc;
    pub(crate) use ::std::alloc::dealloc;
    pub(crate) use ::std::alloc::handle_alloc_error;
  }

  pub(crate) use self::exports::*;
}

mod sync {
  #[cfg(all(loom, shuttle))]
  compile_error!("cannot use loom and shuttle at once");

  #[cfg(not(any(loom, shuttle)))]
  mod exports {
    pub(crate) mod atomic {
      pub(crate) use ::core::sync::atomic::AtomicU32;
      pub(crate) use ::core::sync::atomic::AtomicUsize;
      pub(crate) use ::core::sync::atomic::Ordering;
    }
  }

  #[cfg(loom)]
  mod exports {
    pub(crate) mod atomic {
      pub(crate) use ::loom::sync::atomic::AtomicU32;
      pub(crate) use ::loom::sync::atomic::AtomicUsize;
      pub(crate) use ::loom::sync::atomic::Ordering;
    }
  }

  #[cfg(shuttle)]
  mod exports {
    pub(crate) mod atomic {
      #[repr(transparent)]
      pub(crate) struct AtomicUsize {
        inner: Box<::shuttle::sync::atomic::AtomicUsize>,
      }

      impl AtomicUsize {
        #[inline]
        pub(crate) fn new(value: usize) -> Self {
          Self {
            inner: Box::new(::shuttle::sync::atomic::AtomicUsize::new(value)),
          }
        }
      }

      impl ::core::ops::Deref for AtomicUsize {
        type Target = ::shuttle::sync::atomic::AtomicUsize;

        #[inline]
        fn deref(&self) -> &Self::Target {
          &self.inner
        }
      }

      pub(crate) use ::shuttle::sync::atomic::AtomicU32;
      pub(crate) use ::shuttle::sync::atomic::Ordering;
    }
  }

  pub(crate) use self::exports::*;
}
