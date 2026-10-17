use parking_lot::RwLock;
use std::sync::Arc;

pub trait Collection<T> {
  type Local: LocalHandle<T>;
  type Async: Handle<T>;

  fn new<const C: usize>() -> Self;
  fn into_local(self) -> Self::Local;
  fn into_async(self) -> Self::Async;
}

pub trait LocalHandle<T> {
  type Key: Copy + Send + Sync + 'static;

  fn set(&mut self, value: T) -> Option<Self::Key>;

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy;

  fn del(&mut self, key: Self::Key) -> bool;
}

pub trait Handle<T>: Send + Sync + 'static {
  type Key: Copy + Send + Sync + 'static;

  fn set(&self, value: T) -> Option<Self::Key>;

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy;

  fn del(&self, key: Self::Key) -> bool;
}

impl<T, U> Handle<T> for Arc<RwLock<U>>
where
  U: Send + Sync + 'static + LocalHandle<T>,
{
  type Key = U::Key;

  fn set(&self, value: T) -> Option<Self::Key> {
    self.write().set(value)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.read().get(key)
  }

  fn del(&self, key: Self::Key) -> bool {
    self.write().del(key)
  }
}
