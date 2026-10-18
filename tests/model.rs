//! Loom and shuttle model tests.

#![cfg(any(loom, shuttle))]

#[cfg(loom)]
mod driver {
  pub(crate) use ::loom::sync::Arc;
  pub(crate) use ::loom::thread;
  pub(crate) use ::loom::thread::JoinHandle;

  #[inline]
  pub(crate) fn check<F>(f: F)
  where
    F: Fn() + Sync + Send + 'static,
  {
    let mut builder: loom::model::Builder = loom::model::Builder::new();

    // Filling a table is single-threaded but still branches on every tracked
    // access, and a table spans two lines of slots. At loom's default of 1000
    // `reinsert_exists_read` fails once a line holds 32 slots (s390x).
    builder.max_branches = builder.max_branches.max(MAX_BRANCHES);
    builder.check(f);
  }

  const MAX_BRANCHES: usize = 10_000;
}

#[cfg(shuttle)]
mod driver {
  pub(crate) use ::shuttle::sync::Arc;
  pub(crate) use ::shuttle::thread;
  pub(crate) use ::shuttle::thread::JoinHandle;

  const ITERATIONS: usize = 100_000;

  #[inline]
  pub(crate) fn check<F>(f: F)
  where
    F: Fn() + Sync + Send + 'static,
  {
    shuttle::check_random(f, ITERATIONS);
  }
}

use self::driver::Arc;
use self::driver::JoinHandle;
use self::driver::thread;

use core::cell::RefCell;
use core::marker::PhantomData;
use core::mem;
use core::mem::MaybeUninit;
use core::ptr;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ptab::Capacity;
use ptab::Detached;
use ptab::PTab;
use ptab::Params;
use ptab::config::CACHE_LINE_SLOTS;
use ptab::memory::Atomic;
use ptab::memory::Collector;
use ptab::memory::CollectorWeak;
use ptab::memory::Shared;
use ptab::memory::collector::Leak;

/// At least two blocks, so keys spread across cache lines on every target.
const LENGTH: Capacity = Capacity::new(2 * CACHE_LINE_SLOTS);

const _: () = assert!(LENGTH.as_usize() >= 2 * CACHE_LINE_SLOTS);

/// The key the `n`th insert into a fresh table hands out: consecutive
/// inserts go to consecutive blocks.
const fn key(n: usize) -> Detached {
  let blocks: usize = LENGTH.as_usize() / CACHE_LINE_SLOTS;

  Detached::from_bits((n % blocks) * CACHE_LINE_SLOTS + n / blocks)
}

// -----------------------------------------------------------------------------
// Payload
// -----------------------------------------------------------------------------

/// A value whose write and reads loom checks for causality, so a reader that
/// sees an entry without happening after its publish fails the model.
#[cfg(loom)]
struct Payload(loom::cell::UnsafeCell<usize>);

#[cfg(loom)]
impl Payload {
  fn init(uninit: &mut MaybeUninit<Self>, value: usize) {
    let this: &mut Self = uninit.write(Self(loom::cell::UnsafeCell::new(0)));

    // SAFETY: the cell was created above and `&mut` excludes other access.
    this.0.with_mut(|ptr| unsafe { *ptr = value });
  }

  fn get(&self) -> usize {
    // SAFETY: the only write is in `init`, before the entry is published.
    self.0.with(|ptr| unsafe { *ptr })
  }
}

// SAFETY: the cell is written once in `init`, through `&mut`, before the entry
// is published, and only read afterwards. Loom checks that every read happens
// after that write.
#[cfg(loom)]
unsafe impl Sync for Payload {}

#[cfg(shuttle)]
struct Payload(usize);

#[cfg(shuttle)]
impl Payload {
  const fn init(uninit: &mut MaybeUninit<Self>, value: usize) {
    uninit.write(Self(value));
  }

  const fn get(&self) -> usize {
    self.0
  }
}

