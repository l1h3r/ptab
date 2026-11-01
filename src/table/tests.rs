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
use crate::params::Params as _;
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
  let guard: Guard<MinParams> = MinParams::guard();
  let index: Detached = table.insert(123).unwrap();

  assert!(table.exists(index, &guard));
  assert_eq!(table.read(index, &guard), Some(123));
  assert!(table.remove(index));
  refute!(table.exists(index, &guard));
}
