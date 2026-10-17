use divan::Bencher;
use divan::black_box;
use divan::black_box_drop;
use mimalloc::MiMalloc;

use ptab_benchmarks::Collection;
use ptab_benchmarks::Handle;
use ptab_benchmarks::LocalHandle;
use ptab_benchmarks::adapter::*;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

// -----------------------------------------------------------------------------
// Config
// -----------------------------------------------------------------------------

const SAMPLES: u32 = 5000;

const CAP: usize = ptab::Capacity::DEF.as_usize();

#[cfg(feature = "slow")]
const OPS: &[usize] = &[
  1 << 4,
  1 << 5,
  1 << 6,
  1 << 7,
  1 << 8,
  1 << 9,
  1 << 10,
  1 << 11,
  1 << 12,
  1 << 13,
  1 << 14,
  1 << 15,
  1 << 16,
];

#[cfg(not(feature = "slow"))]
const OPS: &[usize] = &[1 << 4, 1 << 10, 1 << 16];

// // https://developer.apple.com/library/archive/documentation/Performance/Conceptual/EnergyGuide-iOS/PrioritizeWorkWithQoS.html
// #[cfg(target_os = "macos")]
// mod qos {
//   use libc::pthread_set_qos_class_self_np;
//   use libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE;

//   #[inline]
//   pub fn set_realtime() -> bool {
//     unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) == 0 }
//   }
// }

// #[cfg(not(target_os = "macos"))]
// mod qos {
//   #[inline]
//   pub fn set_realtime() -> bool {
//     false
//   }
// }

// -----------------------------------------------------------------------------
// Macros
// -----------------------------------------------------------------------------

macro_rules! benches {
  (async; $($name:ident as $collection:ty),+ $(,)?) => {
    $(
      #[divan::bench(args = OPS, sample_count = SAMPLES, threads = [1, 4, 6, 8])]
      fn $name(bencher: Bencher<'_, '_>, ops: usize) {
        bench::<$collection>(bencher, ops);
      }
    )+
  };
  (local; $($name:ident as $collection:ty),+ $(,)?) => {
    $(
      #[divan::bench(args = OPS, sample_count = SAMPLES, threads = 1)]
      fn $name(bencher: Bencher<'_, '_>, ops: usize) {
        bench::<$collection>(bencher, ops);
      }
    )+
  };
}

// -----------------------------------------------------------------------------
// Actual Benchmarks
// -----------------------------------------------------------------------------

#[divan::bench_group(name = "ReadSeq::Local", skip_ext_time)]
mod read_seq_local {
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Collection<usize>,
  {
    let mut this: T::Local = T::new::<CAP>().into_local();
    let keys: Vec<_> = (0..ops).map(|index| this.set(index).unwrap()).collect();

    bencher.counter(ops).bench_local(move || {
      for key in keys.iter() {
        let item: Option<usize> = black_box(this.get(black_box(*key)));
        _ = black_box(item.unwrap());
      }
    });
  }

  benches! {
    local;
    // generational_arena as GenerationalArena<usize>,
    // idr as Idr<usize>,
    // ptab as PTab<usize>,
    // sharded_slab as ShardedSlab<usize>,
    // slab as Slab<usize>,
    // slotmap as SlotMap<usize>,
    // thunderdome as Thunderdome<usize>,
    ptab as PTab<usize>,
    ptab_develop as PTabDevelop<usize>,
  }
}

#[divan::bench_group(name = "ReadSeq::Multi", skip_ext_time)]
mod read_seq_multi {
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Collection<usize>,
  {
    let this: T::Async = T::new::<CAP>().into_async();
    let keys: Vec<_> = (0..ops).map(|index| this.set(index).unwrap()).collect();

    bencher.counter(ops).bench(move || {
      for key in keys.iter() {
        let item: Option<usize> = black_box(this.get(black_box(*key)));
        _ = black_box(item.unwrap());
      }
    });
  }

  benches! {
    async;
    // dashmap as DashMap<usize>,
    // flurry as Flurry<usize>,
    // idr as Idr<usize>,
    // papaya as Papaya<usize>,
    // ptab as PTab<usize>,
    // sharded_slab as ShardedSlab<usize>,
    ptab as PTab<usize>,
    ptab_develop as PTabDevelop<usize>,
  }
}

#[divan::bench_group(name = "ReadHot::Multi", skip_ext_time)]
mod read_hot_multi {
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Collection<usize>,
  {
    let this: T::Async = T::new::<CAP>().into_async();
    let hkey: <T::Async as Handle<usize>>::Key = this.set(0).unwrap();

    bencher.counter(ops).bench(move || {
      for _ in 0..ops {
        let item: Option<usize> = black_box(this.get(black_box(hkey)));
        _ = black_box(item.unwrap());
      }
    });
  }

