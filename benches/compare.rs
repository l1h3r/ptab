use divan::Bencher;
use divan::bench;
use divan::bench_group;
use divan::black_box;
use divan::black_box_drop;
use idr_ebr::Idr;
use ptab::Detached;
use sharded_slab::Slab;

type PTab<T> = ptab::PTab<T, ptab::ConstParams<{ 1 << 16 }>>;

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

const THREADS: &[usize] = &[0, 1, 4, 8, 16];

// -----------------------------------------------------------------------------
// Unify APIs for Simplicity
// -----------------------------------------------------------------------------

trait Table<T>: Sized + Send + Sync + 'static
where
  T: Send + Sync + 'static,
{
  type Key: Copy + Send + Sync + 'static;

  fn new() -> Self;

  fn set(&self, value: T) -> Option<Self::Key>;

  fn del(&self, key: Self::Key) -> bool;

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy;
}

impl<T> Table<T> for PTab<T>
where
  T: Send + Sync + 'static,
{
  type Key = Detached;

  fn new() -> Self {
    PTab::new()
  }

  fn set(&self, value: T) -> Option<Self::Key> {
    self.insert(value)
  }

  fn del(&self, key: Self::Key) -> bool {
    self.remove(key)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.read(key)
  }
}

impl<T> Table<T> for Slab<T>
where
  T: Send + Sync + 'static,
{
  type Key = usize;

  fn new() -> Self {
    Slab::new()
  }

  fn set(&self, value: T) -> Option<Self::Key> {
    self.insert(value)
  }

  fn del(&self, key: Self::Key) -> bool {
    self.remove(key)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.get(key).map(|item| *item)
  }
}

impl<T> Table<T> for Idr<T>
where
  T: Send + Sync + 'static,
{
  type Key = idr_ebr::Key;

  fn new() -> Self {
    Idr::new()
  }

  fn set(&self, value: T) -> Option<Self::Key> {
    self.insert(value)
  }

  fn del(&self, key: Self::Key) -> bool {
    self.remove(key)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.get(key, &idr_ebr::EbrGuard::new()).map(|item| *item)
  }
}

// -----------------------------------------------------------------------------
// Actual Benchmarks
// -----------------------------------------------------------------------------

#[bench_group(name = "ReadSeq", skip_ext_time, threads = THREADS)]
mod read_seq {
  use super::bench;
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Table<usize>,
  {
    let this: T = <T as Table<usize>>::new();
    let keys: Vec<T::Key> = (0..ops).map(|index| this.set(index).unwrap()).collect();

    bencher.counter(ops).bench(move || {
      for key in keys.iter() {
        let hkey: T::Key = black_box(*key);
        let item: Option<usize> = black_box(this.get(hkey));
        _ = black_box(item.unwrap());
      }
    });
  }

  #[bench(args = OPS)]
  fn bench_ptab(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<PTab<usize>>(bencher, ops);
  }

  #[bench(args = OPS)]
  fn bench_slab(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<Slab<usize>>(bencher, ops);
  }

  #[bench(args = OPS)]
  fn bench_idr(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<Idr<usize>>(bencher, ops);
  }
}

#[bench_group(name = "ReadHot", skip_ext_time, threads = THREADS)]
mod read_hot {
  use super::bench;
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Table<usize>,
  {
    let this: T = <T as Table<usize>>::new();
    let hkey: T::Key = this.set(0).unwrap();

    bencher.counter(ops).bench(move || {
      for _ in 0..ops {
        let hkey: T::Key = black_box(hkey);
        let item: Option<usize> = black_box(this.get(hkey));
        _ = black_box(item.unwrap());
      }
    });
  }

  #[bench(args = OPS)]
  fn bench_ptab(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<PTab<usize>>(bencher, ops);
  }

  #[bench(args = OPS)]
  fn bench_slab(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<Slab<usize>>(bencher, ops);
  }

  #[bench(args = OPS)]
  fn bench_idr(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<Idr<usize>>(bencher, ops);
  }
}

#[bench_group(name = "InsertSeq", skip_ext_time)]
mod insert_seq {
  use super::bench;
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Table<usize>,
  {
    bencher
      .counter(ops)
      .with_inputs(<T as Table<usize>>::new)
      .bench_local_refs(move |this: &mut T| {
        for index in 0..ops {
          let item: usize = black_box(index);
          let hkey: Option<T::Key> = black_box(this.set(item));
          _ = black_box(hkey.unwrap());
        }
      });
  }

  #[bench(args = OPS)]
  fn bench_ptab(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<PTab<usize>>(bencher, ops);
  }

  #[bench(args = OPS)]
  fn bench_slab(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<Slab<usize>>(bencher, ops);
  }

  #[bench(args = OPS)]
  fn bench_idr(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<Idr<usize>>(bencher, ops);
  }
}

#[bench_group(name = "Churn", skip_ext_time)]
mod churn {
  use super::bench;
  use super::*;

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Table<usize>,
  {
    bencher
      .counter(ops)
      .with_inputs(<T as Table<usize>>::new)
      .bench_local_refs(move |this: &mut T| {
        for index in 0..ops {
          let item: usize = black_box(index);
          let hkey: Option<T::Key> = black_box(this.set(item));
          let gone: bool = black_box(this.del(hkey.unwrap()));
          _ = black_box(gone);
        }
      });
  }

  #[bench(args = OPS)]
  fn bench_ptab(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<PTab<usize>>(bencher, ops);
  }

  #[bench(args = OPS)]
  fn bench_slab(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<Slab<usize>>(bencher, ops);
  }

  #[bench(args = OPS)]
  fn bench_idr(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<Idr<usize>>(bencher, ops);
  }
}

#[bench_group(name = "Drop", skip_ext_time)]
mod drop {
  use super::bench;
  use super::*;

  struct DropMe(usize);

  impl Drop for DropMe {
    fn drop(&mut self) {
      let _ignore: usize = self.0;
    }
  }

  fn bench<T>(bencher: Bencher<'_, '_>, ops: usize)
  where
    T: Table<DropMe>,
  {
    bencher
      .counter(ops)
      .with_inputs(move || {
        let this: T = <T as Table<DropMe>>::new();

        for index in 0..ops {
          let _ignore: T::Key = this.set(DropMe(index)).unwrap();
        }

        this
      })
      .bench_local_values(black_box_drop);
  }

  #[bench(args = OPS)]
  fn bench_ptab(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<PTab<DropMe>>(bencher, ops);
  }

  #[bench(args = OPS)]
  fn bench_slab(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<Slab<DropMe>>(bencher, ops);
  }

  #[bench(args = OPS)]
  fn bench_idr(bencher: Bencher<'_, '_>, ops: usize) {
    bench::<Idr<DropMe>>(bencher, ops);
  }
}

// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

fn main() {
  divan::main();
}
