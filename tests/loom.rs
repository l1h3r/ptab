#![cfg(loom)]

use loom::sync::Arc;
use loom::thread;
use loom::thread::JoinHandle;
use std::ops::Deref;

use ptab::Capacity;
use ptab::ConstParams;
use ptab::Detached;
use ptab::PTab;

type Insert = JoinHandle<Option<Detached>>;
type Remove = JoinHandle<bool>;
type Lookup = JoinHandle<Option<usize>>;
type Exists = JoinHandle<bool>;
type Reader<T = usize> = JoinHandle<Option<T>>;

type ArcTable = Arc<PTab<usize, ConstParams<{ Capacity::MIN.as_usize() }>>>;

struct LoomTable {
  inner: ArcTable,
}

impl LoomTable {
  fn new() -> Self {
    Self {
      inner: Arc::new(PTab::new()),
    }
  }

  fn spawn_insert(&self, value: usize) -> Insert {
    let table: ArcTable = ArcTable::clone(&self.inner);
    thread::spawn(move || table.insert(value))
  }

  fn spawn_remove(&self, index: Detached) -> Remove {
    let table: ArcTable = ArcTable::clone(&self.inner);
    thread::spawn(move || table.remove(index))
  }

  fn spawn_lookup(&self, index: Detached) -> Lookup {
    let table: ArcTable = ArcTable::clone(&self.inner);
    thread::spawn(move || table.read(index))
  }

  fn spawn_exists(&self, index: Detached) -> Exists {
    let table: ArcTable = ArcTable::clone(&self.inner);
    thread::spawn(move || table.exists(index))
  }

  fn spawn_reader<T, F>(&self, index: Detached, f: F) -> Reader<T>
  where
    T: 'static,
    F: Fn(&usize) -> T + 'static,
  {
    let table: ArcTable = ArcTable::clone(&self.inner);
    thread::spawn(move || table.with(index, f))
  }
}

impl Deref for LoomTable {
  type Target = ArcTable;

  #[inline]
  fn deref(&self) -> &Self::Target {
    &self.inner
  }
}

#[test]
fn test_insert() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();

    let thread_a: Insert = table.spawn_insert(1);
    let thread_b: Insert = table.spawn_insert(2);

    let result_a: Option<Detached> = thread_a.join().unwrap();
    let result_b: Option<Detached> = thread_b.join().unwrap();

    assert!(result_a.is_some());
    assert!(result_b.is_some());

    assert_ne!(result_a, result_b);
    assert_eq!(table.len(), 2);
  });
}

#[test]
fn test_insert_read() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index: Detached = table.insert(123).unwrap();

    let insert: Insert = table.spawn_insert(100);
    let lookup: Lookup = table.spawn_lookup(index);

    assert!(insert.join().unwrap().is_some());
    assert_eq!(lookup.join().unwrap(), Some(123));
  });
}

#[test]
fn test_insert_remove() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index: Detached = table.insert(1).unwrap();

    let insert: Insert = table.spawn_insert(2);
    let remove: Remove = table.spawn_remove(index);

    assert!(insert.join().unwrap().is_some());
    assert!(remove.join().unwrap());
    assert!(!table.exists(index));
  });
}

#[test]
fn test_insert_remove_read() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index: Detached = table.insert(123).unwrap();

    let insert: Insert = table.spawn_insert(456);
    let remove: Remove = table.spawn_remove(index);
    let lookup: Lookup = table.spawn_lookup(index);

    assert!(insert.join().unwrap().is_some());
    assert!(remove.join().unwrap());

    if let Some(value) = lookup.join().unwrap() {
      assert_eq!(value, 123);
    }
  });
}

#[test]
fn test_remove() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index_a: Detached = table.insert(1).unwrap();
    let index_b: Detached = table.insert(2).unwrap();

    let remove_a: Remove = table.spawn_remove(index_a);
    let remove_b: Remove = table.spawn_remove(index_b);

    assert!(remove_a.join().unwrap());
    assert!(remove_b.join().unwrap());

    assert!(!table.exists(index_a));
    assert!(!table.exists(index_b));

    assert_eq!(table.len(), 0);
  });
}

#[test]
fn test_remove_race() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index: Detached = table.insert(123).unwrap();

    let remove_a: Remove = table.spawn_remove(index);
    let remove_b: Remove = table.spawn_remove(index);

    let removed_a: bool = remove_a.join().unwrap();
    let removed_b: bool = remove_b.join().unwrap();

    assert!(removed_a || removed_b, "at least one remove should succeed");
    assert!(removed_a != removed_b, "exactly one remove should succeed");

    assert!(!table.exists(index));
    assert_eq!(table.len(), 0);
  });
}

#[test]
fn test_remove_race_read() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index: Detached = table.insert(123).unwrap();

    let lookup: Lookup = table.spawn_lookup(index);
    let remove: Remove = table.spawn_remove(index);

    assert!(remove.join().unwrap());

    if let Some(value) = lookup.join().unwrap() {
      assert_eq!(value, 123);
    }
  });
}

