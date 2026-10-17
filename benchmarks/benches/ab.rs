//! A/B harness for `ptab`. Driven by `src/bin/ab.rs`; run that instead of this file.
//!
//! Each process runs every cell once and prints one TSV line per cell to stdout:
//! `row<TAB>threads<TAB>ns_per_op<TAB>samples`. Worker QoS goes to stderr. Every sample moves
//! the same total number of operations, so a row's time is comparable across thread counts.
//!
//! Workers are created once per cell and released by a generation counter, so the timed region
//! is the workload alone: no thread spawn, no park/unpark, no rendezvous.

use std::hint::black_box;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use ptab::DefaultParams;
use ptab::Detached;
use ptab::PTab;
use ptab_benchmarks::qos;

type Table<T> = PTab<T, DefaultParams>;

/// Operations per sample, summed over threads. Below the default capacity, so inserts fit.
const TOTAL: usize = 1 << 19;

/// Threads for the multi-thread rows. Counts above the 8 P-cores are reported, not gated.
const FULL_THREADS: &[usize] = &[2, 4, 6, 8, 12];
const QUICK_THREADS: &[usize] = &[6];

const FULL_BUDGET: Duration = Duration::from_millis(1000);
const QUICK_BUDGET: Duration = Duration::from_millis(2000);
const MIN_SAMPLES: usize = 9;

// -----------------------------------------------------------------------------
// Workers
// -----------------------------------------------------------------------------

type Job = Arc<dyn Fn(usize) + Send + Sync>;

struct Shared {
  generation: AtomicUsize,
  finished: AtomicUsize,
  stop: AtomicBool,
  job: Mutex<Option<Job>>,
}

/// Spins for a while, then yields, until `cond` holds.
fn spin_until(mut cond: impl FnMut() -> bool) {
  let mut spins: u32 = 0;

  while !cond() {
    spins = spins.saturating_add(1);

    if spins < 1 << 12 {
      std::hint::spin_loop();
    } else {
      thread::yield_now();
    }
  }
}

fn worker(shared: Arc<Shared>, id: usize) {
  qos::set_realtime();

  let mut seen: usize = 0;

  loop {
    spin_until(|| shared.generation.load(Ordering::Acquire) != seen);
    seen = shared.generation.load(Ordering::Acquire);

    if shared.stop.load(Ordering::Acquire) {
      return;
    }

    let job: Job = shared
      .job
      .lock()
      .unwrap()
      .clone()
      .expect("a job is set before the generation advances");

    job(id);

    shared.finished.fetch_add(1, Ordering::Release);
  }
}

/// A fixed set of workers that run one job per release.
struct Pool {
  threads: usize,
  shared: Arc<Shared>,
  handles: Vec<JoinHandle<()>>,
}

impl Pool {
  fn new(threads: usize) -> Self {
    let shared: Arc<Shared> = Arc::new(Shared {
      generation: AtomicUsize::new(0),
      finished: AtomicUsize::new(0),
      stop: AtomicBool::new(false),
      job: Mutex::new(None),
    });

    let handles: Vec<JoinHandle<()>> = (0..threads)
      .map(|id| {
        let shared: Arc<Shared> = Arc::clone(&shared);
        thread::spawn(move || worker(shared, id))
      })
      .collect();

    Self {
      threads,
      shared,
      handles,
    }
  }

  /// Releases every worker on `job` and returns the time until the last one finishes.
  fn run(&self, job: Job) -> Duration {
    *self.shared.job.lock().unwrap() = Some(job);
    self.shared.finished.store(0, Ordering::Relaxed);

    let start: Instant = Instant::now();
    self.shared.generation.fetch_add(1, Ordering::Release);

    let threads: usize = self.threads;
    spin_until(|| self.shared.finished.load(Ordering::Acquire) == threads);

    start.elapsed()
  }
}

impl Drop for Pool {
  fn drop(&mut self) {
    self.shared.stop.store(true, Ordering::Release);
    self.shared.generation.fetch_add(1, Ordering::Release);

    for handle in self.handles.drain(..) {
      handle.join().unwrap();
    }
  }
}

// -----------------------------------------------------------------------------
// Measurement
// -----------------------------------------------------------------------------

/// Runs `sample` until `budget` is spent (at least `MIN_SAMPLES` times). Returns the median
/// nanoseconds per operation and the sample count.
fn measure(budget: Duration, ops: usize, mut sample: impl FnMut() -> Duration) -> (f64, usize) {
  let deadline: Instant = Instant::now() + budget;
  let mut values: Vec<f64> = Vec::new();

  while values.len() < MIN_SAMPLES || Instant::now() < deadline {
    values.push(sample().as_nanos() as f64 / ops as f64);
  }

  values.sort_by(f64::total_cmp);

  let count: usize = values.len();
  let median: f64 = if count % 2 == 1 {
    values[count / 2]
  } else {
    (values[count / 2 - 1] + values[count / 2]) / 2.0
  };

  (median, count)
}

