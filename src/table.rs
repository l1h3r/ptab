use core::fmt::Debug;
use core::fmt::DebugMap;
use core::fmt::Formatter;
use core::fmt::Result as FmtResult;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::panic::RefUnwindSafe;
use core::panic::UnwindSafe;
use core::ptr::NonNull;

use crate::array::Array;
use crate::index::Abstract;
use crate::index::Concrete;
use crate::index::Detached;
use crate::padded::CachePadded;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::Capacity;
use crate::params::Params;
use crate::params::ParamsExt;
use crate::reclaim;
use crate::reclaim::Atomic as _;
use crate::reclaim::CollectorWeak;
use crate::reclaim::Shared as _;
use crate::sync::atomic::AtomicU32;
use crate::sync::atomic::AtomicUsize;
use crate::sync::atomic::Ordering::AcqRel;
use crate::sync::atomic::Ordering::Acquire;
use crate::sync::atomic::Ordering::Relaxed;
use crate::sync::atomic::Ordering::Release;

/// Marker indicating a slot is reserved for an in-progress allocation.
const RESERVED: usize = usize::MAX;

type Guard<P> = <<P as Params>::Collector as CollectorWeak>::Guard;

type Atomic<T, P> = <<P as Params>::Collector as CollectorWeak>::Atomic<T>;
type Shared<'guard, T, P> = <Atomic<T, P> as reclaim::Atomic<T>>::Shared<'guard>;

type DataArray<T, P> = Array<Atomic<T, P>, P>;
type SlotArray<P> = Array<AtomicUsize, P>;

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
    // See `Volatile::new`
    if P::LENGTH.as_usize() == Capacity::MAX.as_usize() {
      P::LENGTH.as_usize().wrapping_sub(1)
    } else {
      P::LENGTH.as_usize()
    }
  }

  #[inline]
  pub(crate) fn len(&self) -> u32 {
    let mut len: u32 = self.volatile.load_entries();
    let mut max: u32 = P::LENGTH.as_u32();

    // See `Volatile::new`
    if max == Capacity::MAX.as_u32() {
      len = len.wrapping_sub(1);
      max = max.wrapping_sub(1);
    }

    // We may see an invalid `len` from a concurrent insert attempt; fix it here
    if len > max {
      return max;
    }

    len
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

    self
      .readonly
      .data
      .get(concrete_idx)
      .write(Release, |maybe| init(maybe, detached_idx));

    Some(detached_idx)
  }

  #[inline]
  pub(crate) fn remove(&self, key: Detached) -> bool {
    let index: Concrete<P> = Concrete::from_detached(key);
    let entry: &Atomic<T, P> = self.readonly.data.get(index);

    if entry.evict(AcqRel) {
      self.release_slot(Abstract::from_detached(key));
      true
    } else {
      false
    }
  }

  #[inline]
  pub(crate) fn with<F, R>(&self, key: Detached, guard: &Guard<P>, f: F) -> Option<R>
  where
    F: Fn(&T) -> R,
  {
    self.find(key, guard).as_ref().map(f)
  }

  #[inline]
  pub(crate) fn exists(&self, key: Detached, guard: &Guard<P>) -> bool {
    !self.find(key, guard).is_null()
  }

  #[inline]
  pub(crate) fn read(&self, key: Detached, guard: &Guard<P>) -> Option<T>
  where
    T: Copy,
  {
    self.with(key, guard, |data| *data)
  }

  #[inline]
  pub(crate) fn weak_keys(&self, guard: Guard<P>) -> WeakKeys<'_, T, P> {
    WeakKeys::new(guard, self)
  }

  #[inline]
  fn find<'guard>(&self, key: Detached, guard: &'guard Guard<P>) -> Shared<'guard, T, P> {
    self.load(Concrete::from_detached(key), guard)
  }

  #[inline]
  fn load<'guard>(&self, index: Concrete<P>, guard: &'guard Guard<P>) -> Shared<'guard, T, P> {
    self.readonly.data.get(index).read(Acquire, guard)
  }

  #[inline]
  fn reserve_slot(&self) -> Option<Permit<'_, T, P>> {
    let prev: u32 = self.volatile.incr_entries();

    if prev < P::LENGTH.as_u32() {
      return Some(Permit::new(self));
    }

    // Table is full; undo the increment.
    let mut current: u32 = prev.wrapping_add(1);

    while let Err(next) = self.volatile.swap_entries(current, current.wrapping_sub(1)) {
      current = next;
    }

    None
  }

  #[inline]
  fn acquire_slot(&self, _permit: Permit<'_, T, P>) -> Abstract<P> {
    loop {
      let abstract_idx: Abstract<P> = self.volatile.fetch_next_id();
      let concrete_idx: Concrete<P> = Concrete::from_abstract(abstract_idx);

      let atomic: &AtomicUsize = self.readonly.slot.get(concrete_idx);
      let result: usize = atomic.swap(RESERVED, Relaxed);

      if result == RESERVED {
        continue;
      }

      break Abstract::new(result);
    }
  }

  #[inline]
  fn release_slot(&self, index: Abstract<P>) {
    let data: usize = self.generate_next_slot(index);

    while self
      .readonly
      .slot
      .get(Concrete::from_abstract(self.volatile.fetch_free_id()))
      .compare_exchange_weak(RESERVED, data, Relaxed, Relaxed)
      .is_err()
    {}

    self.volatile.decr_entries();
  }

  #[inline]
  const fn generate_next_slot(&self, index: Abstract<P>) -> usize {
    let mut data: usize = index.get();

    data = data.wrapping_add(P::LENGTH.as_usize());

    if data == RESERVED {
      data = data.wrapping_add(P::LENGTH.as_usize());
    }

    data
  }
}