// -----------------------------------------------------------------------------
// Model Collector
// -----------------------------------------------------------------------------

/// A collector built on the model atomics, so loom and shuttle track every
/// data-slot access, removes included.
///
/// Its `write` is a swap, because the free-ring proof needs both data-slot
/// accesses to be RMWs. `Leak` writes with a plain store on purpose
/// and has no `remove`. Evicted entries leak.
enum ModelCollector {}

// SAFETY: test-only. Evicted entries leak, which `Collector` forbids but which
// is memory safe; the models only need `remove` to be callable.
unsafe impl Collector for ModelCollector {}

impl CollectorWeak for ModelCollector {
  type Guard = ();
  type Atomic<T> = ModelSlot<T>;

  fn guard() -> Self::Guard {}

  fn flush() {}
}

#[cfg(loom)]
type ModelPtr<T> = loom::sync::atomic::AtomicPtr<T>;

// Shuttle's `AtomicPtr` is wider than a pointer; the table needs one word.
#[cfg(shuttle)]
type ModelPtr<T> = Box<shuttle::sync::atomic::AtomicPtr<T>>;

struct ModelSlot<T>(ModelPtr<T>);

// SAFETY: the slot owns the `Box<T>` behind its pointer, so moving it moves a
// `T`, which needs `T: Send`.
unsafe impl<T: Send> Send for ModelSlot<T> {}

// SAFETY: `read` hands out `&T` to any thread, which needs `T: Sync`, and a
// `T` written on one thread may be dropped on another, which needs `T: Send`.
unsafe impl<T: Send + Sync> Sync for ModelSlot<T> {}

// SAFETY:
// - Only `write` stores a non-null pointer, from `Box::into_raw`.
// - `evict` leaks the value, so a pointer `read` returned stays live. Only
//   `clear` frees it, and its contract excludes readers.
// - `clear` nulls the slot before it frees the value.
// - The `Send` and `Sync` impls above bound `T` as the trait requires.
unsafe impl<T> Atomic<T> for ModelSlot<T> {
  type Guard = ();

  #[rustfmt::skip]
  type Shared<'guard> = ModelShared<'guard, T>
  where
    T: 'guard;

  fn null() -> Self {
    #[cfg(loom)]
    let inner: ModelPtr<T> = loom::sync::atomic::AtomicPtr::new(ptr::null_mut());
    #[cfg(shuttle)]
    let inner: ModelPtr<T> = Box::new(shuttle::sync::atomic::AtomicPtr::new(ptr::null_mut()));

    Self(inner)
  }

  fn read<'guard>(&self, order: Ordering, _guard: &'guard Self::Guard) -> Self::Shared<'guard> {
    ModelShared {
      pointer: self.0.load(order),
      phantom: PhantomData,
    }
  }

  fn write(&self, order: Ordering, init: impl FnOnce(&mut MaybeUninit<T>))
  where
    T: 'static,
  {
    let mut uninit: Box<MaybeUninit<T>> = Box::new_uninit();

    init(&mut uninit);

    // SAFETY: `init` is required to fully initialize `uninit`.
    let value: *mut T = Box::into_raw(unsafe { uninit.assume_init() });

    assert!(
      self.0.swap(value, order).is_null(),
      "Atomic<T> is occupied!"
    );
  }

  fn evict(&self, order: Ordering) -> bool {
    !self.0.swap(ptr::null_mut(), order).is_null()
  }

  unsafe fn clear(&mut self) -> bool {
    #[cfg(loom)]
    let old: *mut T = self.0.with_mut(|ptr| mem::replace(ptr, ptr::null_mut()));
    #[cfg(shuttle)]
    let old: *mut T = mem::replace(self.0.get_mut(), ptr::null_mut());

    if let Some(ptr) = NonNull::new(old) {
      // SAFETY: the pointer came from `Box::into_raw` in `write`, the slot is
      // already null so it is freed once, and the caller excludes readers.
      drop(unsafe { Box::from_raw(ptr.as_ptr()) });
      true
    } else {
      false
    }
  }
}

