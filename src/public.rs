use core::fmt::Debug;
use core::fmt::Formatter;
use core::fmt::Result;
use core::mem::MaybeUninit;

use crate::index::Detached;
use crate::params::DefaultParams;
use crate::params::Params;
use crate::params::ParamsExt;
use crate::reclaim::Collector;
use crate::table::Table;

pub use crate::table::WeakKeys;

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(doctest)]
mod tests;

// -----------------------------------------------------------------------------
// Implementation
// -----------------------------------------------------------------------------

/// A lock-free concurrent table.
///
/// See the [crate-level documentation] for an overview.
///
/// # Examples
///
/// ```
/// use ptab::PTab;
///
/// let table: PTab<i32> = PTab::new();
///
/// let index = table.insert(42).unwrap();
/// assert_eq!(table.with(index, |&v| v), Some(42));
/// ```
///
/// [crate-level documentation]: crate
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

  /// Returns the number of slots in the table.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::{PTab, ConstParams};
  ///
  /// let table: PTab<u64, ConstParams<1024>> = PTab::new();
  /// assert_eq!(table.capacity(), 1024);
  /// ```
  #[inline]
  pub const fn capacity(&self) -> usize {
    self.inner.cap()
  }

  /// Returns the number of entries in the table.
  ///
  /// The result may be stale on return.
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
  /// Returns [`None`] if the table is full. See [`write()`] to build the value
  /// from its index.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<&str> = PTab::new();
  ///
  /// let index = table.insert("hello").unwrap();
  /// assert!(table.exists(index));
  /// ```
  ///
  /// [`write()`]: Self::write
  #[inline]
  pub fn insert(&self, value: T) -> Option<Detached>
  where
    T: 'static,
  {
    self.inner.insert(value)
  }

  /// Inserts the value `init` writes, passing it the new index.
  ///
  /// Returns [`None`] if the table is full. `init` must initialize the
  /// [`MaybeUninit<T>`]. A panic in `init` leaks the slot.
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
  /// let index = table.write(|slot, id| {
  ///   slot.write(Process { id, data: 42 });
  /// }).unwrap();
  ///
  /// assert_eq!(table.with(index, |process| process.id), Some(index));
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

  /// Removes the entry at `index`.
  ///
  /// Returns `true` if it removed an entry, `false` if the slot was already
  /// empty. A later insert may reuse the slot at once. The value drops once
  /// every guard that can see it has dropped.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<i32> = PTab::new();
  /// let index = table.insert(42).unwrap();
  ///
  /// # #[cfg(feature = "sdd")]
  /// # {
  /// assert!(table.remove(index));
  /// assert!(!table.remove(index));
  /// # }
  /// ```
  #[inline]
  pub fn remove(&self, index: Detached) -> bool
  where
    T: Send,
    P::Collector: Collector,
  {
    self.inner.remove(index)
  }

  /// Returns `true` if an entry exists at `index`.
  ///
  /// The result may be stale on return.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<i32> = PTab::new();
  /// let index = table.insert(42).unwrap();
  ///
  /// assert!(table.exists(index));
  /// # #[cfg(feature = "sdd")]
  /// # {
  /// assert!(table.remove(index));
  /// assert!(!table.exists(index));
  /// # }
  /// ```
  #[inline]
  pub fn exists(&self, index: Detached) -> bool {
    self.inner.exists(index, &P::guard())
  }

  /// Accesses an entry by index, applying a function to it.
  ///
  /// Returns [`None`] if no entry exists. A concurrent [`remove()`] defers the
  /// drop until `f` returns.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<String> = PTab::new();
  /// let index = table.insert("hello".to_string()).unwrap();
  ///
  /// let len = table.with(index, |s| s.len());
  /// assert_eq!(len, Some(5));
  /// ```
  ///
  /// [`remove()`]: Self::remove
  #[inline]
  pub fn with<F, R>(&self, index: Detached, f: F) -> Option<R>
  where
    F: FnOnce(&T) -> R,
  {
    self.inner.with(index, &P::guard(), f)
  }

  /// Returns a copy of the entry at `index`.
  ///
  /// The same as `self.with(index, |v| *v)`.
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<i32> = PTab::new();
  /// let index = table.insert(42).unwrap();
  ///
  /// assert_eq!(table.read(index), Some(42));
  /// ```
  #[inline]
  pub fn read(&self, index: Detached) -> Option<T>
  where
    T: Copy,
  {
    self.inner.read(index, &P::guard())
  }

  /// Returns an iterator over the indices of present entries.
  ///
  /// See [`WeakKeys`] for consistency.
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
  ///
  /// [`WeakKeys`]: crate::table::WeakKeys
  #[inline]
  pub fn weak_keys(&self) -> WeakKeys<'_, T, P> {
    self.inner.weak_keys(P::guard())
  }
}

/// # Examples
///
/// ```
/// use ptab::PTab;
///
/// let table: PTab<i32> = PTab::new();
/// let index = table.insert(42).unwrap();
///
/// assert!(format!("{table:?}").contains(&format!("{index:?}: 42")));
/// ```
impl<T, P> Debug for PTab<T, P>
where
  T: Debug,
  P: Params + ?Sized,
{
  fn fmt(&self, f: &mut Formatter<'_>) -> Result {
    f.debug_struct("PTab")
      .field("values", &self.inner)
      .field("params", &P::debug())
      .finish()
  }
}

/// # Examples
///
/// ```
/// use ptab::PTab;
///
/// let table: PTab<i32> = PTab::default();
/// assert!(table.is_empty());
/// ```
impl<T, P> Default for PTab<T, P>
where
  P: Params + ?Sized,
{
  #[inline]
  fn default() -> Self {
    Self::new()
  }
}
