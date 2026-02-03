use std::collections::HashSet;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use crate::index::Detached;
use crate::params::Capacity;
use crate::params::ConstParams;
use crate::table::Table;

type TestParams = ConstParams<64>;

#[cfg(not(miri))]
#[test]
fn test_new() {
  let table: Table<usize, ConstParams<{ Capacity::DEF.as_usize() }>> = Table::new();

  assert_eq!(table.cap(), Capacity::DEF.as_usize());
  assert_eq!(table.len(), 0);
  assert!(table.is_empty());
}

#[test]
fn test_insert_single() {
  let table: Table<usize, TestParams> = Table::new();
  let index: Detached = table.insert(123).unwrap();

  assert_eq!(table.len(), 1);
  assert!(!table.is_empty());
  assert!(table.exists(index));
  assert_eq!(table.read(index), Some(123));
}

#[test]
fn test_insert_multiple() {
  let table: Table<usize, TestParams> = Table::new();
  let mut keys: Vec<Detached> = Vec::with_capacity(32);

  for index in 0..32 {
    keys.push(table.insert(index * 100).unwrap());
  }

  assert_eq!(table.len(), 32);

  for (index, key) in keys.iter().enumerate() {
    assert_eq!(table.read(*key), Some(index * 100));
  }
}

#[test]
fn test_insert_unique_ids() {
  let table: Table<usize, TestParams> = Table::new();
  let mut keys: HashSet<Detached> = HashSet::new();

  for _ in 0..table.cap() {
    assert!(keys.insert(table.insert(0).unwrap()));
  }
}

#[test]
fn test_insert_maximum() {
  let table: Table<usize, TestParams> = Table::new();

  for index in 0..table.cap() {
    assert!(table.insert(index).is_some());
  }

  assert_eq!(table.len(), table.cap() as u32);
  assert_eq!(table.insert(9999), None);
}

#[test]
fn test_write_callback_receives_correct_index() {
  let table: Table<usize, TestParams> = Table::new();

  let index: Detached = table
    .write(|uninit, index| {
      uninit.write(index.into_bits());
    })
    .unwrap();

  assert_eq!(table.read(index), Some(index.into_bits()));
}

#[test]
fn test_remove_existing() {
  let table: Table<usize, TestParams> = Table::new();
  let index: Detached = table.insert(123).unwrap();

  assert_eq!(table.len(), 1);
  assert!(!table.is_empty());
  assert!(table.exists(index));
  assert!(table.remove(index));

  assert_eq!(table.len(), 0);
  assert!(table.is_empty());
  assert!(!table.exists(index));
}

#[test]
fn test_remove_nonexistent() {
  let table: Table<usize, TestParams> = Table::new();
  let index: Detached = table.insert(123).unwrap();

  assert!(table.remove(index));
  assert!(!table.remove(index));
}

#[test]
fn test_remove_isolation() {
  let table: Table<usize, TestParams> = Table::new();
  let mut keys: Vec<Detached> = Vec::with_capacity(16);

  for index in 0..16 {
    keys.push(table.insert(index).unwrap());
  }

  for index in (0..16).step_by(2) {
    table.remove(keys[index]);
  }

  assert_eq!(table.len(), 8);

  for index in (1..16).step_by(2) {
    assert_eq!(table.read(keys[index]), Some(index));
  }

  for index in (0..16).step_by(2) {
    assert!(!table.exists(keys[index]));
  }
}

#[test]
fn test_remove_recycling() {
  let table: Table<usize, TestParams> = Table::new();
  let mut keys: Vec<Detached> = Vec::with_capacity(table.cap() - 1);

  for index in 0..table.cap() {
    keys.push(table.insert(index).unwrap());
  }

  assert_eq!(table.insert(99), None);

  table.remove(keys[0]);

  let index: Detached = table.insert(100).unwrap();

  assert!(table.exists(index));
  assert_eq!(table.read(index), Some(100));
}

#[test]
fn test_exists_existing() {
  let table: Table<usize, TestParams> = Table::new();
  let index: Detached = table.insert(123).unwrap();

  assert!(table.exists(index));
}

#[test]
fn test_exists_nonexistent() {
  let table: Table<usize, TestParams> = Table::new();
  let index: Detached = table.insert(123).unwrap();

  assert!(table.remove(index));
  assert!(!table.exists(index));
}

#[test]
fn test_exists_multiple() {
  let table: Table<usize, TestParams> = Table::new();

  let index1: Detached = table.insert(1).unwrap();
  let index2: Detached = table.insert(2).unwrap();
  let index3: Detached = table.insert(3).unwrap();

  assert!(table.exists(index1));
  assert!(table.exists(index2));
  assert!(table.exists(index3));

  table.remove(index2);

  assert!(table.exists(index1));
  assert!(!table.exists(index2));
  assert!(table.exists(index3));
}

