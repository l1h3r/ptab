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

    // The tracked data slots add branches; loom's default of 1_000 is too low.
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

use ptab::Capacity;
use ptab::Detached;

use ptab::config::Params;
use ptab::memory::Collector;

use self::collector::ModelCollector;

type PTab<P> = ptab::PTab<usize, P>;

/// A table configuration the models run on.
trait Model: Params<Collector: Collector> + 'static {}

impl<P> Model for P where P: Params<Collector: Collector> + 'static {}

/// The real reclaimer. Loom and shuttle don't see its atomics, so only the
/// remove models run on it.
type SddParams = ptab::ConstParams<{ Capacity::MIN.as_usize() }>;

/// Data slots that loom and shuttle track.
enum TrackedParams {}

impl Params for TrackedParams {
  type Collector = ModelCollector;

  const LENGTH: Capacity = Capacity::MIN;
}

/// The lookups each lookup thread does in `insert_lookup`.
const LOOKUPS: usize = 10;

// -----------------------------------------------------------------------------
// Model Collector
// -----------------------------------------------------------------------------

mod collector {
  use core::marker::PhantomData;
  use core::mem::MaybeUninit;
  use core::ptr;
  use core::sync::atomic::Ordering;

  use ptab::memory::Atomic;
  use ptab::memory::Collector;
  use ptab::memory::CollectorWeak;
  use ptab::memory::Shared;

  #[cfg(loom)]
  type Inner<T> = ::loom::sync::atomic::AtomicPtr<T>;

  // Shuttle's `AtomicPtr` is wider than a pointer; the table's layout needs
  // one pointer per slot, so box it.
  #[cfg(shuttle)]
  type Inner<T> = Box<::shuttle::sync::atomic::AtomicPtr<T>>;

  /// A collector whose data slots are model atomics. It leaks evicted
  /// entries.
  pub(crate) enum ModelCollector {}

  /// The guard of [`ModelCollector`]; it pins nothing.
  pub(crate) struct ModelGuard;

  impl CollectorWeak for ModelCollector {
    type Guard = ModelGuard;
    type Atomic<T> = ModelAtomic<T>;

    fn guard() -> Self::Guard {
      ModelGuard
    }

    fn flush() {
      // nothing is retired for reclamation
    }
  }

  // SAFETY:
  // - This breaks the eventual-reclamation clause of `Collector`: `retire`
  //   leaks every evicted entry. It is allowed in these tests only, for now.
  // - Memory safety doesn't depend on that clause: nothing is freed early, so
  //   a pointer read under a guard stays valid.
  unsafe impl Collector for ModelCollector {}

  /// Hands an evicted entry to the collector. It leaks the entry.
  fn retire<T>(pointer: *mut T) {
    let _leaked: *mut T = pointer;
  }

  /// A data slot backed by a loom or shuttle `AtomicPtr`.
  ///
  /// Every method passes the caller's ordering through unchanged, so the
  /// models see the orderings the table really uses.
  #[repr(transparent)]
  pub(crate) struct ModelAtomic<T> {
    inner: Inner<T>,
  }

  impl<T> ModelAtomic<T> {
    #[cfg(loom)]
    fn take(&mut self) -> *mut T {
      self.inner.with_mut(|pointer| *pointer)
    }

    #[cfg(shuttle)]
    fn take(&mut self) -> *mut T {
      *self.inner.get_mut()
    }
  }

  // SAFETY:
  // - `read` returns null or a pointer from `Box::into_raw` in `write`. `evict`
  //   leaks what it removes, so the pointer stays valid for the whole test.
  // - `Inner` is a pointer-sized handle: a boxed shuttle `AtomicPtr`, or loom's
  //   `AtomicPtr`, which the `PTab::new` size check covers. Dropping the handle
  //   frees only the handle, never a `T`.
  // - Every method is a loom or shuttle atomic operation, which is sound from
  //   several threads at once for any `T`.
  // - `evict` hands the removed entry to `retire`, which leaks it and never
  //   drops it, so no guard sees a dropped value.
  // - The handle adds no thread affinity, so moving it to another thread is
  //   sound when `T: Send`.
  unsafe impl<T> Atomic<T> for ModelAtomic<T> {
    type Guard = ModelGuard;

