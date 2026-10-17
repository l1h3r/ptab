use std::fmt::Debug;
use std::hint::black_box;
use std::iter;
use std::sync::Arc;
use std::sync::Barrier;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::collect::Collection;
use crate::collect::Handle;

// -----------------------------------------------------------------------------
// LCG
// -----------------------------------------------------------------------------

struct Lcg {
  a: usize,
  c: usize,
  m: usize,
  x: usize,
}

impl Lcg {
  const fn new(len: usize) -> Self {
    assert!(len.is_power_of_two() && len > 4);

    Self {
      a: len / 2 + 1,
      c: len / 4 - 1,
      m: len - 1,
      x: 0,
    }
  }

  const fn next(&mut self) -> usize {
    let data: usize = self.x;
    self.x = (self.a * self.x + self.c) & self.m;
    data
  }

  fn shuffle<T>(&mut self, slice: &mut [T]) {
    for index in (1..slice.len()).rev() {
      slice.swap(index, self.next() % (index + 1));
    }
  }
}

// -----------------------------------------------------------------------------
// Mix
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Mix {
  pub lookup: u8,
  pub insert: u8,
  pub remove: u8,
}

impl Mix {
  pub const fn lookup() -> Self {
    Self {
      lookup: 100,
      insert: 0,
      remove: 0,
    }
  }

  pub const fn insert() -> Self {
    Self {
      lookup: 0,
      insert: 100,
      remove: 0,
    }
  }

  pub const fn lookup_heavy() -> Self {
    Self {
      lookup: 95,
      insert: 3,
      remove: 2,
    }
  }

  pub const fn insert_heavy() -> Self {
    Self {
      lookup: 10,
      insert: 85,
      remove: 5,
    }
  }

  pub const fn uniform() -> Self {
    Self {
      lookup: 34,
      insert: 33,
      remove: 33,
    }
  }

  pub const fn churn() -> Self {
    Self {
      lookup: 20,
      insert: 40,
      remove: 40,
    }
  }

  pub const fn insert_remove() -> Self {
    Self {
      lookup: 0,
      insert: 50,
      remove: 50,
    }
  }

  fn validate(&self) {
    assert_eq!(self.lookup + self.insert + self.remove, 100);
  }
}

// -----------------------------------------------------------------------------
// Stats
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Stats {
  pub median: f64,
  pub stddev: f64,
  pub samples: Vec<f64>,
}

impl Stats {
  fn from_samples(mut samples: Vec<f64>) -> Self {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let variance = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;

    Self {
      median: samples[samples.len() / 2],
      stddev: variance.sqrt(),
      samples,
    }
  }

  pub fn mean(&self) -> f64 {
    self.samples.iter().sum::<f64>() / self.samples.len() as f64
  }

  pub fn cv(&self) -> f64 {
    self.stddev / self.mean() * 100.0
  }
}

// -----------------------------------------------------------------------------
// Config
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Config {
  pub warmup: usize,
  pub samples: usize,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      warmup: 9,
      // samples: 9,
      samples: 30,
    }
  }
}

// -----------------------------------------------------------------------------
// Workload
// -----------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Action {
  Insert,
  Lookup,
  Remove,
}

const CAPACITY: usize = ptab::Capacity::DEF.as_usize();
const MULTIPLE: f64 = 0.75;
const FRACTION: f64 = 0.50;

struct Context<T>
where
  T: Collection<usize>,
{
  total_ops: usize,
  prefill: usize,
  ops_per_thread: usize,
  actions: Arc<[Action]>,
  this: Arc<T::Async>,
  keys: Arc<[<T::Async as Handle<usize>>::Key]>,
}

