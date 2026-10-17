use ptab_develop::DefaultParams;
use ptab_develop::Detached;

use crate::collect::Collection;
use crate::collect::Handle;
use crate::collect::LocalHandle;

pub struct PTabDevelop<T>(ptab_develop::PTab<T, DefaultParams>);

impl<T> Collection<T> for PTabDevelop<T>
where
  T: Send + Sync + 'static,
{
  type Local = Self;
  type Async = Self;

  fn new<const C: usize>() -> Self {
    Self(ptab_develop::PTab::new())
  }

  fn into_local(self) -> Self::Local {
    self
  }

  fn into_async(self) -> Self::Async {
    self
  }
}

impl<T> LocalHandle<T> for PTabDevelop<T>
where
  T: 'static,
{
  type Key = Detached;

  fn set(&mut self, value: T) -> Option<Self::Key> {
    self.0.insert(value)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.0.read(key)
  }

  fn del(&mut self, key: Self::Key) -> bool {
    self.0.remove(key)
  }
}

impl<T> Handle<T> for PTabDevelop<T>
where
  T: Send + Sync + 'static,
{
  type Key = Detached;

  fn set(&self, value: T) -> Option<Self::Key> {
    self.0.insert(value)
  }

  fn get(&self, key: Self::Key) -> Option<T>
  where
    T: Copy,
  {
    self.0.read(key)
  }

  fn del(&self, key: Self::Key) -> bool {
    self.0.remove(key)
  }
}
