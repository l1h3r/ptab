use std::fmt::Debug;

use mimalloc::MiMalloc;

use ptab_benchmarks::Collection;
use ptab_benchmarks::Config;
use ptab_benchmarks::Handle;
use ptab_benchmarks::Mix;
use ptab_benchmarks::Stats;
use ptab_benchmarks::Workload;
use ptab_benchmarks::adapter::*;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

// -----------------------------------------------------------------------------
// Config
// -----------------------------------------------------------------------------

const THREADS: &[usize] = &[1, 2, 4, 8];

const MIXES: &[(&str, Mix)] = &[
  ("lookup", Mix::lookup()),
  ("insert", Mix::insert()),
  ("lookup_heavy", Mix::lookup_heavy()),
  ("insert_heavy", Mix::insert_heavy()),
  ("churn", Mix::churn()),
  ("uniform", Mix::uniform()),
  ("insert_remove", Mix::insert_remove()),
];

// -----------------------------------------------------------------------------
// Macros
// -----------------------------------------------------------------------------

macro_rules! run {
  ($name:literal, $collection:ty) => {
    for (scenario, settings) in MIXES {
      for threads in THREADS {
        run::<$collection>($name, scenario, *settings, *threads);
      }
    }

    println!();
  };
}

// -----------------------------------------------------------------------------
// Benchmark Wrapper
// -----------------------------------------------------------------------------

fn run<T>(name: &str, scenario: &str, mix: Mix, threads: usize)
where
  T: Collection<usize>,
  <T::Async as Handle<usize>>::Key: Debug,
{
  let stats: Stats = Workload::new(mix, threads).run::<T>(&Config::default());

  println!(
    "{:>12} | {:>13} | {:>2} threads | {:>8.2} ± {:>5.2} M ops/sec (cv={:.1}%)",
    name,
    scenario,
    threads,
    stats.median,
    stats.stddev,
    stats.cv(),
  );
}

// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

fn main() {
  println!("Workload Benchmarks");
  println!("{}", "=".repeat(80));
  println!();

  // run!("dashmap", DashMap<usize>);
  // run!("flurry", Flurry<usize>);
  // run!("idr", Idr<usize>);
  // run!("papaya", Papaya<usize>);
  run!("ptab", PTab<usize>);
  // run!("sharded_slab", ShardedSlab<usize>);
}