#[test]
fn test_with_value() {
  let table: Table<usize, TestParams> = Table::new();
  let index: Detached = table.insert(12345).unwrap();

  assert_eq!(table.with(index, |data| *data), Some(12345));
}

#[test]
fn test_with_return_value() {
  let table: Table<usize, TestParams> = Table::new();
  let index: Detached = table.insert(123).unwrap();

  assert_eq!(table.with(index, |data| data + 1), Some(124));
}

#[test]
fn test_with_nonexistent() {
  let table: Table<usize, TestParams> = Table::new();
  let index: Detached = table.insert(123).unwrap();

  assert!(table.remove(index));
  assert_eq!(table.with(index, |data| *data), None);
}

#[test]
fn test_with_multiple() {
  let table: Table<usize, TestParams> = Table::new();
  let index: Detached = table.insert(123).unwrap();

  for _ in 0..100 {
    assert_eq!(table.with(index, |data| *data), Some(123));
  }
}

#[test]
fn test_len_tracks_insertions() {
  let table: Table<usize, TestParams> = Table::new();

  for index in 0..16 {
    assert!(table.insert(0).is_some());
    assert_eq!(table.len(), index + 1);
  }
}

#[test]
fn test_len_tracks_removals() {
  let table: Table<usize, TestParams> = Table::new();
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
fn test_is_empty() {
  let table: Table<usize, TestParams> = Table::new();

  assert!(table.is_empty());

  let index: Detached = table.insert(0).unwrap();

  assert!(!table.is_empty());
  assert!(table.remove(index));
  assert!(table.is_empty());
}

#[test]
fn test_interleaved_insert_remove() {
  let table: Table<usize, TestParams> = Table::new();
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
    assert!(table.exists(key));
  }
}

#[test]
fn test_multiple_refills() {
  let table: Table<usize, TestParams> = Table::new();

  for round in 0..3 {
    let mut keys: Vec<Detached> = Vec::new();

    for index in 0..16 {
      keys.push(table.insert(round * 100 + index).unwrap());
    }
    assert_eq!(table.len(), 16);

    for (index, key) in keys.iter().enumerate() {
      assert_eq!(table.read(*key), Some(round * 100 + index));
    }

    for key in keys {
      table.remove(key);
    }
    assert_eq!(table.len(), 0);
  }
}

#[test]
fn test_uniqueness_across_multiple_generations() {
  const GENS: usize = 10;

  let table: Table<usize, TestParams> = Table::new();
  let mut key_set: HashSet<usize> = HashSet::with_capacity(GENS * 16);

  for _ in 0..GENS {
    let mut key_arr: Vec<Detached> = Vec::with_capacity(16);

    for _ in 0..16 {
      let index: Detached = table.insert(0).unwrap();

      key_arr.push(index);

      assert!(key_set.insert(index.into_bits()));
    }

    for key in key_arr {
      table.remove(key);
    }
  }
}

#[test]
fn test_min_capacity_operations() {
  type Params = ConstParams<{ Capacity::MIN.as_usize() }>;

  let table: Table<usize, Params> = Table::new();
  assert_eq!(table.cap(), Capacity::MIN.as_usize());

  let index: Detached = table.insert(99).unwrap();

  assert!(table.exists(index));
  assert_eq!(table.read(index), Some(99));
  assert!(table.remove(index));
  assert!(!table.exists(index));
}

#[cfg_attr(not(feature = "slow"), ignore = "enable the 'slow' feature to run this test.")]
#[test]
fn test_max_capacity_operations() {
  type Params = ConstParams<{ Capacity::MAX.as_usize() }>;

  let table: Table<usize, Params> = Table::new();

  assert_eq!(table.cap(), Capacity::MAX.as_usize());
  assert_eq!(table.len(), 1); // See `Volatile::new`
}

#[test]
fn test_drop() {
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

  let drop_0: Table<DropMe, TestParams> = Table::new();

  assert_eq!(COUNT.load(Ordering::Relaxed), 0);
  assert_eq!(COUNT.load(Ordering::Relaxed), drop_0.len());
  drop(drop_0);
  assert_eq!(COUNT.load(Ordering::Relaxed), 0);

  let drop_1: Table<DropMe, TestParams> = {
    let this = Table::new();
    this.insert(DropMe::new()).unwrap();
    this
  };

  assert_eq!(COUNT.load(Ordering::Relaxed), 1);
  assert_eq!(COUNT.load(Ordering::Relaxed), drop_1.len());
  drop(drop_1);
  assert_eq!(COUNT.load(Ordering::Relaxed), 0);

  let drop_full: Table<DropMe, TestParams> = {
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
