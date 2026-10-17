use parking_lot::RwLock;
use std::sync::Arc;

use crate::collect::Collection;
use crate::collect::LocalHandle;

pub struct Slab<T>(slab::Slab<T>);

impl<T> Collection<T> for Slab<T>
where
  T: Send + Sync + 'static,
{
  type Local = Self;
  type Async = Arc<RwLock<Self>>;

  fn new<const C: usize>() -> Self {
    Self(slab::Slab::with_capacity(C))
  }

  fn into_local(self) -> Self::Local {
    self
  }

  fn into_async(self) -> Self::Async {
    Arc::new(RwLock::new(self))
  }
}

impl<T> LocalHandle<T> for Slab<T> {
  type Key = usize;

  fn set(&mut self, value: T) -> Option<Self::Key> {
    Some(self.0.insert(value))
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.0.get(key).copied()
  }

  fn del(&mut self, key: Self::Key) -> bool {
    if self.0.contains(key) {
      self.0.remove(key);
      true
    } else {
      false
    }
  }
}
