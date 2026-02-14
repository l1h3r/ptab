use core::fmt::Debug;
use core::fmt::Formatter;
use core::fmt::Result;
use core::mem::MaybeUninit;
use core::panic::RefUnwindSafe;
use core::panic::UnwindSafe;

use sdd::Guard;

use crate::index::Detached;
use crate::params::DefaultParams;
use crate::params::Params;
use crate::params::ParamsExt;
use crate::table::Table;
use crate::table::WeakKeys;

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
/// - `P`: Configuration implementing [`Params`]. Defaults to [`DefaultParams`].
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
/// [`ConstParams`]: crate::params::ConstParams
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
  /// Determined by [`Params::LENGTH`] and fixed for the lifetime of the table.
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
  /// May change immediately due to concurrent operations.
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
  /// Returns [`None`] if the table is at capacity. Use [`write`] instead when
  /// the stored value needs to know its own index.
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
  ///
  /// [`write`]: Self::write
  #[inline]
  pub fn insert(&self, value: T) -> Option<Detached>
  where
    T: 'static,
  {
    self.inner.insert(value)
  }

  /// Inserts a value using an initialization function that receives the index.
  ///
  /// Enables self-referential structures where the stored value contains its
  /// own index. Returns [`None`] if the table is at capacity.
  ///
  /// # Requirements
  ///
  /// The `init` function:
  ///
  /// - **Must** fully initialize the [`MaybeUninit<T>`] before returning
  /// - **Must not** panic (panics permanently leak a slot)
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
  ///
  /// [`MaybeUninit<T>`]: core::mem::MaybeUninit
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
  /// Returns `true` if an entry was removed, `false` if already absent. The
  /// slot becomes available for reuse immediately; memory is reclaimed via
  /// epoch-based reclamation once no readers hold references.
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
  /// May become stale immediately due to concurrent operations.
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
    self.inner.exists(index, &Guard::new())
  }

  /// Accesses an entry by index, applying a function to it.
  ///
  /// Returns [`None`] if no entry exists. The reference remains valid for the
  /// callback's duration even under concurrent removal, due to epoch-based
  /// reclamation.
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
    self.inner.with(index, &Guard::new(), f)
  }

  /// Returns a copy of the entry at the given index.
  ///
  /// Convenience method equivalent to `self.with(idx, |v| *v)`. Returns
  /// [`None`] if no entry exists.
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
    self.inner.read(index, &Guard::new())
  }

  /// Returns a weakly consistent iterator over all currently allocated indices.
  ///
  /// # Semantics
  ///
  /// This iterator provides **weak snapshot semantics**:
  ///
  /// - Entries inserted after iteration begins **may or may not** be observed.
  /// - Entries removed during iteration **may or may not** be observed.
  /// - Each yielded index was valid at some instant during iteration.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<i32> = PTab::new();
  /// let a = table.insert(1).unwrap();
  /// let b = table.insert(2).unwrap();
  ///
  /// let mut seen = Vec::new();
  /// for key in table.weak_keys() {
  ///   seen.push(key);
  /// }
  ///
  /// assert!(seen.contains(&a));
  /// assert!(seen.contains(&b));
  /// ```
  #[inline]
  pub fn weak_keys(&self) -> WeakKeys<'_, T, P> {
    self.inner.weak_keys()
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

// SAFETY: Internal state uses atomics and epoch-based reclamation; sharing
// across threads is safe when `T` can be sent between threads.
unsafe impl<T, P> Send for PTab<T, P>
where
  T: Send,
  P: Params + ?Sized,
{
}

// SAFETY: Concurrent access is mediated through atomics. `T: Sync` is not
// required because `with` only provides shared references.
unsafe impl<T, P> Sync for PTab<T, P>
where
  T: Send,
  P: Params + ?Sized,
{
}

// Unconditional because `PTab` provides only shared access to `T` via `with`,
// and epoch-based reclamation handles panic unwind safely.
impl<T, P> RefUnwindSafe for PTab<T, P> where P: Params + ?Sized {}
impl<T, P> UnwindSafe for PTab<T, P> where P: Params + ?Sized {}
