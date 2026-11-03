#![allow(clippy::let_unit_value, reason = "conditional")]
#![allow(clippy::unit_arg, reason = "conditional")]

#[cfg(feature = "sdd")]
use core::time::Duration;
#[cfg(feature = "std")]
use std::sync::Arc;
#[cfg(feature = "std")]
use std::sync::Barrier;
#[cfg(feature = "std")]
use std::thread;
#[cfg(feature = "std")]
use std::thread::JoinHandle;
#[cfg(feature = "sdd")]
use std::time::Instant;

use crate::index::Abstract;
use crate::index::Concrete;
use crate::index::Detached;
use crate::params::CACHE_LINE_SLOTS;
use crate::params::Capacity;
use crate::params::ConstParams;
use crate::params::Params;
use crate::params::ParamsExt as _;
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
use crate::utils::refute;
use crate::utils::std::BTreeSet;
use crate::utils::std::String;
use crate::utils::std::Vec;
use crate::utils::std::format;

#[allow(unused_imports, reason = "conditional")]
use crate::reclaim::Shared as _;

type DefParams = ConstParams<{ Capacity::DEF.as_usize() }>;
type MaxParams = ConstParams<{ Capacity::MAX.as_usize() }>;
type MinParams = ConstParams<{ Capacity::MIN.as_usize() }>;

type ReadOnly<P = DefParams> = table::ReadOnly<u64, P>;

#[cfg(feature = "std")]
const THREADS: usize = 8;

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

// -----------------------------------------------------------------------------
// Internals
// -----------------------------------------------------------------------------

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

#[cfg(feature = "std")]
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

fn undo_reservation_keeps_when_room<P: Params>() {
  let table: Table<usize, P> = Table::new();
  let length: u32 = P::LENGTH.as_u32();

  table.volatile.entries.store(length - 1, Ordering::Relaxed);

  assert!(table.undo_reservation(length + 1).is_some());
  assert_eq!(table.volatile.entries.load(Ordering::Relaxed), length - 1);
}

fn undo_reservation_keeps_at_length<P: Params>() {
  let table: Table<usize, P> = Table::new();
  let length: u32 = P::LENGTH.as_u32();

  table.volatile.entries.store(length, Ordering::Relaxed);

  assert!(table.undo_reservation(length + 1).is_some());
  assert_eq!(table.volatile.entries.load(Ordering::Relaxed), length);
}

fn undo_reservation_undoes_when_full<P: Params>() {
  let table: Table<usize, P> = Table::new();
  let length: u32 = P::LENGTH.as_u32();

  table.volatile.entries.store(length + 1, Ordering::Relaxed);

  assert!(table.undo_reservation(length + 1).is_none());
  assert_eq!(table.volatile.entries.load(Ordering::Relaxed), length);
}

#[test]
fn undo_reservation_keeps_when_room_def() {
  undo_reservation_keeps_when_room::<DefParams>();
}

#[test]
fn undo_reservation_keeps_when_room_max() {
  undo_reservation_keeps_when_room::<MaxParams>();
}

#[test]
fn undo_reservation_keeps_at_length_def() {
  undo_reservation_keeps_at_length::<DefParams>();
}

#[test]
fn undo_reservation_keeps_at_length_max() {
  undo_reservation_keeps_at_length::<MaxParams>();
}

#[test]
fn undo_reservation_undoes_when_full_def() {
  undo_reservation_undoes_when_full::<DefParams>();
}

#[test]
fn undo_reservation_undoes_when_full_max() {
  undo_reservation_undoes_when_full::<MaxParams>();
}