struct ModelShared<'guard, T> {
  pointer: *mut T,
  phantom: PhantomData<&'guard T>,
}

// SAFETY: `pointer` is null or a `Box<T>` from `write` that only `clear` frees,
// and `clear` excludes readers, so it stays live for `'guard`.
unsafe impl<'guard, T> Shared<'guard, T> for ModelShared<'guard, T> {
  fn is_null(&self) -> bool {
    self.pointer.is_null()
  }

  fn as_ref(&self) -> Option<&'guard T> {
    // SAFETY: see the impl-level comment above.
    unsafe { self.pointer.as_ref() }
  }
}

// -----------------------------------------------------------------------------
// Tables
// -----------------------------------------------------------------------------

/// Tracked data slots, removes included.
enum TrackedParams {}

impl Params for TrackedParams {
  type Collector = ModelCollector;

  const LENGTH: Capacity = LENGTH;
}

/// `Leak`'s own slot, routed through the crate's model atomics.
enum LeakParams {}

impl Params for LeakParams {
  type Collector = Leak;

  const LENGTH: Capacity = LENGTH;
}

/// `Sdd`'s slot atomics are not modelled, so only the table's are.
#[cfg(feature = "sdd")]
enum SddParams {}

#[cfg(feature = "sdd")]
impl Params for SddParams {
  type Collector = ptab::memory::collector::Sdd;

  const LENGTH: Capacity = LENGTH;
}

/// The table operations the models use, over each backend.
trait Model: Send + Sync + 'static {
  fn new() -> Self;
  fn capacity(&self) -> usize;
  fn len(&self) -> usize;
  fn insert(&self, value: usize) -> Option<Detached>;
  fn exists(&self, index: Detached) -> bool;
  fn read(&self, index: Detached) -> Option<usize>;
  fn double(&self, index: Detached) -> Option<usize>;
}

trait ModelRemove: Model {
  fn remove(&self, index: Detached) -> bool;
}

impl<P> Model for PTab<Payload, P>
where
  P: Params + 'static,
{
  fn new() -> Self {
    Self::new()
  }

  fn capacity(&self) -> usize {
    Self::capacity(self)
  }

  fn len(&self) -> usize {
    Self::len(self)
  }

  fn insert(&self, value: usize) -> Option<Detached> {
    self.write(|uninit, _| Payload::init(uninit, value))
  }

  fn exists(&self, index: Detached) -> bool {
    Self::exists(self, index)
  }

  fn read(&self, index: Detached) -> Option<usize> {
    self.with(index, Payload::get)
  }

  fn double(&self, index: Detached) -> Option<usize> {
    self.with(index, |value| value.get() * 2)
  }
}

impl<P> ModelRemove for PTab<Payload, P>
where
  P: Params + 'static,
  P::Collector: Collector,
{
  fn remove(&self, index: Detached) -> bool {
    Self::remove(self, index)
  }
}

#[cfg(feature = "sdd")]
impl Model for PTab<usize, SddParams> {
  fn new() -> Self {
    Self::new()
  }

  fn capacity(&self) -> usize {
    Self::capacity(self)
  }

  fn len(&self) -> usize {
    Self::len(self)
  }

  fn insert(&self, value: usize) -> Option<Detached> {
    Self::insert(self, value)
  }

  fn exists(&self, index: Detached) -> bool {
    Self::exists(self, index)
  }

  fn read(&self, index: Detached) -> Option<usize> {
    Self::read(self, index)
  }

  fn double(&self, index: Detached) -> Option<usize> {
    self.with(index, |value| *value * 2)
  }
}

#[cfg(feature = "sdd")]
impl ModelRemove for PTab<usize, SddParams> {
  fn remove(&self, index: Detached) -> bool {
    Self::remove(self, index)
  }
}

