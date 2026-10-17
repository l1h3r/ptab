use idr_ebr::EbrGuard;
use idr_ebr::Key;

use crate::collect::Collection;
use crate::collect::Handle;
use crate::collect::LocalHandle;

pub struct Idr<T>(idr_ebr::Idr<T>);

impl<T> Collection<T> for Idr<T>
where
  T: Send + Sync + 'static,
{
  type Local = Self;
  type Async = Self;

  fn new<const C: usize>() -> Self {
    Self(idr_ebr::Idr::new())
  }

  fn into_local(self) -> Self::Local {
    self
  }

  fn into_async(self) -> Self::Async {
    self
  }
}

impl<T> LocalHandle<T> for Idr<T>
where
  T: 'static,
{
  type Key = Key;

  fn set(&mut self, value: T) -> Option<Self::Key> {
    self.0.insert(value)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.0.get(key, &EbrGuard::new()).map(|item| *item)
  }

  fn del(&mut self, key: Self::Key) -> bool {
    self.0.remove(key)
  }
}

impl<T> Handle<T> for Idr<T>
where
  T: Send + 'static,
{
  type Key = Key;

  fn set(&self, value: T) -> Option<Self::Key> {
    self.0.insert(value)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.0.get(key, &EbrGuard::new()).map(|item| *item)
  }

  fn del(&self, key: Self::Key) -> bool {
    self.0.remove(key)
  }
}
