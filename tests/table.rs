#![cfg(not(any(loom, shuttle)))]

use std::collections::HashSet;

use ptab::Capacity;
use ptab::ConstParams;
use ptab::Detached;
use ptab::PTab;

type Table = PTab<usize, ConstParams<{ Capacity::MIN.as_usize() }>>;

#[test]
fn insert_beyond_capacity() {
  let table: Table = Table::new();

  for _ in 0..table.capacity() {
    assert_ne!(table.insert(123), None);
  }

  assert_eq!(table.len(), table.capacity());
  assert_eq!(table.insert(123), None);
  assert_eq!(table.len(), table.capacity());
}

#[test]
fn insert_unique_ids() {
  let table: Table = Table::new();
  let mut keys: HashSet<Detached> = HashSet::new();

  for _ in 0..table.capacity() {
    assert!(keys.insert(table.insert(123).unwrap()));
  }
}

#[test]
fn write_callback_correct_index() {
  let table: Table = Table::new();

  let index: Detached = table
    .write(|uninit, index| {
      uninit.write(index.into_bits());
    })
    .unwrap();

  assert_eq!(table.read(index), Some(index.into_bits()));
}
