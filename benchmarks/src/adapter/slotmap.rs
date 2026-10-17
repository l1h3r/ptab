use parking_lot::RwLock;
use slotmap::DefaultKey;
use std::sync::Arc;

use crate::collect::Collection;
use crate::collect::LocalHandle;

pub struct SlotMap<T>(slotmap::SlotMap<DefaultKey, T>);

impl<T> Collection<T> for SlotMap<T>
where
  T: Send + Sync + 'static,
{
  type Local = Self;
  type Async = Arc<RwLock<Self>>;

  fn new<const C: usize>() -> Self {
    Self(slotmap::SlotMap::with_capacity(C))
  }

  fn into_local(self) -> Self::Local {
    self
  }

  fn into_async(self) -> Self::Async {
    Arc::new(RwLock::new(self))
  }
}

impl<T> LocalHandle<T> for SlotMap<T> {
  type Key = DefaultKey;

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
    self.0.remove(key).is_some()
  }
}