  benches! {
    async;
    // dashmap as DashMap<usize>,
    // flurry as Flurry<usize>,
    // idr as Idr<usize>,
    // papaya as Papaya<usize>,
    // ptab as PTab<usize>,
    // sharded_slab as ShardedSlab<usize>,
    ptab as PTab<usize>,
    ptab_develop as PTabDevelop<usize>,
  }
}

#[divan::bench_group(name = "InsertSeq::Local", skip_ext_time)]
mod insert_seq_local {
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Collection<usize>,
  {
    bencher
      .counter(ops)
      .with_inputs(|| T::new::<CAP>().into_local())
      .bench_local_refs(move |this: &mut T::Local| {
        for index in 0..ops {
          let hkey: Option<_> = black_box(this.set(black_box(index)));
          _ = black_box(hkey.unwrap());
        }
      });
  }

  benches! {
    local;
    // generational_arena as GenerationalArena<usize>,
    // idr as Idr<usize>,
    // ptab as PTab<usize>,
    // sharded_slab as ShardedSlab<usize>,
    // slab as Slab<usize>,
    // slotmap as SlotMap<usize>,
    // thunderdome as Thunderdome<usize>,
    ptab as PTab<usize>,
    ptab_develop as PTabDevelop<usize>,
  }
}

#[divan::bench_group(name = "InsertSeq::Multi", skip_ext_time)]
mod insert_seq_multi {
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Collection<usize>,
  {
    bencher
      .counter(ops)
      .with_inputs(|| T::new::<CAP>().into_async())
      .bench_refs(move |this: &mut T::Async| {
        for index in 0..ops {
          let hkey: Option<_> = black_box(this.set(black_box(index)));
          _ = black_box(hkey.unwrap());
        }
      });
  }

  benches! {
    async;
    // dashmap as DashMap<usize>,
    // flurry as Flurry<usize>,
    // idr as Idr<usize>,
    // papaya as Papaya<usize>,
    // ptab as PTab<usize>,
    // sharded_slab as ShardedSlab<usize>,
    ptab as PTab<usize>,
    ptab_develop as PTabDevelop<usize>,
  }
}

#[divan::bench_group(name = "Churn::Local", skip_ext_time)]
mod churn_local {
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Collection<usize>,
  {
    bencher
      .counter(ops)
      .with_inputs(|| T::new::<CAP>().into_local())
      .bench_local_refs(move |this: &mut T::Local| {
        for index in 0..ops {
          let hkey: Option<_> = black_box(this.set(black_box(index)));
          let _del: bool = black_box(this.del(hkey.unwrap()));
        }
      });
  }

  benches! {
    local;
    // generational_arena as GenerationalArena<usize>,
    // idr as Idr<usize>,
    // ptab as PTab<usize>,
    // sharded_slab as ShardedSlab<usize>,
    // slab as Slab<usize>,
    // slotmap as SlotMap<usize>,
    // thunderdome as Thunderdome<usize>,
    ptab as PTab<usize>,
    ptab_develop as PTabDevelop<usize>,
  }
}

#[divan::bench_group(name = "Churn::Multi", skip_ext_time)]
mod churn_multi {
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Collection<usize>,
  {
    bencher
      .counter(ops)
      .with_inputs(|| T::new::<CAP>().into_async())
      .bench_refs(move |this: &mut T::Async| {
        for index in 0..ops {
          let hkey: Option<_> = black_box(this.set(black_box(index)));
          let _del: bool = black_box(this.del(hkey.unwrap()));
        }
      });
  }

  benches! {
    async;
    // dashmap as DashMap<usize>,
    // flurry as Flurry<usize>,
    // idr as Idr<usize>,
    // papaya as Papaya<usize>,
    // ptab as PTab<usize>,
    // sharded_slab as ShardedSlab<usize>,
    ptab as PTab<usize>,
    ptab_develop as PTabDevelop<usize>,
  }
}

#[divan::bench_group(name = "Drop", skip_ext_time)]
mod drop {
  use super::*;

  struct DropMe(usize);

  impl Drop for DropMe {
    fn drop(&mut self) {
      black_box(self.0);
    }
  }

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Collection<DropMe>,
  {
    bencher
      .counter(ops)
      .with_inputs(move || {
        let mut this: T::Local = T::new::<CAP>().into_local();

        for index in 0..ops {
          _ = this.set(DropMe(index)).unwrap();
        }

        this
      })
      .bench_local_values(black_box_drop);
  }

  benches! {
    local;
    ptab as PTab<DropMe>,
    ptab_develop as PTabDevelop<DropMe>,
    // generational_arena as GenerationalArena<DropMe>,
    // idr as Idr<DropMe>,
    // ptab as PTab<DropMe>,
    // sharded_slab as ShardedSlab<DropMe>,
    // slab as Slab<DropMe>,
    // slotmap as SlotMap<DropMe>,
    // thunderdome as Thunderdome<DropMe>,
  }
}

// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

fn main() {
  divan::main();
}