// -----------------------------------------------------------------------------
// Trace
// -----------------------------------------------------------------------------

struct Trace {
  inserts: TraceList<Option<Detached>>,
  lookups: TraceList<Option<usize>>,
  removes: TraceList<bool>,
  exists: TraceList<bool>,
  withs: TraceList<Option<usize>>,
}

impl Trace {
  const fn new() -> Self {
    Self {
      inserts: TraceList::new(),
      lookups: TraceList::new(),
      removes: TraceList::new(),
      exists: TraceList::new(),
      withs: TraceList::new(),
    }
  }

  fn push(&mut self, item: Action) {
    match item {
      Action::Insert(data) => self.inserts.push(data),
      Action::Lookup(data) => self.lookups.push(data),
      Action::Remove(data) => self.removes.push(data),
      Action::Exists(data) => self.exists.push(data),
      Action::With(data) => self.withs.push(data),
    }
  }
}

struct TraceList<T> {
  inner: Vec<T>,
}

impl<T> TraceList<T> {
  const fn new() -> Self {
    Self { inner: Vec::new() }
  }

  fn len(&self) -> usize {
    self.inner.len()
  }

  fn push(&mut self, item: T) {
    self.inner.push(item);
  }

  fn count(&self, count: usize, item: &T) -> bool
  where
    T: PartialEq,
  {
    let items: usize = self.inner.iter().filter(|data| item.eq(data)).count();

    items == count
  }
}

// -----------------------------------------------------------------------------
// Work Queue
// -----------------------------------------------------------------------------

enum Action {
  Insert(Option<Detached>),
  Lookup(Option<usize>),
  Remove(bool),
  Exists(bool),
  With(Option<usize>),
}

impl Action {
  fn build<F>(count: usize, f: F) -> Vec<Self>
  where
    F: Fn(usize) -> Self,
  {
    let mut this: Vec<Self> = Vec::with_capacity(count);

    for index in 0..count {
      this.push(f(index));
    }

    this
  }
}

struct WorkQueue {
  threads: RefCell<Vec<JoinHandle<Vec<Action>>>>,
}

impl WorkQueue {
  const THREADS: usize = 4;

  fn new() -> Self {
    Self {
      threads: RefCell::new(Vec::with_capacity(Self::THREADS)),
    }
  }

  fn spawn<F>(&self, f: F)
  where
    F: FnOnce() -> Vec<Action> + Send + 'static,
  {
    assert!(self.threads.borrow().len() < Self::THREADS);
    self.threads.borrow_mut().push(thread::spawn(f));
  }

  fn spawn_insert<M, F>(&self, table: Arc<M>, count: usize, f: F)
  where
    M: Model,
    F: Fn(usize) -> usize + Send + 'static,
  {
    self.spawn(move || Action::build(count, |index| Action::Insert(table.insert(f(index)))));
  }

  fn spawn_lookup<M: Model>(&self, table: Arc<M>, count: usize, index: Detached) {
    self.spawn(move || Action::build(count, |_| Action::Lookup(table.read(index))));
  }

  fn spawn_remove<M: ModelRemove>(&self, table: Arc<M>, count: usize, index: Detached) {
    self.spawn(move || Action::build(count, |_| Action::Remove(table.remove(index))))
  }

  fn spawn_exists<M: Model>(&self, table: Arc<M>, count: usize, index: Detached) {
    self.spawn(move || Action::build(count, |_| Action::Exists(table.exists(index))));
  }

  fn spawn_with<M: Model>(&self, table: Arc<M>, count: usize, index: Detached) {
    self.spawn(move || Action::build(count, |_| Action::With(table.double(index))));
  }

  fn spawn_remove_insert<M: ModelRemove>(&self, table: Arc<M>, index: Detached, value: usize) {
    self.spawn(move || {
      let removed: bool = table.remove(index);
      let inserted: Option<Detached> = table.insert(value);

      vec![Action::Remove(removed), Action::Insert(inserted)]
    });
  }