    type Shared<'guard>
      = ModelShared<'guard, T>
    where
      T: 'guard;

    fn null() -> Self {
      #[cfg(loom)]
      let inner: Inner<T> = Inner::new(ptr::null_mut());

      #[cfg(shuttle)]
      let inner: Inner<T> = Box::new(::shuttle::sync::atomic::AtomicPtr::new(ptr::null_mut()));

      Self { inner }
    }

    fn read<'guard>(&self, order: Ordering, _guard: &'guard Self::Guard) -> Self::Shared<'guard> {
      ModelShared {
        pointer: self.inner.load(order),
        phantom: PhantomData,
      }
    }

    fn write(&self, order: Ordering, init: impl FnOnce(&mut MaybeUninit<T>))
    where
      T: 'static,
    {
      let mut uninit: Box<MaybeUninit<T>> = Box::new_uninit();

      init(&mut uninit);

      // SAFETY: the `init` closure is required to fully initialize `uninit`.
      let old: *mut T = self
        .inner
        .swap(Box::into_raw(unsafe { uninit.assume_init() }), order);

      assert!(old.is_null(), "ModelAtomic is occupied!");
    }

    fn evict(&self, order: Ordering) -> bool {
      let pointer: *mut T = self.inner.swap(ptr::null_mut(), order);

      if pointer.is_null() {
        false
      } else {
        retire(pointer);
        true
      }
    }

    unsafe fn clear(&mut self) -> bool {
      let pointer: *mut T = self.take();

      if pointer.is_null() {
        false
      } else {
        // SAFETY:
        // - `pointer` came from `Box::into_raw` in `write`.
        // - `&mut self` gives exclusive access, so the box is freed once.
        drop(unsafe { Box::from_raw(pointer) });
        true
      }
    }
  }

  /// A pointer read from a [`ModelAtomic`].
  pub(crate) struct ModelShared<'guard, T> {
    pointer: *mut T,
    phantom: PhantomData<&'guard T>,
  }

  // SAFETY:
  // - `as_ref` returns a reference only to a non-null pointer, which is the
  //   `Box::into_raw` value of a live `T` (see the `Atomic` impl).
  // - `as_ref` returns `None` exactly when `is_null` is `true`, because
  //   `<*mut T>::as_ref` does.
  unsafe impl<'guard, T> Shared<'guard, T> for ModelShared<'guard, T> {
    fn is_null(&self) -> bool {
      self.pointer.is_null()
    }

    fn as_ref(&self) -> Option<&'guard T> {
      // SAFETY:
      // - `pointer` is null or points to a `T` written by `write`.
      // - Nothing is freed while the table is shared, so it stays valid.
      unsafe { self.pointer.as_ref() }
    }
  }
}

// -----------------------------------------------------------------------------
// Suites
// -----------------------------------------------------------------------------

macro_rules! suite {
  ($suite:ident, $params:ty, [$($model:ident),* $(,)?]) => {
    mod $suite {
      $(
        #[test]
        fn $model() {
          super::$model::<$params>();
        }
      )*
    }
  };
}

suite!(
  tracked,
  super::TrackedParams,
  [
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
    remove_distinct,
    lookup_remove_multi,
    insert_three_way,
    length_insert_remove,
    insert_exists_read,
  ]
);

