use papaya::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use crate::collect::Collection;
use crate::collect::Handle;
use crate::collect::LocalHandle;

pub struct Papaya<T> {
  map: HashMap<usize, T>,
  key: AtomicUsize,
}

impl<T> Collection<T> for Papaya<T>
where
  T: Send + Sync + 'static,
{
  type Local = Self;
  type Async = Self;

  fn new<const C: usize>() -> Self {
    Self {
      map: HashMap::with_capacity(C),
      key: AtomicUsize::new(0),
    }
  }

  fn into_local(self) -> Self::Local {
    self
  }

  fn into_async(self) -> Self::Async {
    self
  }
}

impl<T> LocalHandle<T> for Papaya<T> {
  type Key = usize;

  fn set(&mut self, value: T) -> Option<Self::Key> {
    let key: usize = self.key.fetch_add(1, Ordering::Relaxed);
    self.map.pin().insert(key, value);
    Some(key)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.map.pin().get(&key).map(|item| *item)
  }

  fn del(&mut self, key: Self::Key) -> bool {
    self.map.pin().remove(&key).is_some()
  }
}

impl<T> Handle<T> for Papaya<T>
where
  T: Send + Sync + 'static,
{
  type Key = usize;

  fn set(&self, value: T) -> Option<Self::Key> {
    let key: usize = self.key.fetch_add(1, Ordering::Relaxed);
    self.map.pin().insert(key, value);
    Some(key)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.map.pin().get(&key).map(|item| *item)
  }

  fn del(&self, key: Self::Key) -> bool {
    self.map.pin().remove(&key).is_some()
  }
}