  /// Checks `index` a few times, recording `exists` then `read` each time.
  fn spawn_exists_read<M: Model>(&self, table: Arc<M>, index: Detached) {
    self.spawn(move || {
      let mut actions: Vec<Action> = Vec::with_capacity(2 * EXISTS_READS);

      for _ in 0..EXISTS_READS {
        actions.push(Action::Exists(table.exists(index)));
        actions.push(Action::Lookup(table.read(index)));
      }

      actions
    });
  }

  fn join(self) -> Trace {
    let mut trace: Trace = Trace::new();

    for handle in self.threads.borrow_mut().drain(..) {
      for action in handle.join().unwrap() {
        trace.push(action);
      }
    }

    trace
  }
}

/// Lookups per thread in `insert_lookup`. Every tracked load is a loom branch,
/// so loom runs far fewer than shuttle.
#[cfg(loom)]
const LOOKUPS: usize = 4;
#[cfg(shuttle)]
const LOOKUPS: usize = 1000;

/// `exists` and `read` pairs in the reader models. No spin and no yield: the
/// models must find the interleavings themselves.
const EXISTS_READS: usize = 2;

/// Asserts that each `exists` that saw the entry was followed by a `read` of
/// the value written, since nothing removes it.
fn assert_exists_implies_read(trace: &Trace, value: usize) {
  assert_eq!(trace.exists.len(), EXISTS_READS);
  assert_eq!(trace.lookups.len(), EXISTS_READS);

  for (exists, read) in trace.exists.inner.iter().zip(&trace.lookups.inner) {
    assert!(!exists || *read == Some(value), "exists then {read:?}");
    assert!(matches!(read, Some(found) if *found == value) || read.is_none());
  }
}

fn fill<M: Model>(table: &M) -> Vec<Detached> {
  let mut keys: Vec<Detached> = Vec::with_capacity(table.capacity());

  for index in 0..table.capacity() {
    keys.push(table.insert(index).unwrap());
  }

  keys
}

// -----------------------------------------------------------------------------
// Models
// -----------------------------------------------------------------------------

fn insert<M: Model>() {
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();

    assert_eq!(table.len(), 0);

    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 4);
    assert_eq!(trace.inserts.len(), 4);
    assert_eq!(trace.lookups.len(), 0);
    assert_eq!(trace.removes.len(), 0);

    for n in 0..4 {
      assert!(trace.inserts.count(1, &Some(key(n))));
      assert_eq!(table.read(key(n)), Some(0));
    }

    // The keys span more than one block.
    assert!(key(1).into_bits() >= CACHE_LINE_SLOTS);
  });
}

fn insert_sequential<M: Model>() {
  // Scenario: one thread inserts twice in a row while another inserts once.
  // Expected: `len` counts all three entries and each key is handed out once.
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();

    queue.spawn_insert(Arc::clone(&table), 2, |index| index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 3);
    assert_eq!(trace.inserts.len(), 3);

    for n in 0..3 {
      assert!(trace.inserts.count(1, &Some(key(n))));
    }
  });
}

fn lookup<M: Model>() {
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.len(), 1);

    queue.spawn_lookup(Arc::clone(&table), 1, index);
    queue.spawn_lookup(Arc::clone(&table), 1, Detached::from_bits(usize::MAX));
    queue.spawn_lookup(Arc::clone(&table), 1, index);
    queue.spawn_lookup(Arc::clone(&table), 1, Detached::from_bits(usize::MAX));

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 1);
    assert_eq!(trace.inserts.len(), 0);
    assert_eq!(trace.lookups.len(), 4);
    assert_eq!(trace.removes.len(), 0);

    assert!(trace.lookups.count(2, &Some(123)));
    assert!(trace.lookups.count(2, &None));

    assert_eq!(table.read(index), Some(123));
  });
}

