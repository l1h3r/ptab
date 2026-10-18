use core::fmt::Debug;
use core::fmt::Formatter;
use core::fmt::Result;
use core::mem::MaybeUninit;

use crate::index::Detached;
use crate::params::DefaultParams;
use crate::params::Params;
use crate::params::ParamsExt as _;
use crate::reclaim::Collector;
use crate::table::Table;

pub use crate::table::WeakKeys;

/// A lock-free concurrent table.
///
/// Entries of type `T` are accessed through opaque [`Detached`] indices. `P`
/// sets the capacity at compile time.
///
/// See the [crate-level documentation] for an overview and examples.
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
/// Sharing the table across threads moves values between them, so `PTab` is
/// `Sync` only when `T` is both `Send` and `Sync`. A `MutexGuard` is `Sync` but
/// not `Send`, so it can't be removed from another thread:
///
/// ```compile_fail,E0277
/// use std::sync::Mutex;
/// use std::sync::MutexGuard;
/// use std::thread;
///
/// use ptab::ConstParams;
/// use ptab::PTab;
///
/// static M: Mutex<u32> = Mutex::new(0);
///
/// let table: PTab<MutexGuard<'static, u32>, ConstParams<16>> = PTab::new();
/// let idx = table.insert(M.lock().unwrap()).unwrap();
///
/// thread::scope(|s| {
///   s.spawn(|| {
///     table.remove(idx);
///   });
/// });
/// ```
///
/// [crate-level documentation]: crate
/// [`Detached`]: crate::index::Detached
/// [`ConstParams`]: crate::params::ConstParams
/// [`DefaultParams`]: crate::params::DefaultParams
/// [`Params`]: crate::params::Params
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

  /// Returns the number of entries the table can hold.
  ///
  /// Set by [`Params::LENGTH`] and fixed for the lifetime of the table. At
  /// [`Capacity::MAX`] one slot is reserved, so this is one less than
  /// [`Params::LENGTH`].
  ///
  /// # Examples
  ///
  /// ```
  /// use ptab::{PTab, ConstParams};
  ///
  /// let table: PTab<u64, ConstParams<512>> = PTab::new();
  /// assert_eq!(table.capacity(), 512);
  /// ```
  ///
  /// [`Capacity::MAX`]: crate::params::Capacity::MAX
  /// [`Params::LENGTH`]: crate::params::Params::LENGTH
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
  /// Returns [`None`] if the table is full. [`write()`] gives the value its own
  /// index.
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
  /// [`write()`]: Self::write
  #[inline]
  pub fn insert(&self, value: T) -> Option<Detached>
  where
    T: 'static,
  {
    self.inner.insert(value)
  }

  /// Inserts a value built by `init`, which receives the entry's index.
  ///
  /// The stored value can contain its own index. Returns [`None`] if the table
  /// is full.
  ///
  /// `init` must fully initialize the [`MaybeUninit<T>`] before returning, and
  /// should not call back into the table.
  ///
  /// # Panics
  ///
  /// Panics if `init` panics. The slot is then leaked for the lifetime of the
  /// table.
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
  /// Returns `true` if an entry was removed, `false` if the slot was empty. The
  /// slot is available for reuse immediately. The collector reclaims the memory
  /// once no reader can hold a reference.
  ///
  /// Only the slot bits of `index` are used, so an index kept after its entry
  /// was removed removes the entry that now occupies that slot.
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
  pub fn remove(&self, index: Detached) -> bool
  where
    P::Collector: Collector, // Removed entries must eventually be reclaimed
  {
    self.inner.remove(index)
  }

  /// Returns `true` if an entry exists at the given index.
  ///
  /// The result may be stale as soon as it returns.
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
    self.inner.exists(index, &P::guard())
  }

  /// Accesses an entry by index, applying a function to it.
  ///
  /// Returns [`None`] if no entry exists. The reference stays valid for the
  /// duration of `f`, even if another thread removes the entry.
  ///
  /// Panics in `f` propagate to the caller.
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
    self.inner.with(index, &P::guard(), f)
  }

  /// Returns a copy of the entry at the given index.
  ///
  /// Equivalent to `self.with(index, |v| *v)`. Returns [`None`] if no entry
  /// exists.
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
    self.inner.read(index, &P::guard())
  }

  /// Returns a weakly consistent iterator over the indices of present entries.
  ///
  /// Entries inserted or removed during iteration may or may not be observed.
  /// See [`WeakKeys`] for details.
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
    self.inner.weak_keys(P::guard())
  }
}

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

impl<T, P> Default for PTab<T, P>
where
  P: Params + ?Sized,
{
  #[inline]
  fn default() -> Self {
    Self::new()
  }
}

#[cfg(test)]
mod tests {
  use super::PTab;

  fn assert_sync<T: Send + Sync>() {}

  #[test]
  fn ptab_string_is_send_and_sync() {
    assert_sync::<PTab<String>>();
  }
}
