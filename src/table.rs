//! Core table implementation.
//!
//! Uses lock-free operations with epoch-based reclamation via [`sdd`].
//!
//! [`sdd`]: https://docs.rs/sdd

use core::fmt::Debug;
use core::fmt::DebugMap;
use core::fmt::Formatter;
use core::fmt::Result;
use core::hint;
use core::marker::PhantomData;
use core::mem;
use core::mem::MaybeUninit;
use core::ptr::NonNull;

use sdd::AtomicOwned;
use sdd::Guard;
use sdd::Owned;
use sdd::Ptr;
use sdd::Tag;

use crate::array::Array;
use crate::index::Abstract;
use crate::index::Concrete;
use crate::index::Detached;
use crate::padded::CachePadded;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::Capacity;
use crate::params::Params;
use crate::params::ParamsExt;
use crate::sync::atomic::AtomicU32;
use crate::sync::atomic::AtomicUsize;
use crate::sync::atomic::Ordering;
use crate::sync::atomic::Ordering::AcqRel;
use crate::sync::atomic::Ordering::Acquire;
use crate::sync::atomic::Ordering::Relaxed;
use crate::sync::atomic::Ordering::Release;

/// Marker indicating a slot is reserved for an in-progress allocation.
const RESERVED: usize = usize::MAX;

#[inline]
fn store<F, T>(atomic: &AtomicOwned<T>, init: F, index: Detached)
where
  T: 'static,
  F: FnOnce(&mut MaybeUninit<T>, Detached),
{
  let new: Owned<T> = Owned::new_with(|| {
    let mut uninit: MaybeUninit<T> = MaybeUninit::uninit();

    init(&mut uninit, index);

    // SAFETY: Caller contract requires `init` to fully initialize `uninit`.
    unsafe { uninit.assume_init() }
  });

  let prv: (Option<Owned<T>>, Tag) = atomic.swap((Some(new), Tag::None), Release);

  debug_assert!(prv.0.is_none(), "AtomicOwned<T> is occupied!");
  debug_assert!(prv.1 == Tag::None, "AtomicOwned<T> is tagged!");

  // SAFETY: Slot allocation logic ensures the slot is empty before `store`.
  unsafe {
    hint::assert_unchecked(prv.0.is_none());
    hint::assert_unchecked(prv.1 == Tag::None);
  }
}

// -----------------------------------------------------------------------------
// Table State
// -----------------------------------------------------------------------------

#[repr(C)]
pub(crate) struct Table<T, P>
where
  P: Params + ?Sized,
{
  volatile: CachePadded<Volatile<P>>,
  readonly: CachePadded<ReadOnly<T, P>>,
}