impl<T, P> Drop for Table<T, P>
where
  P: Params + ?Sized,
{
  #[inline]
  fn drop(&mut self) {
    let mut count: u32 = self.len();

    if count == 0 {
      return;
    }

    for entry in self.readonly.data.as_mut_slice() {
      // SAFETY:
      // - `Drop` provides exclusive access via `&mut self`, so no concurrent
      //   access can occur.
      // - Each slot is dropped at most once.
      if unsafe { entry.clear() } {
        count = count.wrapping_sub(1);

        if count == 0 {
          break;
        }
      }
    }
  }
}

// SAFETY:
// - All internal mutation is performed via atomics.
// - Memory reclamation is handled through epoch-based reclamation.
// - Transferring ownership of `Table` between threads is safe provided
//   `T: Send`, so contained values may be transferred across threads.
unsafe impl<T, P> Send for Table<T, P>
where
  T: Send,
  P: Params + ?Sized,
{
}

// SAFETY:
// - All shared access is mediated through atomic operations.
// - Methods only yield shared references tied to a `Guard`.
// - Values may be accessed from multiple threads provided `T: Send`, which is
//   sufficient because no `&mut T` is ever exposed.
unsafe impl<T, P> Sync for Table<T, P>
where
  T: Send,
  P: Params + ?Sized,
{
}

// Unconditional because `Table` provides only shared access to `T` via `with`,
// and epoch-based reclamation handles panic unwind safely.
impl<T, P> RefUnwindSafe for Table<T, P> where P: Params + ?Sized {}
impl<T, P> UnwindSafe for Table<T, P> where P: Params + ?Sized {}

impl<T, P> Debug for Table<T, P>
where
  T: Debug,
  P: Params + ?Sized,
{
  fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
    let guard: Guard<P> = <P::Collector as CollectorWeak>::guard();
    let mut debug: DebugMap<'_, '_> = f.debug_map();

    for index in 0..P::LENGTH.as_usize() {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let concrete_idx: Concrete<P> = Concrete::from_abstract(abstract_idx);

      if let Some(value) = self.load(concrete_idx, &guard).as_ref() {
        debug.entry(&Detached::from_abstract(abstract_idx), value);
      }
    }

    debug.finish()
  }
}

// -----------------------------------------------------------------------------
// Volatile State
// -----------------------------------------------------------------------------

#[repr(C)]
struct Volatile<P>
where
  P: Params + ?Sized,
{
  entries: AtomicU32,
  next_id: AtomicU32,
  free_id: AtomicU32,
  phantom: PhantomData<fn(P)>,
}

