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
    loom::model(f);
  }
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

use ptab::Detached;

type PTab = ptab::PTab<usize, ptab::ConstParams<{ 1 << 4 }>>;

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

  fn spawn_insert<F>(&self, table: Arc<PTab>, count: usize, f: F)
  where
    F: Fn(usize) -> usize + Send + 'static,
  {
    self.spawn(move || Action::build(count, |index| Action::Insert(table.insert(f(index)))));
  }

  fn spawn_lookup(&self, table: Arc<PTab>, count: usize, index: Detached) {
    self.spawn(move || Action::build(count, |_| Action::Lookup(table.read(index))));
  }

  fn spawn_remove(&self, table: Arc<PTab>, count: usize, index: Detached) {
    self.spawn(move || Action::build(count, |_| Action::Remove(table.remove(index))))
  }

  fn spawn_exists(&self, table: Arc<PTab>, count: usize, index: Detached) {
    self.spawn(move || Action::build(count, |_| Action::Exists(table.exists(index))));
  }

  fn spawn_with(&self, table: Arc<PTab>, count: usize, index: Detached) {
    self.spawn(move || {
      Action::build(count, |_| {
        Action::With(table.with(index, |value| *value * 2))
      })
    });
  }

  fn spawn_remove_insert(&self, table: Arc<PTab>, index: Detached, value: usize) {
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

#[test]
fn insert() {
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn lookup() {
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn remove() {
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn insert_lookup() {
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
    let queue: WorkQueue = WorkQueue::new();
    let index: Detached = table.insert(123).unwrap();

    assert_eq!(table.len(), 1);

    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_lookup(Arc::clone(&table), 1000, index);
    queue.spawn_insert(Arc::clone(&table), 1, |index| index);
    queue.spawn_lookup(Arc::clone(&table), 1000, index);

    let trace: Trace = queue.join();

    assert_eq!(table.len(), 3);
    assert_eq!(trace.inserts.len(), 2);
    assert_eq!(trace.lookups.len(), 2000);
    assert_eq!(trace.removes.len(), 0);

    assert!(trace.inserts.count(1, &Some(Detached::from_bits(1))));
    assert!(trace.inserts.count(1, &Some(Detached::from_bits(2))));
    assert!(trace.lookups.count(2000, &Some(123)));

    assert_eq!(table.read(index), Some(123));
  });
}

#[test]
fn insert_remove() {
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn lookup_remove() {
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn insert_lookup_remove() {
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn capacity_race() {
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn exists_remove() {
  // Scenario: two threads check a key while two others remove it.
  // Expected: exactly one remove succeeds, each check sees either state, and
  // the key is gone afterwards.
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn with_remove() {
  // Scenario: two threads run a closure on a value while another removes it.
  // Expected: a closure sees the whole value or nothing, never anything else.
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn read_unaffected_by_other_remove() {
  // Scenario: one thread reads a key while another removes a different key.
  // Expected: the read always sees its own value.
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn remove_and_reinsert() {
  // Scenario: a full table has one key removed while another thread inserts.
  // Expected: the insert either misses the free slot or gets a fresh key that
  // reads back its value.
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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

#[test]
fn concurrent_remove_reinsert() {
  // Scenario: two threads each remove a key from a full table, then insert.
  // Expected: both inserts may succeed, with distinct keys and intact values.
  driver::check(|| {
    let table: Arc<PTab> = Arc::new(PTab::new());
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