fn remove<M: ModelRemove>() {
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.len(), 1);

    queue.spawn_remove(Arc::clone(&table), 1, index);
    queue.spawn_remove(Arc::clone(&table), 1, index);
    queue.spawn_remove(Arc::clone(&table), 1, index);
    queue.spawn_remove(Arc::clone(&table), 1, index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 0);
    assert_eq!(trace.inserts.len(), 0);
    assert_eq!(trace.lookups.len(), 0);
    assert_eq!(trace.removes.len(), 4);

    assert!(trace.removes.count(1, &true));
    assert!(trace.removes.count(3, &false));

    assert_eq!(table.read(index), None);
  });
}

fn insert_lookup<M: Model>() {
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.len(), 1);

    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_lookup(Arc::clone(&table), LOOKUPS, index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_lookup(Arc::clone(&table), LOOKUPS, index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 3);
    assert_eq!(trace.inserts.len(), 2);
    assert_eq!(trace.lookups.len(), 2 * LOOKUPS);
    assert_eq!(trace.removes.len(), 0);

    assert!(trace.inserts.count(1, &Some(key(1))));
    assert!(trace.inserts.count(1, &Some(key(2))));
    assert!(trace.lookups.count(2 * LOOKUPS, &Some(123)));

    assert_eq!(table.read(index), Some(123));
  });
}

fn insert_remove<M: ModelRemove>() {
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.len(), 1);

    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_remove(Arc::clone(&table), 1, index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_remove(Arc::clone(&table), 1, index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 2);
    assert_eq!(trace.inserts.len(), 2);
    assert_eq!(trace.lookups.len(), 0);
    assert_eq!(trace.removes.len(), 2);

    assert!(trace.inserts.count(1, &Some(key(1))));
    assert!(trace.inserts.count(1, &Some(key(2))));
    assert!(trace.removes.count(1, &true));
    assert!(trace.removes.count(1, &false));

    assert_eq!(table.read(index), None);
  });
}

fn lookup_remove<M: ModelRemove>() {
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.len(), 1);

    queue.spawn_lookup(Arc::clone(&table), 1, index);
    queue.spawn_remove(Arc::clone(&table), 1, index);
    queue.spawn_lookup(Arc::clone(&table), 1, index);
    queue.spawn_remove(Arc::clone(&table), 1, index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 0);
    assert_eq!(trace.inserts.len(), 0);
    assert_eq!(trace.lookups.len(), 2);
    assert_eq!(trace.removes.len(), 2);

    assert!(matches!(trace.lookups.inner[0], Some(123) | None));
    assert!(matches!(trace.lookups.inner[1], Some(123) | None));
    assert!(trace.removes.count(1, &true));
    assert!(trace.removes.count(1, &false));

    assert_eq!(table.read(index), None);
  });
}

fn insert_lookup_remove<M: ModelRemove>() {
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.len(), 1);

    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_lookup(Arc::clone(&table), 1, index);
    queue.spawn_remove(Arc::clone(&table), 1, index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 1);
    assert_eq!(trace.inserts.len(), 1);
    assert_eq!(trace.lookups.len(), 1);
    assert_eq!(trace.removes.len(), 1);

    assert!(trace.inserts.count(1, &Some(key(1))));
    assert!(matches!(trace.lookups.inner[0], Some(123) | None));
    assert!(trace.removes.count(1, &true));

    assert_eq!(table.read(index), None);
  });
}

fn capacity_race<M: Model>() {
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();

    for index in 0..table.capacity() - 1 {
      let _index: Detached = table.insert(index).unwrap();
    }

    assert_eq!(table.len(), table.capacity() - 1);

    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), table.capacity());
    assert_eq!(trace.inserts.len(), 4);
    assert_eq!(trace.lookups.len(), 0);
    assert_eq!(trace.removes.len(), 0);

    assert!(trace.inserts.count(1, &Some(key(table.capacity() - 1))));
    assert!(trace.inserts.count(3, &None));
  });
}