impl<T, P> Table<T, P>
where
  P: Params + ?Sized,
{
  #[inline]
  pub(crate) fn new() -> Self {
    Self {
      volatile: CachePadded::new(Volatile::new()),
      readonly: CachePadded::new(ReadOnly::new()),
    }
  }

  #[inline]
  pub(crate) const fn cap(&self) -> usize {
    P::LENGTH.as_usize()
  }

  #[inline]
  pub(crate) fn len(&self) -> u32 {
    self.volatile.entries.load(Relaxed)
  }

  #[inline]
  pub(crate) fn is_empty(&self) -> bool {
    self.len() == 0
  }

  #[inline]
  pub(crate) fn insert(&self, value: T) -> Option<Detached>
  where
    T: 'static,
  {
    self.write(|entry, _| {
      entry.write(value);
    })
  }

  #[inline]
  pub(crate) fn write<F>(&self, init: F) -> Option<Detached>
  where
    T: 'static,
    F: FnOnce(&mut MaybeUninit<T>, Detached),
  {
    let claim_permit: Permit<'_, T, P> = self.reserve_slot()?;
    let abstract_idx: Abstract<P> = self.acquire_slot(claim_permit);
    let concrete_idx: Concrete<P> = Concrete::from_abstract(abstract_idx);
    let detached_idx: Detached = Detached::from_abstract(abstract_idx);

    store(self.readonly.data.get(concrete_idx), init, detached_idx);

    Some(detached_idx)
  }

  #[inline]
  pub(crate) fn remove(&self, key: Detached) -> bool {
    let index: Concrete<P> = Concrete::from_detached(key);
    let entry: &AtomicOwned<T> = self.readonly.data.get(index);
    let value: Option<Owned<T>> = entry.swap((None, Tag::None), AcqRel).0;

    if value.is_none() {
      return false;
    }

    self.release_slot(key);

    self.volatile.entries.fetch_sub(1, Release);

    true
  }

  #[inline]
  pub(crate) fn with<F, R>(&self, key: Detached, guard: &Guard, f: F) -> Option<R>
  where
    F: Fn(&T) -> R,
  {
    let value: Ptr<'_, T> = self.find(key, guard);

    // SAFETY: Tag bits are never set on data pointers.
    match unsafe { value.as_ref_unchecked() } {
      Some(data) => Some(f(data)),
      None => None,
    }
  }

  #[inline]
  pub(crate) fn exists(&self, key: Detached, guard: &Guard) -> bool {
    !self.find(key, guard).is_null()
  }

  #[inline]
  pub(crate) fn read(&self, key: Detached, guard: &Guard) -> Option<T>
  where
    T: Copy,
  {
    self.with(key, guard, |data| *data)
  }

  #[inline]
  pub(crate) fn weak_keys(&self) -> WeakKeys<'_, T, P> {
    WeakKeys::new(self)
  }

  #[inline]
  fn find<'guard>(&self, key: Detached, guard: &'guard Guard) -> Ptr<'guard, T> {
    self.load(Concrete::from_detached(key), guard)
  }

  #[inline]
  fn load<'guard>(&self, index: Concrete<P>, guard: &'guard Guard) -> Ptr<'guard, T> {
    self.readonly.data.get(index).load(Acquire, guard)
  }

  #[inline]
  fn reserve_slot(&self) -> Option<Permit<'_, T, P>> {
    let prev: u32 = self.volatile.entries.fetch_add(1, Relaxed);

    if prev < self.cap() as u32 {
      return Some(Permit::new(self));
    }

    // Table is full; undo the increment.
    self.restore_slot(prev)
  }

  #[cold]
  #[inline(never)]
  fn restore_slot(&self, prev: u32) -> Option<Permit<'_, T, P>> {
    let mut current: u32 = prev + 1;

    while let Err(next) =
      self
        .volatile
        .entries
        .compare_exchange_weak(current, current - 1, Relaxed, Relaxed)
    {
      current = next;
    }

    None
  }

  #[inline]
  fn acquire_slot(&self, _permit: Permit<'_, T, P>) -> Abstract<P> {
    loop {
      let result: usize = self
        .readonly
        .slot
        .get(Concrete::from_abstract(self.volatile.fetch_next_id()))
        .swap(RESERVED, Relaxed);

      if result != RESERVED {
        return Abstract::new(result);
      }
    }
  }

  #[inline]
  fn release_slot(&self, index: Detached) {
    let data: usize = self.generate_next_slot(index);

    while self
      .readonly
      .slot
      .get(Concrete::from_abstract(self.volatile.fetch_free_id()))
      .compare_exchange_weak(RESERVED, data, Relaxed, Relaxed)
      .is_err()
    {}
  }

  /// Computes the next abstract index for a slot being released.
  ///
  /// Adding capacity advances the generation, ensuring the same concrete slot
  /// produces a different abstract index on reuse (mitigating ABA problems).
  #[inline]
  const fn generate_next_slot(&self, index: Detached) -> usize {
    let mut data: usize = Abstract::<P>::from_detached(index).get();

    data = data.wrapping_add(self.cap());

    // Avoid the reserved marker value.
    if data == RESERVED {
      data = data.wrapping_add(self.cap());
    }

    data
  }

  #[cold]
  #[inline(never)]
  fn drop_slow(&mut self) {
    let count: u32 = self.len();

    if count == 0 {
      return;
    }

    let mut count: u32 = count;

    for entry in self.readonly.data.as_mut_slice() {
      #[cfg(not(loom))]
      let item: AtomicOwned<T> = mem::replace(entry, const { AtomicOwned::null() });

      #[cfg(loom)]
      let item: AtomicOwned<T> = mem::replace(entry, AtomicOwned::null());

      if let Some(value) = item.into_owned(Ordering::Relaxed) {
        // SAFETY: `Drop` provides exclusive access; no concurrent readers.
        unsafe {
          value.drop_in_place();
        }

        count -= 1;
      }

      if count == 0 {
        break;
      }
    }
  }
}

impl<T, P> Drop for Table<T, P>
where
  P: Params + ?Sized,
{
  #[inline]
  fn drop(&mut self) {
    if mem::needs_drop::<T>() {
      self.drop_slow();
    }
  }
}