impl<P> Volatile<P>
where
  P: Params + ?Sized,
{
  #[inline]
  fn new() -> Self {
    // At `Capacity::MAX`, one slot is permanently reserved because we can't
    // produce enough unique identifiers.
    Self {
      entries: AtomicU32::new(u32::from(P::LENGTH == Capacity::MAX)),
      next_id: AtomicU32::new(0),
      free_id: AtomicU32::new(0),
      phantom: PhantomData,
    }
  }

  #[inline]
  fn load_entries(&self) -> u32 {
    self.entries.load(Relaxed)
  }

  #[inline]
  fn incr_entries(&self) -> u32 {
    self.entries.fetch_add(1, Acquire)
  }

  #[inline]
  fn decr_entries(&self) -> u32 {
    self.entries.fetch_sub(1, Release)
  }

  #[inline]
  fn swap_entries(&self, current: u32, updated: u32) -> Result<u32, u32> {
    self
      .entries
      .compare_exchange_weak(current, updated, Release, Relaxed)
  }

  #[inline]
  fn fetch_next_id(&self) -> Abstract<P> {
    Abstract::new(self.next_id.fetch_add(1, Relaxed) as usize)
  }

  #[inline]
  fn fetch_free_id(&self) -> Abstract<P> {
    Abstract::new(self.free_id.fetch_add(1, Relaxed) as usize)
  }

  #[allow(dead_code, reason = "not used by loom/shuttle tests")]
  #[cfg(test)]
  #[inline]
  fn load_next_id(&self) -> usize {
    self.next_id.load(Relaxed) as usize
  }

  #[allow(dead_code, reason = "not used by loom/shuttle tests")]
  #[cfg(test)]
  #[inline]
  fn load_free_id(&self) -> usize {
    self.free_id.load(Relaxed) as usize
  }
}

// -----------------------------------------------------------------------------
// Read-only State
// -----------------------------------------------------------------------------