fn exists_remove<M: ModelRemove>() {
  // Scenario: two threads check a key while two others remove it.
  // Expected: exactly one remove succeeds, each check sees either state, and
  // the key is gone afterwards.
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(123).unwrap();

    queue.spawn_exists(Arc::clone(&table), 1, index);
    queue.spawn_remove(Arc::clone(&table), 1, index);
    queue.spawn_exists(Arc::clone(&table), 1, index);
    queue.spawn_remove(Arc::clone(&table), 1, index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 0);
    assert_eq!(trace.exists.len(), 2);
    assert_eq!(trace.removes.len(), 2);

    assert!(trace.removes.count(1, &true));
    assert!(trace.removes.count(1, &false));

    assert!(!table.exists(index));
  });
}

fn with_remove<M: ModelRemove>() {
  // Scenario: two threads run a closure on a value while another removes it.
  // Expected: a closure sees the whole value or nothing, never anything else.
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(123).unwrap();

    queue.spawn_with(Arc::clone(&table), 1, index);
    queue.spawn_remove(Arc::clone(&table), 1, index);
    queue.spawn_with(Arc::clone(&table), 1, index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 0);
    assert_eq!(trace.withs.len(), 2);
    assert_eq!(trace.removes.len(), 1);

    assert!(trace.removes.count(1, &true));
    assert!(matches!(trace.withs.inner[0], Some(246) | None));
    assert!(matches!(trace.withs.inner[1], Some(246) | None));

    assert_eq!(table.read(index), None);
  });
}

fn read_unaffected_by_other_remove<M: ModelRemove>() {
  // Scenario: one thread reads a key while another removes a different key.
  // Expected: the read always sees its own value.
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let index_a: Detached = table.insert(111).unwrap();
    let index_b: Detached = table.insert(222).unwrap();

    queue.spawn_lookup(Arc::clone(&table), 1, index_b);
    queue.spawn_remove(Arc::clone(&table), 1, index_a);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 1);
    assert_eq!(trace.lookups.len(), 1);
    assert_eq!(trace.removes.len(), 1);

    assert!(trace.lookups.count(1, &Some(222)));
    assert!(trace.removes.count(1, &true));
  });
}

fn remove_and_reinsert<M: ModelRemove>() {
  // Scenario: a full table has one key removed while another thread inserts.
  // Expected: the insert either misses the free slot or gets a fresh key that
  // reads back its value.
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let keys: Vec<Detached> = fill(&*table);

    assert!(table.insert(5).is_none());

    queue.spawn_remove(Arc::clone(&table), 1, keys[0]);
    queue.spawn_insert(Arc::clone(&table), 1, |_| 10);

    let trace: Trace = queue.join();

    assert_eq!(trace.inserts.len(), 1);
    assert_eq!(trace.removes.len(), 1);
    assert!(trace.removes.count(1, &true));

    if let Some(index) = trace.inserts.inner[0] {
      assert!(!keys.contains(&index));
      assert_eq!(table.read(index), Some(10));
      assert_eq!(table.len(), table.capacity());
    } else {
      assert_eq!(table.len(), table.capacity() - 1);
    }
  });
}

fn concurrent_remove_reinsert<M: ModelRemove>() {
  // Scenario: two threads each remove a key from a full table, then insert.
  // Expected: both inserts may succeed, with distinct keys and intact values.
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let keys: Vec<Detached> = fill(&*table);

    queue.spawn_remove_insert(Arc::clone(&table), keys[0], 100);
    queue.spawn_remove_insert(Arc::clone(&table), keys[1], 200);

    let trace: Trace = queue.join();

    assert_eq!(trace.inserts.len(), 2);
    assert_eq!(trace.removes.len(), 2);
    assert!(trace.removes.count(2, &true));

    let new_a: Detached = trace.inserts.inner[0].unwrap();
    let new_b: Detached = trace.inserts.inner[1].unwrap();

    assert_ne!(new_a, new_b);
    assert_eq!(table.len(), table.capacity());

    let mut values: Vec<usize> = Vec::with_capacity(2);

    values.push(table.read(new_a).unwrap());
    values.push(table.read(new_b).unwrap());
    values.sort_unstable();

    assert_eq!(values, [100, 200]);
  });
}

