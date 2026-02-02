use core::fmt::Debug;
use core::fmt::Formatter;
use core::fmt::Result;
use core::mem::MaybeUninit;
use core::panic::RefUnwindSafe;
use core::panic::UnwindSafe;

use crate::index::Detached;
use crate::params::DefaultParams;
use crate::params::Params;
use crate::params::ParamsExt;
use crate::table::Table;

/// A lock-free concurrent table.
///
/// `PTab` stores entries of type `T` and provides concurrent access through
/// opaque [`Detached`] indices. It is parameterized by `P` to configure
/// capacity at compile time.
///
/// See the [crate-level documentation][crate] for an overview and examples.
///
/// # Type Parameters
///
/// - `T`: The type of values stored in the table.
/// - `P`: Configuration parameters implementing [`Params`]. Defaults to
///   [`DefaultParams`] (1,048,576 slots).
///
/// # Examples
///
/// Basic usage with default configuration:
///
/// ```
/// use ptab::PTab;
///
/// let table: PTab<i32> = PTab::new();
///
/// let idx = table.insert(42).unwrap();
/// assert_eq!(table.with(idx, |&v| v), Some(42));
/// ```
///
/// Custom capacity using [`ConstParams`]:
///
/// ```
/// use ptab::{PTab, ConstParams};
///
/// let table: PTab<i32, ConstParams<256>> = PTab::new();
/// assert_eq!(table.capacity(), 256);
/// ```
///
/// [`ConstParams`]: crate::ConstParams
#[repr(transparent)]
pub struct PTab<T, P = DefaultParams>
where
  P: Params + ?Sized,
{
  inner: Table<T, P>,
}