impl<T> Context<T>
where
  T: Collection<usize>,
{
  fn new(workload: &Workload) -> Self {
    let total_ops: usize = (CAPACITY as f64 * MULTIPLE) as usize;
    let prefill: usize = (CAPACITY as f64 * FRACTION) as usize;
    let ops_per_thread: usize = total_ops / workload.threads;

    let actions: Arc<[Action]> = {
      let mut ops: Vec<Action> = Vec::with_capacity(100);
      ops.extend(iter::repeat(Action::Insert).take(workload.actions.insert as usize));
      ops.extend(iter::repeat(Action::Lookup).take(workload.actions.lookup as usize));
      ops.extend(iter::repeat(Action::Remove).take(workload.actions.remove as usize));
      Lcg::new(128).shuffle(&mut ops);
      ops.into()
    };

    let this: Arc<T::Async> = Arc::new(T::new::<{ CAPACITY }>().into_async());

    let keys: Arc<[<T::Async as Handle<usize>>::Key]> =
      (0..prefill).map(|index| this.set(index).unwrap()).collect();

    Self {
      total_ops,
      prefill,
      ops_per_thread,
      actions,
      this,
      keys,
    }
  }
}

#[derive(Clone, Debug)]
pub struct Workload {
  actions: Mix,
  threads: usize,
}

impl Workload {
  pub const fn new(actions: Mix, threads: usize) -> Self {
    Self { actions, threads }
  }

  pub fn run<T>(&self, config: &Config) -> Stats
  where
    T: Collection<usize>,
    <T::Async as Handle<usize>>::Key: Debug,
  {
    self.actions.validate();

    let context: Context<T> = Context::new(self);

    for _ in 0..config.warmup {
      let _ = self.sample::<T>(&context);
    }

    Stats::from_samples(Vec::from_iter(
      (0..config.samples).map(|_| self.sample::<T>(&context)),
    ))
  }

  fn sample<T>(&self, context: &Context<T>) -> f64
  where
    T: Collection<usize>,
    <T::Async as Handle<usize>>::Key: Debug,
  {
    let total_ops: usize = context.total_ops;
    let prefill: usize = context.prefill;
    let ops_per_thread: usize = context.ops_per_thread;

    black_box(&*context.keys);
    black_box(&*context.this);

    let barrier: Arc<Barrier> = Arc::new(Barrier::new(self.threads + 1));

    let handles: Vec<_> = (0..self.threads)
      .map(|_| {
        let this: Arc<T::Async> = Arc::clone(&context.this);
        let keys: Arc<[_]> = Arc::clone(&context.keys);

        let actions: Arc<[Action]> = Arc::clone(&context.actions);
        let barrier: Arc<Barrier> = Arc::clone(&barrier);

        thread::spawn(move || {
          // qos::set_realtime();

          let mut lcg: Lcg = Lcg::new(keys.len());
          let mut val: usize = prefill;

          let iter: _ = iter::repeat(actions.iter()).flatten().take(ops_per_thread);

          barrier.wait();

          for action in iter {
            match action {
              Action::Insert => {
                _ = black_box(this.set(black_box(val)));
                val += 1;
              }
              Action::Lookup => {
                _ = black_box(this.get(black_box(keys[lcg.next()])));
              }
              Action::Remove => {
                _ = black_box(this.del(black_box(keys[lcg.next()])));
              }
            }
          }

          barrier.wait();
        })
      })
      .collect();

    barrier.wait();
    let start: Instant = Instant::now();
    barrier.wait();
    let elapsed: Duration = start.elapsed();

    for handle in handles {
      handle.join().expect("worker panicked");
    }

    garbage::collect();

    total_ops as f64 / elapsed.as_secs_f64() / 1_000_000.0
  }
}

mod garbage {
  pub fn collect() {
    // -------------------------------------------------------------------------
    // Handle SDD
    // -------------------------------------------------------------------------

    for _ in 0..32 {
      sdd::Guard::new().accelerate();
    }

    // -------------------------------------------------------------------------
    // Handle Crossbeam
    // -------------------------------------------------------------------------

    let mut guard: crossbeam_epoch::Guard = crossbeam_epoch::pin();

    guard.flush();

    for _ in 0..32 {
      guard.repin();
    }
  }
}

// https://developer.apple.com/library/archive/documentation/Performance/Conceptual/EnergyGuide-iOS/PrioritizeWorkWithQoS.html
#[cfg(target_os = "macos")]
mod qos {
  use libc::pthread_set_qos_class_self_np;
  use libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE;

  #[inline]
  pub fn set_realtime() -> bool {
    unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) == 0 }
  }
}

#[cfg(not(target_os = "macos"))]
mod qos {
  #[inline]
  pub fn set_realtime() -> bool {
    false
  }
}