suite!(
  sdd,
  super::SddParams,
  [
    remove,
    insert_remove,
    lookup_remove,
    insert_lookup_remove,
    exists_remove,
    with_remove,
    read_unaffected_by_other_remove,
    remove_and_reinsert,
    concurrent_remove_reinsert,
    remove_distinct,
    lookup_remove_multi,
    length_insert_remove,
  ]
);

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

  fn spawn_insert<P: Model, F>(&self, table: Arc<PTab<P>>, count: usize, f: F)
  where
    F: Fn(usize) -> usize + Send + 'static,
  {
    self.spawn(move || Action::build(count, |index| Action::Insert(table.insert(f(index)))));
  }

  fn spawn_lookup<P: Model>(&self, table: Arc<PTab<P>>, count: usize, index: Detached) {
    self.spawn(move || Action::build(count, |_| Action::Lookup(table.read(index))));
  }

  fn spawn_remove<P: Model>(&self, table: Arc<PTab<P>>, count: usize, index: Detached) {
    self.spawn(move || Action::build(count, |_| Action::Remove(table.remove(index))))
  }

  fn spawn_exists<P: Model>(&self, table: Arc<PTab<P>>, count: usize, index: Detached) {
    self.spawn(move || Action::build(count, |_| Action::Exists(table.exists(index))));
  }

  fn spawn_with<P: Model>(&self, table: Arc<PTab<P>>, count: usize, index: Detached) {
    self.spawn(move || {
      Action::build(count, |_| {
        Action::With(table.with(index, |value| *value * 2))
      })
    });
  }

  fn spawn_exists_read<P: Model>(&self, table: Arc<PTab<P>>, index: Detached) {
    self.spawn(move || {
      let exists: bool = table.exists(index);
      let value: Option<usize> = table.read(index);

      vec![Action::Exists(exists), Action::Lookup(value)]
    });
  }

  fn spawn_remove_insert<P: Model>(&self, table: Arc<PTab<P>>, index: Detached, value: usize) {
    self.spawn(move || {
      let removed: bool = table.remove(index);
      let inserted: Option<Detached> = table.insert(value);

      vec![Action::Remove(removed), Action::Insert(inserted)]
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

// -----------------------------------------------------------------------------
// Actual Tests
// -----------------------------------------------------------------------------

fn insert<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

    assert!(trace.inserts.count(1, &Some(Detached::from_bits(0))));
    assert!(trace.inserts.count(1, &Some(Detached::from_bits(1))));
    assert!(trace.inserts.count(1, &Some(Detached::from_bits(2))));
    assert!(trace.inserts.count(1, &Some(Detached::from_bits(3))));

    assert_eq!(table.read(Detached::from_bits(0)), Some(0));
    assert_eq!(table.read(Detached::from_bits(1)), Some(0));
    assert_eq!(table.read(Detached::from_bits(2)), Some(0));
    assert_eq!(table.read(Detached::from_bits(3)), Some(0));
  });
}

fn insert_sequential<P: Model>() {
  // Scenario: one thread inserts twice in a row while another inserts once.
  // Expected: `len` counts all three entries and each key is handed out once.
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
    let queue: WorkQueue = WorkQueue::new();

    queue.spawn_insert(Arc::clone(&table), 2, |index| index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 3);
    assert_eq!(trace.inserts.len(), 3);

    assert!(trace.inserts.count(1, &Some(Detached::from_bits(0))));
    assert!(trace.inserts.count(1, &Some(Detached::from_bits(1))));
    assert!(trace.inserts.count(1, &Some(Detached::from_bits(2))));
  });
}

fn lookup<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

fn remove<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

fn insert_lookup<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

    assert!(trace.inserts.count(1, &Some(Detached::from_bits(1))));
    assert!(trace.inserts.count(1, &Some(Detached::from_bits(2))));
    assert!(trace.lookups.count(2 * LOOKUPS, &Some(123)));

    assert_eq!(table.read(index), Some(123));
  });
}

fn insert_remove<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

    assert!(trace.inserts.count(1, &Some(Detached::from_bits(1))));
    assert!(trace.inserts.count(1, &Some(Detached::from_bits(2))));
    assert!(trace.removes.count(1, &true));
    assert!(trace.removes.count(1, &false));

    assert_eq!(table.read(index), None);
  });
}

fn lookup_remove<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

fn insert_lookup_remove<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

    assert!(trace.inserts.count(1, &Some(Detached::from_bits(1))));
    assert!(matches!(trace.lookups.inner[0], Some(123) | None));
    assert!(trace.removes.count(1, &true));

    assert_eq!(table.read(index), None);
  });
}