#[test]
fn test_remove_race_read_multi() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index: Detached = table.insert(123).unwrap();

    let lookup_a: Lookup = table.spawn_lookup(index);
    let lookup_b: Lookup = table.spawn_lookup(index);
    let remove: Remove = table.spawn_remove(index);

    assert!(remove.join().unwrap());

    if let Some(value) = lookup_a.join().unwrap() {
      assert_eq!(value, 123);
    }
    if let Some(value) = lookup_b.join().unwrap() {
      assert_eq!(value, 123);
    }
  });
}

#[test]
fn test_remove_race_exists() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index: Detached = table.insert(123).unwrap();

    let exists: Exists = table.spawn_exists(index);
    let remove: Remove = table.spawn_remove(index);

    // non-deterministic
    let _exists: bool = exists.join().unwrap();

    assert!(remove.join().unwrap());
  });
}

#[test]
fn test_remove_race_with() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index: Detached = table.insert(123).unwrap();

    let reader: Reader = table.spawn_reader(index, |value| *value * 2);
    let remove: Remove = table.spawn_remove(index);

    assert!(remove.join().unwrap());

    if let Some(value) = reader.join().unwrap() {
      assert_eq!(value, 246);
    }
  });
}

#[test]
fn test_capacity_race() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();

    for index in 0..table.capacity() - 1 {
      let _index: Detached = table.insert(index).unwrap();
    }

    let insert_a: Insert = table.spawn_insert(100);
    let insert_b: Insert = table.spawn_insert(200);

    let result_a: bool = insert_a.join().unwrap().is_some();
    let result_b: bool = insert_b.join().unwrap().is_some();

    assert!(result_a || result_b, "at least one insert should succeed");
    assert!(result_a != result_b, "exactly one insert should succeed");

    assert_eq!(table.len(), table.capacity());
  });
}

#[test]
fn test_length_consistency() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();

    let thread_a: JoinHandle<()> = {
      let table: ArcTable = ArcTable::clone(&table.inner);

      thread::spawn(move || {
        table.insert(1).unwrap();
        table.insert(2).unwrap();
      })
    };

    let thread_b: Insert = table.spawn_insert(3);

    thread_a.join().unwrap();
    thread_b.join().unwrap();

    assert_eq!(table.len(), 3);
  });
}

#[test]
fn test_length_insert_remove() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index: Detached = table.insert(1).unwrap();

    let insert: Insert = table.spawn_insert(2);
    let remove: Remove = table.spawn_remove(index);

    insert.join().unwrap();
    remove.join().unwrap();

    assert_eq!(table.len(), 1);
  });
}

#[test]
fn remove_and_reinsert() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(table.capacity());

    for index in 0..table.capacity() {
      keys.push(table.insert(index).unwrap());
    }

    assert!(table.insert(5).is_none());

    let remove: Remove = table.spawn_remove(keys[0]);
    let insert: Insert = table.spawn_insert(10);

    assert!(remove.join().unwrap());

    if let Some(index) = insert.join().unwrap() {
      assert!(!keys.contains(&index));
      assert_eq!(table.read(index), Some(10));
    }
  });
}

#[test]
fn test_three_way_insert() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();

    let insert_a: Insert = table.spawn_insert(1);
    let insert_b: Insert = table.spawn_insert(2);
    let insert_c: Insert = table.spawn_insert(3);

    let result_a: Option<Detached> = insert_a.join().unwrap();
    let result_b: Option<Detached> = insert_b.join().unwrap();
    let result_c: Option<Detached> = insert_c.join().unwrap();

    assert!(result_a.is_some());
    assert!(result_b.is_some());
    assert!(result_c.is_some());

    assert_ne!(result_a, result_b);
    assert_ne!(result_b, result_c);
    assert_ne!(result_a, result_c);

    assert_eq!(table.len(), 3);
  });
}

#[test]
fn test_concurrent_remove_reinsert() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let mut keys: Vec<Detached> = Vec::with_capacity(table.capacity());

    for index in 0..table.capacity() {
      keys.push(table.insert(index).unwrap());
    }

    let thread_a: JoinHandle<Option<Detached>> = {
      let table: ArcTable = ArcTable::clone(&table.inner);
      let key: Detached = keys[0];
      thread::spawn(move || {
        table.remove(key);
        table.insert(100)
      })
    };

    let thread_b: JoinHandle<Option<Detached>> = {
      let table: ArcTable = ArcTable::clone(&table.inner);
      let key: Detached = keys[1];
      thread::spawn(move || {
        table.remove(key);
        table.insert(200)
      })
    };

    let new_a: Option<Detached> = thread_a.join().unwrap();
    let new_b: Option<Detached> = thread_b.join().unwrap();

    assert!(new_a.is_some());
    assert!(new_b.is_some());

    assert_ne!(new_a, new_b);
    assert_eq!(table.read(new_a.unwrap()), Some(100));
    assert_eq!(table.read(new_b.unwrap()), Some(200));
  });
}

#[test]
fn test_read_unaffected_by_other_remove() {
  loom::model(|| {
    let table: LoomTable = LoomTable::new();
    let index_a: Detached = table.insert(111).unwrap();
    let index_b: Detached = table.insert(222).unwrap();

    let lookup_b: Lookup = table.spawn_lookup(index_b);
    let remove_a: Remove = table.spawn_remove(index_a);

    assert!(remove_a.join().unwrap());
    assert_eq!(lookup_b.join().unwrap(), Some(222));
  });
}