#[repr(C)]
struct ReadOnly<T, P>
where
  P: Params + ?Sized,
{
  data: DataArray<T, P>,
  slot: SlotArray<P>,
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

  #[inline]
  fn new_data_array() -> DataArray<T, P> {
    Array::new(|_, slot| {
      slot.write(Atomic::<T, P>::null());
    })
  }

  #[inline]
  fn new_slot_array() -> SlotArray<P> {
    Array::new(|offset, item| {
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
// Keys Iterator - Weak Snapshot
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
  array: NonNull<Atomic<T, P>>,
  guard: Guard<P>,
  total: usize,
  index: usize,
  table: PhantomData<&'table Table<T, P>>,
}

impl<'table, T, P> WeakKeys<'table, T, P>
where
  P: Params + ?Sized,
{
  #[inline]
  pub(crate) fn new(guard: Guard<P>, table: &'table Table<T, P>) -> Self {
    Self {
      array: table.readonly.data.as_non_null(),
      guard,
      total: table.cap(),
      index: 0,
      table: PhantomData,
    }
  }
}

impl<T, P> Debug for WeakKeys<'_, T, P>
where
  P: Params + ?Sized,
{
  fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
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
    let guard: &Guard<P> = &self.guard;
    let total: usize = self.total;

    let mut index: usize = self.index;

    while index < total {
      let abstract_idx: Abstract<P> = Abstract::new(index);
      let concrete_idx: Concrete<P> = Concrete::from_abstract(abstract_idx);

      index += 1;

      let ptr: Shared<'_, T, P> = {
        // SAFETY:
        // - `Concrete<P>` guarantees `concrete_idx.get() < P::LENGTH`.
        // - `self.array` points to a contiguous allocation of `P::LENGTH` elements.
        let raw: NonNull<Atomic<T, P>> = unsafe { self.array.add(concrete_idx.get()) };

        // SAFETY:
        // - `raw` was derived from a valid allocation.
        // - The pointer is properly aligned for `Atomic<T>`.
        // - The iterator only performs shared access.
        let data: &Atomic<T, P> = unsafe { raw.as_ref() };

        data.read(Acquire, guard)
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
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(test)]
mod tests {
  use std::collections::HashSet;
  use std::sync::Arc;
  use std::sync::Barrier;
  use std::thread;
  use std::thread::JoinHandle;

  use crate::index::Abstract;
  use crate::index::Concrete;
  use crate::index::Detached;
  use crate::params::CACHE_LINE_SLOTS;
  use crate::params::Capacity;
  use crate::params::ConstParams;
  use crate::params::Params;
  use crate::params::ParamsExt;
  use crate::reclaim::Atomic as _;
  use crate::sync::atomic::AtomicUsize;
  use crate::sync::atomic::Ordering;
  use crate::table;
  use crate::table::Atomic;
  use crate::table::DataArray;
  use crate::table::Guard;
  use crate::table::Permit;
  use crate::table::RESERVED;
  use crate::table::SlotArray;
  use crate::table::Table;

  type DefParams = ConstParams<{ Capacity::DEF.as_usize() }>;
  type MaxParams = ConstParams<{ Capacity::MAX.as_usize() }>;
  type MinParams = ConstParams<{ Capacity::MIN.as_usize() }>;

  type ReadOnly<P = DefParams> = table::ReadOnly<u64, P>;

  const THREADS: usize = 8;

  macro_rules! refute {
    ($cond:expr $(,)?) => {
      ::core::assert!(!$cond);
    };
  }

  macro_rules! make_drop {
    ($name:ident) => {
      static COUNT: ::core::sync::atomic::AtomicU32 = ::core::sync::atomic::AtomicU32::new(0);

      struct $name;

      impl $name {
        #[allow(dead_code, reason = "not used by all tests")]
        fn new() -> Self {
          COUNT.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
          Self
        }

        fn load() -> u32 {
          COUNT.load(::core::sync::atomic::Ordering::Relaxed)
        }
      }

      impl Drop for $name {
        fn drop(&mut self) {
          COUNT.fetch_sub(1, ::core::sync::atomic::Ordering::Relaxed);
        }
      }
    };
  }

  // ---------------------------------------------------------------------------
  // Internals
  // ---------------------------------------------------------------------------

  #[test]
  fn new_data_array() {
    let array: DataArray<u64, DefParams> = ReadOnly::new_data_array();
    let slice: &[Atomic<u64, DefParams>] = array.as_slice();
    let guard: Guard<DefParams> = DefParams::guard();

    for atomic in slice {
      assert!(atomic.read(Ordering::Relaxed, &guard).is_null());
    }
  }

  #[test]
  fn new_slot_array() {
    let array: SlotArray<DefParams> = ReadOnly::new_slot_array();
    let slice: &[AtomicUsize] = array.as_slice();

    let mut offset: usize = 0;

    for block in 0..DefParams::BLOCKS.get() {
      for slot in 0..CACHE_LINE_SLOTS {
        let expected: usize = slot * DefParams::BLOCKS.get() + block;
        let received: usize = slice[offset].load(Ordering::Relaxed);
        assert_eq!(received, expected);
        offset += 1;
      }
    }
  }

  #[test]
  fn reserve_slot() {
    let table: Table<usize, DefParams> = Table::new();

    for _ in 0..table.cap() {
      assert!(table.reserve_slot().is_some());
    }

    assert_eq!(table.len(), table.cap() as u32);
    assert!(table.reserve_slot().is_none());
    assert_eq!(table.len(), table.cap() as u32);
  }

  // Scenario: The table fills up and multiple threads race to claim slots.
  // Expected: We never hand out `Permit`s beyond the available capacity.
  #[test]
  fn reserve_slot_race() {
    static PERMITS: AtomicUsize = AtomicUsize::new(0);

    let table: Arc<Table<usize, DefParams>> = Arc::new(Table::new());
    let barrier: Arc<Barrier> = Arc::new(Barrier::new(THREADS + 1));

    let mut threads: Vec<JoinHandle<()>> = Vec::with_capacity(THREADS);

    for _ in 0..THREADS {
      let barrier: Arc<Barrier> = Arc::clone(&barrier);
      let table: Arc<Table<usize, DefParams>> = Arc::clone(&table);

      threads.push(thread::spawn(move || {
        barrier.wait();

        for _ in 0..table.cap() {
          if let Some(_permit) = table.reserve_slot() {
            PERMITS.fetch_add(1, Ordering::Relaxed);
          }

          thread::yield_now();
        }
      }));
    }

    barrier.wait();

    for thread in threads {
      thread.join().unwrap();
    }

    assert_eq!(table.len(), table.cap() as u32);
    assert_eq!(table.cap(), PERMITS.load(Ordering::Relaxed));
  }

  #[test]
  fn acquire_slot() {
    let table: Table<usize, DefParams> = Table::new();
    let mut indices: HashSet<usize> = HashSet::with_capacity(table.cap());

    for _ in 0..table.cap() {
      let reserved: Permit<'_, usize, DefParams> = table.reserve_slot().unwrap();
      let acquired: Abstract<DefParams> = table.acquire_slot(reserved);

      assert!(indices.insert(acquired.get()));
    }

    assert_eq!(table.len(), indices.len() as u32);
    assert_eq!(table.cap(), table.volatile.load_next_id());
  }

  #[test]
  fn release_slot() {
    let table: Table<usize, DefParams> = Table::new();

    for _ in 0..table.cap() {
      let reserved: Permit<'_, usize, DefParams> = table.reserve_slot().unwrap();
      let acquired: Abstract<DefParams> = table.acquire_slot(reserved);

      table.release_slot(acquired);
    }

    assert!(table.is_empty());
    assert!(table.volatile.load_free_id().is_multiple_of(table.cap()));
  }

  #[test]
  fn generate_next_slot() {
    let table: Table<usize, DefParams> = Table::new();

    for index in 0..table.cap() {
      let old: Abstract<DefParams> = Abstract::new(index);
      let new: Abstract<DefParams> = Abstract::new(table.generate_next_slot(old));

      assert_ne!(old, new);
    }
  }

  #[test]
  fn generate_next_slot_uniqueness() {
    const GENERATIONS: usize = 10;

    let table: Table<usize, DefParams> = Table::new();
    let mut indices: HashSet<usize> = HashSet::with_capacity(table.cap());
    let mut uniques: HashSet<Detached> = HashSet::with_capacity(GENERATIONS * table.cap());

    for generation in 0..GENERATIONS {
      let gen_index: usize = generation * table.cap();

      for index in 0..table.cap() {
        let old: Abstract<DefParams> = Abstract::new(gen_index + index);
        let new: Abstract<DefParams> = Abstract::new(table.generate_next_slot(old));
        let _in: bool = indices.insert(Concrete::from_abstract(new).get());

        assert!(uniques.insert(Detached::from_abstract(new)));
      }
    }

    assert_eq!(uniques.len(), GENERATIONS * table.cap());
    assert_eq!(indices.len(), table.cap());
  }

  #[test]
  fn generate_next_slot_skips_reserved() {
    let table: Table<usize, DefParams> = Table::new();
    let index: Abstract<DefParams> = Abstract::new(RESERVED - table.cap());
    let value: usize = table.generate_next_slot(index);

    assert_ne!(value, RESERVED);
  }

  // Scenario: The table fills up as multiple threads manipulate slots.
  // Expected:
  // - We never hand out `Permit`s beyond the available capacity.
  // - Every `Permit` is honored in `acquire_slot`.
  // - All keys are unique.
  // - The table count stays in bounds.
  #[test]
  fn slot_churn() {
    static PERMITS: AtomicUsize = AtomicUsize::new(0);
    static UNIQUES: AtomicUsize = AtomicUsize::new(0);

    let table: Arc<Table<usize, DefParams>> = Arc::new(Table::new());
    let barrier: Arc<Barrier> = Arc::new(Barrier::new(THREADS + 1));
    let capacity: usize = table.cap();

    let mut threads: Vec<JoinHandle<Vec<usize>>> = Vec::with_capacity(THREADS);
    let mut uniques: HashSet<usize> = HashSet::with_capacity(capacity);

    for _ in 0..THREADS {
      let barrier: Arc<Barrier> = Arc::clone(&barrier);
      let table: Arc<Table<usize, DefParams>> = Arc::clone(&table);

      threads.push(thread::spawn(move || {
        let mut track: Vec<usize> = Vec::with_capacity(capacity);
        let mut index: usize = 0;

        barrier.wait();

        for _ in 0..4 {
          for _ in 0..capacity {
            if let Some(permit) = table.reserve_slot() {
              track.push(table.acquire_slot(permit).get());
              index += 1;

              assert!(table.len() <= capacity as u32);
              assert!(PERMITS.fetch_add(1, Ordering::Relaxed) <= capacity);

              UNIQUES.fetch_add(1, Ordering::Relaxed);
            }

            if index.is_multiple_of(2)
              && let Some(index) = track[index..].first()
            {
              table.release_slot(Abstract::new(*index));
              PERMITS.fetch_sub(1, Ordering::Relaxed);
            }

            thread::yield_now();
          }
        }

        track
      }));
    }

    barrier.wait();

    for thread in threads {
      for key in thread.join().unwrap() {
        assert!(uniques.insert(key));
      }
    }

    assert_eq!(uniques.len(), UNIQUES.load(Ordering::Relaxed));
    assert_eq!(table.cap(), PERMITS.load(Ordering::Relaxed));
  }

  // ---------------------------------------------------------------------------
  // API
  // ---------------------------------------------------------------------------

  #[test]
  fn cap() {
    let table: Table<usize, DefParams> = Table::new();
    assert_eq!(table.cap(), Capacity::DEF.as_usize());

    let table: Table<usize, MaxParams> = Table::new();
    assert_eq!(table.cap(), Capacity::MAX.as_usize() - 1);

    let table: Table<usize, MinParams> = Table::new();
    assert_eq!(table.cap(), Capacity::MIN.as_usize());
  }

  #[test]
  fn len() {
    let table: Table<usize, DefParams> = Table::new();
    assert_eq!(table.len(), 0);

    let table: Table<usize, MaxParams> = Table::new();
    assert_eq!(table.len(), 0);

    let table: Table<usize, MinParams> = Table::new();
    assert_eq!(table.len(), 0);
  }

  // Scenario: The table is temporarily above capacity due to concurrent writes.
  // Expected: The `len` never surpasses `cap`.
  #[test]
  fn len_clamp() {
    let table: Table<usize, DefParams> = Table::new();

    table
      .volatile
      .entries
      .store(DefParams::LENGTH.as_u32() + 1, Ordering::Relaxed);

    assert_eq!(table.len(), table.cap() as u32);
  }

  #[test]
  fn is_empty() {
    let table: Table<usize, DefParams> = Table::new();
    assert!(table.is_empty());

    let table: Table<usize, MaxParams> = Table::new();
    assert!(table.is_empty());

    let table: Table<usize, MinParams> = Table::new();
    assert!(table.is_empty());
  }

  #[test]
  fn insert() {
    let table: Table<usize, DefParams> = Table::new();

    assert_ne!(table.insert(123), None);
    assert_eq!(table.len(), 1);

    for index in 0..16 {
      assert_ne!(table.insert(123), None);
      assert_eq!(table.len(), index + 2);
    }

    assert_eq!(table.len(), 17);
  }

  #[test]
  fn insert_beyond_capacity() {
    let table: Table<usize, DefParams> = Table::new();

    for _ in 0..table.cap() {
      assert_ne!(table.insert(123), None);
    }

    assert_eq!(table.len(), table.cap() as u32);
    assert_eq!(table.insert(123), None);
    assert_eq!(table.len(), table.cap() as u32);
  }

  #[test]
  fn insert_unique_ids() {
    let table: Table<usize, DefParams> = Table::new();
    let mut keys: HashSet<Detached> = HashSet::with_capacity(table.cap());

    for _ in 0..table.cap() {
      assert!(keys.insert(table.insert(123).unwrap()));
    }
  }

  #[test]
  fn write_callback_correct_index() {
    let table: Table<usize, DefParams> = Table::new();
    let guard: Guard<DefParams> = DefParams::guard();

    let index: Detached = table
      .write(|uninit, index| {
        uninit.write(index.into_bits());
      })
      .unwrap();

    assert_eq!(table.read(index, &guard), Some(index.into_bits()));
  }

  #[test]
  fn remove() {
    let table: Table<usize, DefParams> = Table::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.len(), 1);
    assert!(!table.is_empty());

    assert!(table.remove(index));

    assert_eq!(table.len(), 0);
    assert!(table.is_empty());
  }

  #[test]
  fn remove_nonexistent() {
    let table: Table<usize, DefParams> = Table::new();
    let index: Detached = table.insert(123).unwrap();

    assert!(table.remove(index));
    refute!(table.remove(index));
  }

  #[test]
  fn remove_recycling() {
    let table: Table<usize, DefParams> = Table::new();
    let guard: Guard<DefParams> = DefParams::guard();
    let mut keys: Vec<Detached> = Vec::with_capacity(table.cap());

    for index in 0..table.cap() {
      keys.push(table.insert(index).unwrap());
    }

    assert!(table.insert(123).is_none());
    assert!(table.remove(keys[0]));

    let index: Detached = table.insert(456).unwrap();

    assert!(table.exists(index, &guard));
    assert_eq!(table.read(index, &guard), Some(456));

    for key in keys.drain(1..).rev() {
      assert!(table.remove(key));
    }

    for index in 0..table.cap() - 1 {
      assert!(table.insert(index).is_some());
    }
  }

  #[test]
  fn with() {
    let table: Table<usize, DefParams> = Table::new();
    let guard: Guard<DefParams> = DefParams::guard();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.with(index, &guard, |item| item + 1), Some(124));
  }

  #[test]
  fn with_nonexistent() {
    let table: Table<usize, DefParams> = Table::new();
    let guard: Guard<DefParams> = DefParams::guard();
    let index: Detached = Detached::from_bits(123);

    assert_eq!(table.with(index, &guard, |item| item + 1), None);
  }

  #[test]
  fn exists() {
    let table: Table<usize, DefParams> = Table::new();
    let guard: Guard<DefParams> = DefParams::guard();
    let index: Detached = table.insert(123).unwrap();

    assert!(table.exists(index, &guard));
  }

  #[test]
  fn exists_nonexistent() {
    let table: Table<usize, DefParams> = Table::new();
    let guard: Guard<DefParams> = DefParams::guard();
    let index: Detached = Detached::from_bits(123);

    refute!(table.exists(index, &guard));
  }

  #[test]
  fn read() {
    let table: Table<usize, DefParams> = Table::new();
    let guard: Guard<DefParams> = DefParams::guard();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.read(index, &guard), Some(123));
  }

  #[test]
  fn read_nonexistent() {
    let table: Table<usize, DefParams> = Table::new();
    let guard: Guard<DefParams> = DefParams::guard();
    let index: Detached = Detached::from_bits(123);

    assert_eq!(table.read(index, &guard), None);
  }

  #[test]
  fn weak_keys() {
    let table: Table<usize, DefParams> = Table::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(table.cap());

    assert_eq!(table.weak_keys(DefParams::guard()).next(), None);

    for index in 0..table.cap() {
      keys.push(table.insert(index).unwrap());
    }

    assert_eq!(table.weak_keys(DefParams::guard()).count(), table.cap());

    for (init_key, iter_key) in keys.into_iter().zip(table.weak_keys(DefParams::guard())) {
      assert_eq!(init_key, iter_key);
    }
  }

  #[test]
  fn drop_empty() {
    make_drop!(DropMe);

    assert_eq!(DropMe::load(), 0);
    drop(Table::<DropMe, DefParams>::new());
    assert_eq!(DropMe::load(), 0);
  }

  #[test]
  fn drop_half_full() {
    make_drop!(DropMe);

    let table: Table<DropMe, DefParams> = Table::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(table.cap() / 2);

    for index in 0..table.cap() {
      if index & 1 == 0 {
        keys.push(table.insert(DropMe::new()).unwrap());
      } else {
        assert_ne!(table.insert(DropMe::new()), None);
      }
    }

    for key in keys {
      assert!(table.remove(key));
    }

    // Force garbage collection - this should always succeed
    // for our use-case since we are single-threaded
    DefParams::flush();

    assert_eq!(DropMe::load(), table.cap() as u32 / 2);
    assert_eq!(DropMe::load(), table.len());
    drop(table);
    assert_eq!(DropMe::load(), 0);
  }

  #[test]
  fn drop_full() {
    make_drop!(DropMe);

    let table: Table<DropMe, DefParams> = Table::new();

    for _ in 0..table.cap() {
      assert_ne!(table.insert(DropMe::new()), None);
    }

    assert_eq!(DropMe::load(), table.cap() as u32);
    assert_eq!(DropMe::load(), table.len());
    drop(table);
    assert_eq!(DropMe::load(), 0);
  }

  #[test]
  fn debug_empty() {
    let table: Table<u64, DefParams> = Table::new();
    let value: String = format!("{table:?}");

    assert_eq!(value, "{}");
  }

  #[test]
  fn debug_entry_format() {
    let table: Table<u64, DefParams> = Table::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(8);

    for index in 0..8 {
      keys.push(table.insert(index).unwrap());
    }

    let value: String = format!("{table:?}");

    for (index, key) in keys.into_iter().enumerate() {
      assert!(value.contains(&format!("{key}: {index}")));
    }
  }

  #[test]
  fn debug_weak_keys() {
    let table: Table<usize, DefParams> = Table::new();
    let debug: String = format!("{:?}", table.weak_keys(DefParams::guard()));

    assert_eq!(debug, "WeakKeys(..)");
  }

  // Scenario: The table has a single block due to min capacity.
  // Expected: Entry operations succeed.
  #[test]
  fn min_capacity_ops() {
    let table: Table<usize, MinParams> = Table::new();
    let guard: Guard<DefParams> = DefParams::guard();
    let index: Detached = table.insert(123).unwrap();

    assert!(table.exists(index, &guard));
    assert_eq!(table.read(index, &guard), Some(123));
    assert!(table.remove(index));
    refute!(table.exists(index, &guard));
  }
}