fn capacity_race<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

    assert!(
      trace
        .inserts
        .count(1, &Some(Detached::from_bits(table.capacity() - 1)))
    );
    assert!(trace.inserts.count(3, &None));
  });
}

fn exists_remove<P: Model>() {
  // Scenario: two threads check a key while two others remove it.
  // Expected: exactly one remove succeeds, each check sees either state, and
  // the key is gone afterwards.
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

fn with_remove<P: Model>() {
  // Scenario: two threads run a closure on a value while another removes it.
  // Expected: a closure sees the whole value or nothing, never anything else.
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

fn read_unaffected_by_other_remove<P: Model>() {
  // Scenario: one thread reads a key while another removes a different key.
  // Expected: the read always sees its own value.
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
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

fn remove_and_reinsert<P: Model>() {
  // Scenario: a full table has one key removed while another thread inserts.
  // Expected: the insert either misses the free slot or gets a fresh key that
  // reads back its value.
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
    let queue: WorkQueue = WorkQueue::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(table.capacity());

    for index in 0..table.capacity() {
      keys.push(table.insert(index).unwrap());
    }

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

fn concurrent_remove_reinsert<P: Model>() {
  // Scenario: two threads each remove a key from a full table, then insert.
  // Expected: both inserts may succeed, with distinct keys and intact values.
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
    let queue: WorkQueue = WorkQueue::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(table.capacity());

    for index in 0..table.capacity() {
      keys.push(table.insert(index).unwrap());
    }

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

fn remove_distinct<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
    let queue: WorkQueue = WorkQueue::new();
    let index_a: Detached = table.insert(1).unwrap();
    let index_b: Detached = table.insert(2).unwrap();

    queue.spawn_remove(Arc::clone(&table), 1, index_a);
    queue.spawn_remove(Arc::clone(&table), 1, index_b);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 0);
    assert_eq!(trace.removes.len(), 2);
    assert!(trace.removes.count(2, &true));

    assert!(!table.exists(index_a));
    assert!(!table.exists(index_b));
  });
}

fn lookup_remove_multi<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(123).unwrap();

    queue.spawn_lookup(Arc::clone(&table), 1, index);
    queue.spawn_lookup(Arc::clone(&table), 1, index);
    queue.spawn_remove(Arc::clone(&table), 1, index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 0);
    assert_eq!(trace.lookups.len(), 2);
    assert_eq!(trace.removes.len(), 1);

    assert!(trace.removes.count(1, &true));
    assert!(matches!(trace.lookups.inner[0], Some(123) | None));
    assert!(matches!(trace.lookups.inner[1], Some(123) | None));

    assert_eq!(table.read(index), None);
  });
}

fn insert_three_way<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
    let queue: WorkQueue = WorkQueue::new();

    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 3);
    assert_eq!(trace.inserts.len(), 3);

    assert!(trace.inserts.count(1, &Some(Detached::from_bits(0))));
    assert!(trace.inserts.count(1, &Some(Detached::from_bits(1))));
    assert!(trace.inserts.count(1, &Some(Detached::from_bits(2))));
  });
}

fn length_insert_remove<P: Model>() {
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(1).unwrap();

    queue.spawn_insert(Arc::clone(&table), 1, |_| 2);
    queue.spawn_remove(Arc::clone(&table), 1, index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 1);
    assert_eq!(trace.inserts.len(), 1);
    assert_eq!(trace.removes.len(), 1);

    assert!(trace.removes.count(1, &true));
  });
}

fn insert_exists_read<P: Model>() {
  // Scenario: one thread inserts while another checks the new key, then reads
  // it.
  // Expected: once the check sees the key, the read returns its value.
  driver::check(|| {
    let table: Arc<PTab<P>> = Arc::new(PTab::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = Detached::from_bits(0);

    queue.spawn_insert(Arc::clone(&table), 1, |_| 123);
    queue.spawn_exists_read(Arc::clone(&table), index);

    let trace: Trace = queue.join();

    assert!(trace.inserts.count(1, &Some(index)));

    let exists: bool = trace.exists.inner[0];
    let value: Option<usize> = trace.lookups.inner[0];

    assert!(!exists || value == Some(123));
  });
}