fn insert_exists_read<M: Model>() {
  // Scenario: one thread inserts into an empty table while another checks the
  // key it will get, then reads it.
  // Expected: whenever `exists` saw the entry, the read sees the value written.
  driver::check(|| {
    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();

    queue.spawn_insert(Arc::clone(&table), 1, |_| 7);
    queue.spawn_exists_read(Arc::clone(&table), key(0));

    let trace: Trace = queue.join();

    assert!(trace.inserts.count(1, &Some(key(0))));
    assert_exists_implies_read(&trace, 7);
    assert_eq!(table.read(key(0)), Some(7));
  });
}

fn reinsert_exists_read<M: ModelRemove>() {
  // Scenario: a full table has one key removed, then one thread inserts into
  // the freed slot while another checks the key it will get, then reads it.
  // Expected: whenever `exists` saw the entry, the read sees the value written.
  driver::check(|| {
    // The same steps on a probe table give the reused key, since key
    // assignment is deterministic without concurrency.
    let probe: M = M::new();
    let probe_keys: Vec<Detached> = fill(&probe);

    assert!(probe.remove(probe_keys[0]));

    let reused: Detached = probe.insert(9).unwrap();

    drop(probe);

    let table: Arc<M> = Arc::new(M::new());
    let queue: WorkQueue = WorkQueue::new();
    let keys: Vec<Detached> = fill(&*table);

    assert!(table.remove(keys[0]));
    assert!(!keys.contains(&reused));

    queue.spawn_insert(Arc::clone(&table), 1, |_| 9);
    queue.spawn_exists_read(Arc::clone(&table), reused);

    let trace: Trace = queue.join();

    assert!(trace.inserts.count(1, &Some(reused)));
    assert_exists_implies_read(&trace, 9);
    assert_eq!(table.read(reused), Some(9));
  });
}

// -----------------------------------------------------------------------------
// Backends
// -----------------------------------------------------------------------------

macro_rules! models {
  ($table:ty => $($model:ident),* $(,)?) => {
    $(
      #[test]
      fn $model() {
        super::$model::<$table>();
      }
    )*
  };
}

/// Every model on the model collector: each data-slot access is tracked.
mod tracked {
  use super::PTab;
  use super::Payload;
  use super::TrackedParams;

  models!(PTab<Payload, TrackedParams> =>
    insert,
    insert_sequential,
    lookup,
    remove,
    insert_lookup,
    insert_remove,
    lookup_remove,
    insert_lookup_remove,
    capacity_race,
    exists_remove,
    with_remove,
    read_unaffected_by_other_remove,
    remove_and_reinsert,
    concurrent_remove_reinsert,
    insert_exists_read,
    reinsert_exists_read,
  );
}

/// The insert and lookup models on `Leak`'s own slot. `Leak` has no `remove`.
mod leak {
  use super::LeakParams;
  use super::PTab;
  use super::Payload;

  models!(PTab<Payload, LeakParams> =>
    insert,
    insert_sequential,
    lookup,
    insert_lookup,
    capacity_race,
    insert_exists_read,
  );
}

/// The remove models on `Sdd`. Its slot atomics are untracked, so these only
/// check the table's own atomics.
#[cfg(feature = "sdd")]
mod sdd {
  use super::PTab;
  use super::SddParams;

  models!(PTab<usize, SddParams> =>
    remove,
    insert_remove,
    lookup_remove,
    exists_remove,
    with_remove,
    concurrent_remove_reinsert,
  );
}
