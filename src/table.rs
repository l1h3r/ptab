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

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(not(any(loom, shuttle)))]
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(test)]
mod tests;

// -----------------------------------------------------------------------------
// Implementation
// -----------------------------------------------------------------------------

/// Marks a free-list entry whose index has been taken.
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

    // A failed `reserve_slot` holds `entries` above `max` until it undoes its
    // increment.
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
  pub(crate) fn remove(&self, key: Detached) -> bool
  where
    T: Send,
  {
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

    self.undo_reservation(prev.wrapping_add(1))
  }

  #[cold]
  #[inline(never)]
  fn undo_reservation(&self, mut current: u32) -> Option<Permit<'_, T, P>> {
    while let Err(next) = self.volatile.swap_entries(current, current.wrapping_sub(1)) {
      if next <= P::LENGTH.as_u32() {
        return Some(Permit::new(self));
      }

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
      .compare_exchange(RESERVED, data, Relaxed, Relaxed)
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
// - `Table` may transfer ownership of entries across threads.
// - This is only sound if `T: Send`.
unsafe impl<T, P> Send for Table<T, P>
where
  T: Send,
  P: Params + ?Sized,
{
}

// SAFETY:
// - `Table` provides shared access to entries from multiple threads.
// - This is only sound if `T: Sync`, as multiple shared references may be held.
// - `remove` and `Drop` drop a value on a thread other than the one that
//   inserted it, which needs `T: Send`.
unsafe impl<T, P> Sync for Table<T, P>
where
  T: Send + Sync,
  P: Params + ?Sized,
{
}

// Unwind safe only when `T: RefUnwindSafe` since `with` and `read` expose `&T`.
impl<T, P> RefUnwindSafe for Table<T, P>
where
  T: RefUnwindSafe,
  P: Params + ?Sized,
{
}

impl<T, P> UnwindSafe for Table<T, P>
where
  T: RefUnwindSafe,
  P: Params + ?Sized,
{
}

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
    // At `Capacity::MAX` one slot stays reserved.
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
      .compare_exchange(current, updated, Release, Relaxed)
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

/// An iterator over the indices of entries in a [`PTab`].
///
/// # Consistency
///
/// Entries inserted or removed during iteration may or may not be yielded.
/// Each yielded index named an entry at some point during iteration.
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
      total: P::LENGTH.as_usize(),
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