impl<T, P> Debug for Table<T, P>
where
  T: Debug,
  P: Params + ?Sized,
{
  fn fmt(&self, f: &mut Formatter<'_>) -> Result {
    let mut debug: DebugMap<'_, '_> = f.debug_map();
    let guard: Guard = Guard::new();

    for (index, entry) in self.readonly.data.as_slice().iter().enumerate() {
      let value: Ptr<'_, T> = entry.load(Acquire, &guard);

      // SAFETY: Tag bits are never set on data pointers.
      if let Some(value) = unsafe { value.as_ref_unchecked() } {
        debug.entry(&index, value);
      }
    }

    debug.finish()
  }
}

// -----------------------------------------------------------------------------
// Volatile State
// -----------------------------------------------------------------------------

/// Mutable table state modified during operations.
///
/// Isolated from [`ReadOnly`] via cache padding to avoid false sharing.
#[repr(C)]
struct Volatile<P>
where
  P: Params + ?Sized,
{
  /// Current number of allocated entries.
  entries: AtomicU32,
  /// Counter for the next slot to try during allocation.
  next_id: AtomicU32,
  /// Counter for the next slot to try during deallocation.
  free_id: AtomicU32,
  phantom: PhantomData<fn(P)>,
}

impl<P> Volatile<P>
where
  P: Params + ?Sized,
{
  #[inline]
  fn new() -> Self {
    // At `Capacity::MAX`, one slot is permanently reserved because the table
    // cannot yield that many unique identifiers.
    Self {
      entries: AtomicU32::new(u32::from(P::LENGTH == Capacity::MAX)),
      next_id: AtomicU32::new(0),
      free_id: AtomicU32::new(0),
      phantom: PhantomData,
    }
  }

  #[inline]
  fn fetch_next_id(&self) -> Abstract<P> {
    Abstract::new(self.next_id.fetch_add(1, Relaxed) as usize)
  }

  #[inline]
  fn fetch_free_id(&self) -> Abstract<P> {
    Abstract::new(self.free_id.fetch_add(1, Relaxed) as usize)
  }
}

// -----------------------------------------------------------------------------
// Read-only State
// -----------------------------------------------------------------------------

/// Immutable table state initialized once at construction.
///
/// Individual slots are modified atomically, but the arrays themselves never
/// resize. Isolated from [`Volatile`] via cache padding.
#[repr(C)]
struct ReadOnly<T, P>
where
  P: Params + ?Sized,
{
  /// Storage for entry data pointers.
  data: Array<AtomicOwned<T>, P>,
  /// Slot metadata for free list management.
  slot: Array<AtomicUsize, P>,
}

impl<T, P> ReadOnly<T, P>
where
  P: Params + ?Sized,
{
  #[inline]
  fn new() -> Self {
    Self {
      data: Self::new_data_array(),
      slot: Self::new_slot_array(),
    }
  }

  #[cfg(not(loom))]
  #[inline]
  fn new_data_array() -> Array<AtomicOwned<T>, P> {
    // SAFETY: All-zeros is a valid null `AtomicOwned<T>`.
    unsafe { Array::new_zeroed().assume_init() }
  }

  #[cfg(loom)]
  #[inline]
  fn new_data_array() -> Array<AtomicOwned<T>, P> {
    Array::new(|_, slot| {
      slot.write(AtomicOwned::null());
    })
  }

  #[inline]
  fn new_slot_array() -> Array<AtomicUsize, P> {
    Array::new(|offset, item| {
      // Initialize slots with abstract indices distributed across cache lines.
      let block: usize = offset / CACHE_LINE_SLOTS;
      let index: usize = offset % CACHE_LINE_SLOTS;
      let value: usize = index * P::BLOCKS.get() + block;

      item.write(AtomicUsize::new(value));
    })
  }
}

// -----------------------------------------------------------------------------
// Permit
// -----------------------------------------------------------------------------

/// Proof token that capacity has been reserved for an allocation.
///
/// Consumed by [`Table::acquire_slot`] to complete the allocation.
struct Permit<'table, T, P>
where
  P: Params + ?Sized,
{
  marker: PhantomData<&'table Table<T, P>>,
}

impl<'table, T, P> Permit<'table, T, P>
where
  P: Params + ?Sized,
{
  #[inline]
  const fn new(_table: &'table Table<T, P>) -> Self {
    Self {
      marker: PhantomData,
    }
  }
}

// -----------------------------------------------------------------------------
// Keys Iterator - Weak Guarantees
// -----------------------------------------------------------------------------