#[test]
fn acquire_slot() {
  let table: Table<usize, DefParams> = Table::new();
  let mut indices: BTreeSet<usize> = BTreeSet::new();

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
fn release_slot_fifo() {
  const ROUNDS: usize = 4;

  let table: Table<usize, MinParams> = Table::new();
  let cap: usize = table.cap();

  let reserved_cells = |table: &Table<usize, MinParams>| -> usize {
    table
      .readonly
      .slot
      .as_slice()
      .iter()
      .filter(|cell| cell.load(Ordering::Relaxed) == RESERVED)
      .count()
  };

  for _ in 0..ROUNDS {
    let mut acquired: Vec<Abstract<MinParams>> = Vec::with_capacity(cap);

    for _ in 0..cap {
      let permit: Permit<'_, usize, MinParams> = table.reserve_slot().unwrap();
      acquired.push(table.acquire_slot(permit));
    }

    assert_eq!(reserved_cells(&table), table.len() as usize);
    assert_eq!(table.len() as usize, cap);

    let mut expected: Vec<usize> = Vec::with_capacity(cap);

    while let Some(index) = acquired.pop() {
      expected.push(table.generate_next_slot(index));
      table.release_slot(index);
    }

    assert_eq!(reserved_cells(&table), table.len() as usize);
    assert!(table.is_empty());

    for want in expected {
      let permit: Permit<'_, usize, MinParams> = table.reserve_slot().unwrap();
      let got: Abstract<MinParams> = table.acquire_slot(permit);

      assert_eq!(got.get(), want);
      acquired.push(got);
    }

    for index in acquired {
      table.release_slot(index);
    }

    assert_eq!(reserved_cells(&table), 0);
  }
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
  let mut indices: BTreeSet<usize> = BTreeSet::new();
  let mut uniques: BTreeSet<Detached> = BTreeSet::new();

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

#[cfg(feature = "std")]
#[test]
fn slot_churn() {
  static PERMITS: AtomicUsize = AtomicUsize::new(0);
  static UNIQUES: AtomicUsize = AtomicUsize::new(0);
  static RELEASES: AtomicUsize = AtomicUsize::new(0);

  let table: Arc<Table<usize, DefParams>> = Arc::new(Table::new());
  let barrier: Arc<Barrier> = Arc::new(Barrier::new(THREADS + 1));
  let capacity: usize = table.cap();

  let mut threads: Vec<JoinHandle<Vec<usize>>> = Vec::with_capacity(THREADS);
  let mut uniques: BTreeSet<usize> = BTreeSet::new();

  for _ in 0..THREADS {
    let barrier: Arc<Barrier> = Arc::clone(&barrier);
    let table: Arc<Table<usize, DefParams>> = Arc::clone(&table);

    threads.push(thread::spawn(move || {
      let mut track: Vec<usize> = Vec::with_capacity(capacity * 4);
      let mut released: usize = 0;

      barrier.wait();

      for _ in 0..4 {
        for _ in 0..capacity {
          if let Some(permit) = table.reserve_slot() {
            track.push(table.acquire_slot(permit).get());

            assert!(table.len() <= capacity as u32);
            assert!(PERMITS.fetch_add(1, Ordering::Relaxed) < capacity);

            UNIQUES.fetch_add(1, Ordering::Relaxed);
          }

          if track.len().is_multiple_of(2)
            && let Some(key) = track.get(released)
          {
            PERMITS.fetch_sub(1, Ordering::Relaxed);
            table.release_slot(Abstract::new(*key));
            released += 1;

            RELEASES.fetch_add(1, Ordering::Relaxed);
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
  assert_ne!(RELEASES.load(Ordering::Relaxed), 0);
  assert_eq!(
    table.len() as usize,
    UNIQUES.load(Ordering::Relaxed) - RELEASES.load(Ordering::Relaxed),
  );
  assert_eq!(table.len() as usize, PERMITS.load(Ordering::Relaxed));
}

// -----------------------------------------------------------------------------
// API
// -----------------------------------------------------------------------------

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

#[cfg(feature = "sdd")]
#[test]
fn remove() {
  let table: Table<usize, DefParams> = Table::new();
  let index: Detached = table.insert(123).unwrap();

  assert_eq!(table.len(), 1);
  refute!(table.is_empty());

  assert!(table.remove(index));

  assert_eq!(table.len(), 0);
  assert!(table.is_empty());
}

#[cfg(feature = "sdd")]
#[test]
fn remove_nonexistent() {
  let table: Table<usize, DefParams> = Table::new();
  let index: Detached = table.insert(123).unwrap();

  assert!(table.remove(index));
  refute!(table.remove(index));
}

#[cfg(feature = "sdd")]
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
fn weak_keys_last_slot() {
  let table: Table<usize, MaxParams> = Table::new();
  let index: Abstract<MaxParams> = Abstract::new(MaxParams::LENGTH.as_usize() - 1);
  let concrete: Concrete<MaxParams> = Concrete::from_abstract(index);

  table
    .readonly
    .data
    .get(concrete)
    .write(Ordering::Release, |entry| {
      entry.write(7);
    });

  assert_eq!(table.weak_keys(MaxParams::guard()).count(), 1);

  table.volatile.incr_entries();
}

#[cfg(feature = "sdd")]
#[cfg_attr(miri, ignore)]
#[test]
fn weak_keys_full_max() {
  let table: Table<(), MaxParams> = Table::new();
  let first: Detached = table.insert(()).unwrap();

  for _ in 1..table.cap() {
    table.insert(()).unwrap();
  }

  assert!(table.remove(first));
  table.insert(()).unwrap();

  assert_eq!(
    table.weak_keys(MaxParams::guard()).count(),
    table.len() as usize
  );
}

#[test]
fn drop_empty() {
  make_drop!(DropMe);

  assert_eq!(DropMe::load(), 0);
  drop(Table::<DropMe, DefParams>::new());
  assert_eq!(DropMe::load(), 0);
}

#[cfg(feature = "sdd")]
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

  // `Sdd::flush` frees the removed entries once no other thread is pinned in an
  // older epoch.
  let expected: u32 = table.cap() as u32 / 2;
  let deadline: Instant = Instant::now() + Duration::from_secs(10);

  while DropMe::load() > expected && Instant::now() < deadline {
    DefParams::flush();
    thread::sleep(Duration::from_millis(1));
  }

  assert_eq!(DropMe::load(), expected);
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

#[cfg(feature = "sdd")]
#[test]
fn min_capacity_ops() {
  let table: Table<usize, MinParams> = Table::new();
  let guard: Guard<MinParams> = MinParams::guard();
  let index: Detached = table.insert(123).unwrap();

  assert!(table.exists(index, &guard));
  assert_eq!(table.read(index, &guard), Some(123));
  assert!(table.remove(index));
  refute!(table.exists(index, &guard));
}