// -----------------------------------------------------------------------------
// Rows
// -----------------------------------------------------------------------------

fn read_seq(threads: usize, budget: Duration) -> (f64, usize) {
  let table: Arc<Table<usize>> = Arc::new(PTab::new());
  let keys: Arc<[Detached]> = (0..TOTAL)
    .map(|index| table.insert(index).unwrap())
    .collect();

  let pool = Pool::new(threads);
  let per: usize = TOTAL / threads;
  let ops: usize = per * threads;

  measure(budget, ops, || {
    let table: Arc<Table<usize>> = Arc::clone(&table);
    let keys: Arc<[Detached]> = Arc::clone(&keys);

    pool.run(Arc::new(move |tid: usize| {
      for key in &keys[tid * per..(tid + 1) * per] {
        let item: Option<usize> = black_box(table.read(black_box(*key)));
        _ = black_box(item.unwrap());
      }
    }))
  })
}

fn read_hot(threads: usize, budget: Duration) -> (f64, usize) {
  let table: Arc<Table<usize>> = Arc::new(PTab::new());
  let hot: Detached = table.insert(0).unwrap();

  let pool = Pool::new(threads);
  let per: usize = TOTAL / threads;
  let ops: usize = per * threads;

  measure(budget, ops, || {
    let table: Arc<Table<usize>> = Arc::clone(&table);

    pool.run(Arc::new(move |_tid: usize| {
      for _ in 0..per {
        let item: Option<usize> = black_box(table.read(black_box(hot)));
        _ = black_box(item.unwrap());
      }
    }))
  })
}

fn insert_seq(threads: usize, budget: Duration) -> (f64, usize) {
  let pool = Pool::new(threads);
  let per: usize = TOTAL / threads;
  let ops: usize = per * threads;

  measure(budget, ops, || {
    // A fresh table per sample; building it is outside the timed region.
    let table: Arc<Table<usize>> = Arc::new(PTab::new());

    pool.run(Arc::new(move |tid: usize| {
      for index in 0..per {
        let key: Option<Detached> = black_box(table.insert(black_box(tid * per + index)));
        _ = black_box(key.unwrap());
      }
    }))
  })
}

fn churn(threads: usize, budget: Duration) -> (f64, usize) {
  let table: Arc<Table<usize>> = Arc::new(PTab::new());

  let pool = Pool::new(threads);
  let per: usize = TOTAL / threads;
  let ops: usize = per * threads;

  measure(budget, ops, || {
    let table: Arc<Table<usize>> = Arc::clone(&table);

    pool.run(Arc::new(move |tid: usize| {
      for index in 0..per {
        let key: Option<Detached> = black_box(table.insert(black_box(tid * per + index)));
        let _del: bool = black_box(table.remove(key.unwrap()));
      }
    }))
  })
}

/// Drops a full table on the calling thread. Only the drop is timed.
fn drop_table(budget: Duration) -> (f64, usize) {
  struct DropMe(usize);

  impl Drop for DropMe {
    fn drop(&mut self) {
      black_box(self.0);
    }
  }

  measure(budget, TOTAL, || {
    let table: Table<DropMe> = PTab::new();

    for index in 0..TOTAL {
      _ = table.insert(DropMe(index)).unwrap();
    }

    let start: Instant = Instant::now();
    drop(black_box(table));
    start.elapsed()
  })
}

// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

fn report(row: &str, threads: usize, (ns, samples): (f64, usize)) {
  println!("{row}\t{threads}\t{ns:.4}\t{samples}");
}

fn main() {
  let mut quick: bool = false;

  for arg in std::env::args().skip(1) {
    match arg.as_str() {
      "--quick" => quick = true,
      "--full" => quick = false,
      "--bench" => {}
      other => {
        eprintln!("ab: unknown argument `{other}`");
        std::process::exit(2);
      }
    }
  }

  let threads: &[usize] = if quick { QUICK_THREADS } else { FULL_THREADS };
  let budget: Duration = if quick { QUICK_BUDGET } else { FULL_BUDGET };

  // Confirm the scheduler class reaches the workers before any timing happens.
  qos::set_realtime();
  eprintln!("qos main {}", qos::current());

  Pool::new(8).run(Arc::new(|tid: usize| {
    eprintln!("qos worker {tid} {}", qos::current());
  }));

  println!("row\tthreads\tns_per_op\tsamples");

  // One thread first, then each multi-thread count; Drop is single-threaded only.
  for count in std::iter::once(1).chain(threads.iter().copied()) {
    report("ReadSeq", count, read_seq(count, budget));
    report("ReadHot", count, read_hot(count, budget));
    report("InsertSeq", count, insert_seq(count, budget));
    report("Churn", count, churn(count, budget));
  }

  report("Drop", 1, drop_table(budget));
}
