//! Core table implementation.
//!
//! Uses lock-free operations with epoch-based reclamation via [`sdd`].
//!
//! [`sdd`]: https://docs.rs/sdd

use core::fmt::Debug;
use core::fmt::DebugMap;
use core::fmt::Formatter;
use core::fmt::Result as FmtResult;
use core::hint;
use core::marker::PhantomData;
use core::mem;
use core::mem::MaybeUninit;

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
  #[track_caller]
  #[inline]
  pub(crate) fn new() -> Self {
    Self {
      volatile: CachePadded::new(Volatile::new()),
      readonly: CachePadded::new(ReadOnly::new()),
    }
  }

  #[track_caller]
  #[inline]
  pub(crate) const fn cap(&self) -> usize {
    P::LENGTH.as_usize()
  }

  #[track_caller]
  #[inline]
  pub(crate) fn len(&self) -> u32 {
    self.volatile.entries.load(Relaxed)
  }

  #[track_caller]
  #[inline]
  pub(crate) fn is_empty(&self) -> bool {
    self.len() == 0
  }

  #[track_caller]
  #[inline]
  pub(crate) fn insert(&self, value: T) -> Option<Detached>
  where
    T: 'static,
  {
    self.write(|entry, _| {
      entry.write(value);
    })
  }

  #[track_caller]
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

  #[track_caller]
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

  #[track_caller]
  #[inline]
  pub(crate) fn exists(&self, key: Detached) -> bool {
    let guard: Guard = Guard::new();
    let value: Ptr<'_, T> = self.entry(key, &guard);

    !value.is_null()
  }

  #[track_caller]
  #[inline]
  pub(crate) fn with<F, R>(&self, key: Detached, f: F) -> Option<R>
  where
    F: Fn(&T) -> R,
  {
    let guard: Guard = Guard::new();
    let value: Ptr<'_, T> = self.entry(key, &guard);

    // SAFETY: Tag bits are never set on data pointers.
    match unsafe { value.as_ref_unchecked() } {
      Some(data) => Some(f(data)),
      None => None,
    }
  }

  #[track_caller]
  #[inline]
  pub(crate) fn read(&self, key: Detached) -> Option<T>
  where
    T: Copy,
  {
    let guard: Guard = Guard::new();
    let value: Ptr<'_, T> = self.entry(key, &guard);

    // SAFETY: No tag bits are set on these pointers.
    match unsafe { value.as_ref_unchecked() } {
      Some(data) => Some(*data),
      None => None,
    }
  }

  #[inline]
  fn entry<'guard>(&self, key: Detached, guard: &'guard Guard) -> Ptr<'guard, T> {
    self
      .readonly
      .data
      .get(Concrete::from_detached(key))
      .load(Acquire, guard)
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
  fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
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
      let block: usize = offset % P::BLOCKS.get();
      let index: usize = offset / P::BLOCKS.get();
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
