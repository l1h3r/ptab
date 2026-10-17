use sharded_slab::Slab;

use crate::collect::Collection;
use crate::collect::Handle;
use crate::collect::LocalHandle;

pub struct ShardedSlab<T>(Slab<T>);

impl<T> Collection<T> for ShardedSlab<T>
where
  T: Send + Sync + 'static,
{
  type Local = Self;
  type Async = Self;

  fn new<const C: usize>() -> Self {
    Self(Slab::new())
  }

  fn into_local(self) -> Self::Local {
    self
  }

  fn into_async(self) -> Self::Async {
    self
  }
}

impl<T> LocalHandle<T> for ShardedSlab<T> {
  type Key = usize;

  fn set(&mut self, value: T) -> Option<Self::Key> {
    self.0.insert(value)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.0.get(key).map(|item| *item)
  }

  fn del(&mut self, key: Self::Key) -> bool {
    self.0.remove(key)
  }
}

impl<T> Handle<T> for ShardedSlab<T>
where
  T: Send + Sync + 'static,
{
  type Key = usize;

  fn set(&self, value: T) -> Option<Self::Key> {
    self.0.insert(value)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.0.get(key).map(|item| *item)
  }

  fn del(&self, key: Self::Key) -> bool {
    self.0.remove(key)
  }
}
