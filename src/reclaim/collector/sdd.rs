use core::hint;
use core::mem;
use core::mem::MaybeUninit;

use crate::reclaim::Atomic;
use crate::reclaim::Collector;
use crate::reclaim::CollectorWeak;
use crate::reclaim::Shared;
use crate::sync::atomic::Ordering;

const NO_TAG: sdd::Tag = sdd::Tag::None;

/// A reclamation strategy based on [`sdd`].
///
/// [`sdd`]: https://crates.io/crates/sdd
pub enum Sdd {}

unsafe impl Collector for Sdd {}

impl CollectorWeak for Sdd {
  type Guard = sdd::Guard;
  type Atomic<T> = sdd::AtomicOwned<T>;

  #[inline]
  fn guard() -> Self::Guard {
    sdd::Guard::new()
  }

  #[inline]
  fn flush() {
    // Taking a guard `EPOCH` times moves this thread three epochs past the
    // epoch that retired a value, even from one epoch behind the global epoch.
    //
    // ref: sdd-4.8.11 src/guard.rs:47
    const EPOCH: usize = 4;

    for _ in 0..EPOCH {
      Self::guard().accelerate();
    }
  }
}

// -----------------------------------------------------------------------------
// Atomic ptr
// -----------------------------------------------------------------------------

impl<T> Atomic<T> for sdd::AtomicOwned<T> {
  type Guard = sdd::Guard;

  #[rustfmt::skip]
  type Shared<'guard> = sdd::Ptr<'guard, T>
  where
    T: 'guard;

  #[inline]
  fn null() -> Self {
    Self::null()
  }

  #[inline]
  fn read<'guard>(&self, order: Ordering, guard: &'guard Self::Guard) -> Self::Shared<'guard> {
    self.load(order, guard)
  }

  #[inline]
  fn write(&self, order: Ordering, init: impl FnOnce(&mut MaybeUninit<T>))
  where
    T: 'static,
  {
    let value: sdd::Owned<T> = sdd::Owned::new_with(|| {
      let mut uninit: MaybeUninit<T> = MaybeUninit::uninit();

      init(&mut uninit);

      // SAFETY:
      // - the `init` closure must fully initialize `uninit`
      // - after `init` returns, this code treats the value as initialized
      unsafe { uninit.assume_init() }
    });

    let old: (Option<sdd::Owned<T>>, sdd::Tag) = self.swap((Some(value), NO_TAG), order);

    debug_assert!(old.0.is_none(), "`Atomic<T>` is occupied");
    debug_assert!(old.1 == NO_TAG, "`Atomic<T>` is tagged");

    // SAFETY:
    // - by slot allocation, the slot is empty before `write` runs
    // - so `old.0` must be `None` and `old.1` must be `Tag::None`
    unsafe {
      hint::assert_unchecked(old.0.is_none());
      hint::assert_unchecked(old.1 == NO_TAG);
    }
  }

  #[inline]
  fn evict(&self, order: Ordering) -> bool {
    self.swap((None, NO_TAG), order).0.is_some()
  }

  #[inline]
  unsafe fn clear(&mut self) -> bool {
    let entry: Self = mem::take(self);

    if let Some(value) = entry.into_owned(Ordering::Relaxed) {
      // SAFETY:
      // - `entry.into_owned` transfers exclusive ownership of the value
      // - if `into_owned` returns `Some(value)`, this function is the unique owner
      // - calling `drop_in_place` drops the contained `T` exactly once and
      //   reclaims the associated allocation
      unsafe {
        value.drop_in_place();
      }

      true
    } else {
      false
    }
  }
}

// -----------------------------------------------------------------------------
// Shared ptr
// -----------------------------------------------------------------------------

impl<'guard, T> Shared<'guard, T> for sdd::Ptr<'guard, T> {
  #[inline]
  fn is_null(&self) -> bool {
    self.is_null()
  }

  #[inline]
  fn as_ref(&self) -> Option<&'guard T> {
    // SAFETY:
    // - `sdd::Ptr::as_ref_unchecked` requires that `self` has no tag bits
    // - `Atomic::null`, `Atomic::write` and `Atomic::evict` store `Tag::None`,
    //   so `self` has no tag bits
    // - `self` is either null or points to the `T` that `sdd::Owned::new_with`
    //   allocated in `Atomic::write`, and the guard pins it for `'guard`
    // - this code creates only shared references to `T`, so it violates no
    //   aliasing rule
    unsafe { self.as_ref_unchecked() }
  }
}