/// Iterator over indices in a [`PTab`] with weak snapshot semantics.
///
/// `WeakKeys` performs a lock-free scan of the underlying table and yields
/// [`Detached`] indices for entries observed as present.
///
/// # Consistency Model
///
/// This iterator is **weakly consistent**:
///
/// - It does not guarantee a consistent snapshot.
/// - It does not prevent concurrent insertions or removals.
/// - It never yields an index that was never fully initialized.
/// - It may miss entries that were present when iteration began.
/// - It may yield entries that are removed immediately afterward.
///
/// [`PTab`]: crate::public::PTab
pub struct WeakKeys<'table, T, P>
where
  P: Params + ?Sized,
{
  array: NonNull<AtomicOwned<T>>,
  guard: Guard,
  index: usize,
  table: PhantomData<&'table ReadOnly<T, P>>,
}

impl<'table, T, P> WeakKeys<'table, T, P>
where
  P: Params + ?Sized,
{
  #[inline]
  pub(crate) fn new(table: &'table Table<T, P>) -> Self {
    Self {
      array: table.readonly.data.as_nonnull(),
      guard: Guard::new(),
      index: 0,
      table: PhantomData,
    }
  }
}

impl<T, P> Debug for WeakKeys<'_, T, P>
where
  P: Params + ?Sized,
{
  fn fmt(&self, f: &mut Formatter<'_>) -> Result {
    f.write_str("WeakKeys(..)")
  }
}

impl<T, P> Iterator for WeakKeys<'_, T, P>
where
  P: Params + ?Sized,
{
  type Item = Detached;

  #[inline]
  fn next(&mut self) -> Option<Self::Item> {
    let capapcity: usize = P::LENGTH.as_usize();
    let mut index: usize = self.index;

    while index < capapcity {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let concrete_idx: Concrete<P> = Concrete::from_abstract(abstract_idx);

      index += 1;

      let ptr: Ptr<'_, T> = {
        // SAFETY: `Concrete<P>` indices are always less than `P::LENGTH`.
        let raw: NonNull<AtomicOwned<T>> = unsafe { self.array.add(concrete_idx.get()) };

        // SAFETY: Pointer is valid and aligned.
        let data: &AtomicOwned<T> = unsafe { raw.as_ref() };

        data.load(Acquire, &self.guard)
      };

      if ptr.is_null() {
        continue;
      }

      self.index = index;

      return Some(Detached::from_abstract(abstract_idx));
    }

    self.index = index;

    None
  }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(not(any(loom, shuttle)))]
#[cfg(test)]
mod tests {
  use std::collections::HashSet;
  use std::sync::atomic::AtomicU32;
  use std::sync::atomic::Ordering;

  use sdd::Guard;

  use crate::index::Detached;
  use crate::params::Capacity;
  use crate::params::ConstParams;
  use crate::params::DefaultParams;
  use crate::table::Table;

  #[cfg(not(miri))]
  #[test]
  fn new() {
    let table: Table<usize, ConstParams<{ Capacity::DEF.as_usize() }>> = Table::new();

    assert_eq!(table.cap(), Capacity::DEF.as_usize());
    assert_eq!(table.len(), 0);
    assert!(table.is_empty());
  }