impl<T, P> PTab<T, P>
where
  P: Params + ?Sized,
{
  /// Creates a new, empty table.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<String> = PTab::new();
  /// assert!(table.is_empty());
  /// ```
  #[inline]
  pub fn new() -> Self {
    Self {
      inner: Table::new(),
    }
  }

  /// Returns the maximum number of entries the table can hold.
  ///
  /// This value is determined by the [`Params::LENGTH`] configuration and
  /// is fixed for the lifetime of the table.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::{PTab, ConstParams};
  ///
  /// let table: PTab<u64, ConstParams<512>> = PTab::new();
  /// assert_eq!(table.capacity(), 512);
  /// ```
  #[inline]
  pub const fn capacity(&self) -> usize {
    self.inner.cap()
  }

  /// Returns the number of entries currently in the table.
  ///
  /// This value may change immediately after reading due to concurrent
  /// operations in other threads.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<i32> = PTab::new();
  /// assert_eq!(table.len(), 0);
  ///
  /// table.insert(1);
  /// table.insert(2);
  /// assert_eq!(table.len(), 2);
  /// ```
  #[inline]
  pub fn len(&self) -> usize {
    self.inner.len() as usize
  }

  /// Returns `true` if the table contains no entries.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<i32> = PTab::new();
  /// assert!(table.is_empty());
  ///
  /// table.insert(42);
  /// assert!(!table.is_empty());
  /// ```
  #[inline]
  pub fn is_empty(&self) -> bool {
    self.inner.is_empty()
  }

  /// Inserts a value into the table and returns its index.
  ///
  /// Returns `None` if the table is at capacity.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<&str> = PTab::new();
  ///
  /// let idx = table.insert("hello").unwrap();
  /// assert!(table.exists(idx));
  /// ```
  #[inline]
  pub fn insert(&self, value: T) -> Option<Detached>
  where
    T: 'static,
  {
    self.inner.insert(value)
  }

  /// Inserts a value into the table using an initialization function.
  ///
  /// The `init` function receives an uninitialized slot and the index that
  /// will identify the entry. This allows the stored value to contain its
  /// own index, which is useful for self-referential data structures.
  ///
  /// Returns `None` if the table is at capacity.
  ///
  /// # Requirements
  ///
  /// The `init` function:
  /// - **Must** fully initialize the [`MaybeUninit<T>`] before returning
  /// - **Must not** panic (panics will permanently leak a slot)
  /// - **Should** avoid recursive table operations
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::{PTab, Detached};
  ///
  /// struct Process {
  ///   id: Detached,
  ///   data: u64,
  /// }
  ///
  /// let table: PTab<Process> = PTab::new();
  ///
  /// let idx = table.write(|slot, id| {
  ///   slot.write(Process { id, data: 42 });
  /// }).unwrap();
  ///
  /// // The stored process knows its own index
  /// table.with(idx, |proc| assert_eq!(proc.id, idx));
  /// ```
  #[inline]
  pub fn write<F>(&self, init: F) -> Option<Detached>
  where
    T: 'static,
    F: FnOnce(&mut MaybeUninit<T>, Detached),
  {
    self.inner.write(init)
  }

  /// Removes the entry at the given index.
  ///
  /// Returns `true` if an entry was present and removed, `false` otherwise.
  ///
  /// The slot becomes available for reuse after removal. Memory is reclaimed
  /// once all concurrent readers have finished accessing the entry.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<i32> = PTab::new();
  /// let idx = table.insert(42).unwrap();
  ///
  /// assert!(table.remove(idx));  // Entry removed
  /// assert!(!table.remove(idx)); // Already gone
  /// ```
  #[inline]
  pub fn remove(&self, index: Detached) -> bool {
    self.inner.remove(index)
  }

  /// Returns `true` if an entry exists at the given index.
  ///
  /// The result may become stale immediately due to concurrent operations.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<i32> = PTab::new();
  /// let idx = table.insert(42).unwrap();
  ///
  /// assert!(table.exists(idx));
  /// table.remove(idx);
  /// assert!(!table.exists(idx));
  /// ```
  #[inline]
  pub fn exists(&self, index: Detached) -> bool {
    self.inner.exists(index)
  }

  /// Accesses an entry by index, applying a function to it.
  ///
  /// Returns `None` if no entry exists at the given index.
  ///
  /// The callback receives a reference that is guaranteed to remain valid
  /// for its duration, even if another thread removes the entry concurrently.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<String> = PTab::new();
  /// let idx = table.insert("hello".to_string()).unwrap();
  ///
  /// let len = table.with(idx, |s| s.len());
  /// assert_eq!(len, Some(5));
  /// ```
  #[inline]
  pub fn with<F, R>(&self, index: Detached, f: F) -> Option<R>
  where
    F: Fn(&T) -> R,
  {
    self.inner.with(index, f)
  }

  /// Returns a copy of the entry at the given index.
  ///
  /// This is a convenience method equivalent to `table.with(idx, |v| *v)`.
  ///
  /// Returns `None` if no entry exists at the given index.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<i32> = PTab::new();
  /// let idx = table.insert(42).unwrap();
  ///
  /// assert_eq!(table.read(idx), Some(42));
  /// ```
  #[inline]
  pub fn read(&self, index: Detached) -> Option<T>
  where
    T: Copy,
  {
    self.inner.read(index)
  }
}

impl<T, P> Debug for PTab<T, P>
where
  T: Debug,
  P: Params + ?Sized,
{
  fn fmt(&self, f: &mut Formatter<'_>) -> Result {
    f.debug_struct("PTab")
      .field("params", &P::debug())
      .field("entries", &self.inner)
      .finish()
  }
}

impl<T, P> Default for PTab<T, P>
where
  P: Params + ?Sized,
{
  #[inline]
  fn default() -> Self {
    Self::new()
  }
}

// SAFETY: All internal state uses atomic operations and epoch-based
// reclamation, so `PTab` is `Send` when `T` is `Send`.
unsafe impl<T, P> Send for PTab<T, P>
where
  T: Send,
  P: Params + ?Sized,
{
}

// SAFETY: Concurrent access is mediated through atomic operations. `T: Sync`
// is not required because `with` borrows the table, preventing simultaneous
// mutable access to the same entry.
unsafe impl<T, P> Sync for PTab<T, P>
where
  T: Send,
  P: Params + ?Sized,
{
}

// These impls are intentionally unconditional on `T` because:
// 1. PTab only provides shared access to T (via `with`)
// 2. Panic unwind cannot observe partially-modified T values
// 3. The epoch-based reclamation is unwind-safe
impl<T, P> RefUnwindSafe for PTab<T, P> where P: Params + ?Sized {}
impl<T, P> UnwindSafe for PTab<T, P> where P: Params + ?Sized {}