  #[test]
  fn insert_single() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.len(), 1);
    assert!(!table.is_empty());
    assert!(table.exists(index, &guard));
    assert_eq!(table.read(index, &guard), Some(123));
  }

  #[test]
  fn insert_multiple() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(16);

    for index in 0..16 {
      keys.push(table.insert(index * 100).unwrap());
    }

    assert_eq!(table.len(), 16);

    for (index, key) in keys.iter().enumerate() {
      assert_eq!(table.read(*key, &guard), Some(index * 100));
    }
  }

  #[test]
  fn insert_unique_ids() {
    let table: Table<usize, DefaultParams> = Table::new();
    let mut keys: HashSet<Detached> = HashSet::new();

    for _ in 0..table.cap() {
      assert!(keys.insert(table.insert(0).unwrap()));
    }
  }

  #[test]
  fn insert_maximum() {
    let table: Table<usize, DefaultParams> = Table::new();

    for index in 0..table.cap() {
      assert!(table.insert(index).is_some());
    }

    assert_eq!(table.len(), table.cap() as u32);
    assert_eq!(table.insert(9999), None);
  }

  #[test]
  fn write_callback_receives_correct_index() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();

    let index: Detached = table
      .write(|uninit, index| {
        uninit.write(index.into_bits());
      })
      .unwrap();

    assert_eq!(table.read(index, &guard), Some(index.into_bits()));
  }

  #[test]
  fn remove_existing() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.len(), 1);
    assert!(!table.is_empty());
    assert!(table.exists(index, &guard));
    assert!(table.remove(index));

    assert_eq!(table.len(), 0);
    assert!(table.is_empty());
    assert!(!table.exists(index, &guard));
  }

  #[test]
  fn remove_nonexistent() {
    let table: Table<usize, DefaultParams> = Table::new();
    let index: Detached = table.insert(123).unwrap();

    assert!(table.remove(index));
    assert!(!table.remove(index));
  }

  #[test]
  fn remove_isolation() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(16);

    for index in 0..16 {
      keys.push(table.insert(index).unwrap());
    }

    for index in (0..16).step_by(2) {
      table.remove(keys[index]);
    }

    assert_eq!(table.len(), 8);

    for index in (1..16).step_by(2) {
      assert_eq!(table.read(keys[index], &guard), Some(index));
    }

    for index in (0..16).step_by(2) {
      assert!(!table.exists(keys[index], &guard));
    }
  }

  #[test]
  fn remove_recycling() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(table.cap() - 1);

    for index in 0..table.cap() {
      keys.push(table.insert(index).unwrap());
    }

    assert_eq!(table.insert(99), None);

    table.remove(keys[0]);

    let index: Detached = table.insert(100).unwrap();

    assert!(table.exists(index, &guard));
    assert_eq!(table.read(index, &guard), Some(100));
  }

  #[test]
  fn exists_existing() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let index: Detached = table.insert(123).unwrap();

    assert!(table.exists(index, &guard));
  }

  #[test]
  fn exists_nonexistent() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let index: Detached = table.insert(123).unwrap();

    assert!(table.remove(index));
    assert!(!table.exists(index, &guard));
  }

  #[test]
  fn exists_multiple() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();

    let index1: Detached = table.insert(1).unwrap();
    let index2: Detached = table.insert(2).unwrap();
    let index3: Detached = table.insert(3).unwrap();

    assert!(table.exists(index1, &guard));
    assert!(table.exists(index2, &guard));
    assert!(table.exists(index3, &guard));

    table.remove(index2);

    assert!(table.exists(index1, &guard));
    assert!(!table.exists(index2, &guard));
    assert!(table.exists(index3, &guard));
  }

  #[test]
  fn with_value() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let index: Detached = table.insert(12345).unwrap();

    assert_eq!(table.with(index, &guard, |data| *data), Some(12345));
  }

  #[test]
  fn with_return_value() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.with(index, &guard, |data| data + 1), Some(124));
  }

  #[test]
  fn with_nonexistent() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let index: Detached = table.insert(123).unwrap();

    assert!(table.remove(index));
    assert_eq!(table.with(index, &guard, |data| *data), None);
  }

  #[test]
  fn with_multiple() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let index: Detached = table.insert(123).unwrap();

    for _ in 0..100 {
      assert_eq!(table.with(index, &guard, |data| *data), Some(123));
    }
  }

  #[test]
  fn len_tracks_insertions() {
    let table: Table<usize, DefaultParams> = Table::new();

    for index in 0..16 {
      assert!(table.insert(0).is_some());
      assert_eq!(table.len(), index + 1);
    }
  }

  #[test]
  fn len_tracks_removals() {
    let table: Table<usize, DefaultParams> = Table::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(16);

    for _ in 0..16 {
      keys.push(table.insert(0).unwrap());
    }

    for (index, key) in keys.iter().enumerate() {
      assert!(table.remove(*key));
      assert_eq!(table.len() as usize, 16 - index - 1);
    }
  }

  #[test]
  fn is_empty() {
    let table: Table<usize, DefaultParams> = Table::new();

    assert!(table.is_empty());

    let index: Detached = table.insert(0).unwrap();

    assert!(!table.is_empty());
    assert!(table.remove(index));
    assert!(table.is_empty());
  }

  #[test]
  fn interleaved_insert_remove() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(16);

    for index in 0..8 {
      keys.push(table.insert(index).unwrap());
    }
    assert_eq!(table.len(), 8);

    for _ in 0..4 {
      table.remove(keys.pop().unwrap());
    }
    assert_eq!(table.len(), 4);

    for index in 100..108 {
      keys.push(table.insert(index).unwrap());
    }
    assert_eq!(table.len(), 12);

    for key in keys {
      assert!(table.exists(key, &guard));
    }
  }

  #[test]
  fn multiple_refills() {
    let table: Table<usize, DefaultParams> = Table::new();
    let guard: Guard = Guard::new();

    for round in 0..3 {
      let mut keys: Vec<Detached> = Vec::new();

      for index in 0..16 {
        keys.push(table.insert(round * 100 + index).unwrap());
      }
      assert_eq!(table.len(), 16);

      for (index, key) in keys.iter().enumerate() {
        assert_eq!(table.read(*key, &guard), Some(round * 100 + index));
      }

      for key in keys {
        table.remove(key);
      }
      assert_eq!(table.len(), 0);
    }
  }

  #[test]
  fn uniqueness_across_multiple_generations() {
    const GENS: usize = 10;

    let table: Table<usize, DefaultParams> = Table::new();
    let mut key_set: HashSet<usize> = HashSet::with_capacity(GENS * 16);

    for _ in 0..GENS {
      let mut key_arr: Vec<Detached> = Vec::with_capacity(16);

      for _ in 0..16 {
        let index: Detached = table.insert(0).unwrap();

        key_arr.push(index);

        assert!(key_set.insert(index.into_bits()));
      }

      for key in key_arr {
        assert!(table.remove(key));
      }
    }
  }

  #[test]
  fn min_capacity_operations() {
    type Params = ConstParams<{ Capacity::MIN.as_usize() }>;

    let table: Table<usize, Params> = Table::new();
    let guard: Guard = Guard::new();

    assert_eq!(table.cap(), Capacity::MIN.as_usize());

    let index: Detached = table.insert(99).unwrap();

    assert!(table.exists(index, &guard));
    assert_eq!(table.read(index, &guard), Some(99));
    assert!(table.remove(index));
    assert!(!table.exists(index, &guard));
  }

  #[cfg_attr(
    any(miri, not(feature = "slow")),
    ignore = "enable the 'slow' feature to run this test."
  )]
  #[test]
  fn max_capacity_operations() {
    type Params = ConstParams<{ Capacity::MAX.as_usize() }>;

    let table: Table<usize, Params> = Table::new();

    assert_eq!(table.cap(), Capacity::MAX.as_usize());
    assert_eq!(table.len(), 1); // See `Volatile::new`
  }

  #[test]
  fn drop_slow() {
    static COUNT: AtomicU32 = AtomicU32::new(0);

    struct DropMe;

    impl DropMe {
      fn new() -> Self {
        COUNT.fetch_add(1, Ordering::Relaxed);
        Self
      }
    }

    impl Drop for DropMe {
      fn drop(&mut self) {
        COUNT.fetch_sub(1, Ordering::Relaxed);
      }
    }

    let drop_0: Table<DropMe, DefaultParams> = Table::new();

    assert_eq!(COUNT.load(Ordering::Relaxed), 0);
    assert_eq!(COUNT.load(Ordering::Relaxed), drop_0.len());
    drop(drop_0);
    assert_eq!(COUNT.load(Ordering::Relaxed), 0);

    let drop_1: Table<DropMe, DefaultParams> = {
      let this = Table::new();
      this.insert(DropMe::new()).unwrap();
      this
    };

    assert_eq!(COUNT.load(Ordering::Relaxed), 1);
    assert_eq!(COUNT.load(Ordering::Relaxed), drop_1.len());
    drop(drop_1);
    assert_eq!(COUNT.load(Ordering::Relaxed), 0);

    let drop_full: Table<DropMe, DefaultParams> = {
      let this = Table::new();

      for _ in 0..this.cap() {
        this.insert(DropMe::new()).unwrap();
      }

      this
    };

    assert_eq!(COUNT.load(Ordering::Relaxed), drop_full.cap() as u32);
    assert_eq!(COUNT.load(Ordering::Relaxed), drop_full.len());
    drop(drop_full);
    assert_eq!(COUNT.load(Ordering::Relaxed), 0);
  }

  #[test]
  fn weak_keys() {
    let table: Table<usize, DefaultParams> = Table::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(16);

    for _ in 0..16 {
      keys.push(table.insert(0).unwrap());
    }

    let new: Vec<Detached> = table.weak_keys().collect();

    for (init_key, iter_key) in keys.into_iter().zip(new.into_iter()) {
      assert_eq!(init_key, iter_key);
      assert!(table.remove(init_key));
    }

    assert_eq!(table.weak_keys().next(), None);
  }
}
