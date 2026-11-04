use core::mem::MaybeUninit;
use core::sync::atomic::Ordering;

// -----------------------------------------------------------------------------
// Collector API
// -----------------------------------------------------------------------------

/// A memory reclamation strategy that guarantees eventual reclamation of
/// evicted atomic entries.
///
/// # Safety
///
/// This is an **unsafe marker trait** extending [`CollectorWeak`]. By
/// implementing this trait, a collector makes the following guarantee:
///
/// > Any value removed via [`Atomic::evict()`] will eventually have its
/// > destructor run and its memory reclaimed, assuming the program continues to
/// > make progress.
///
/// In particular, implementations of `Collector` **must not permanently leak**
/// entries that were successfully evicted. Temporary deferral of reclamation
/// (e.g. due to pinned threads or epoch lag) is expected, but reclamation must
/// occur once it is safe to do so.
///
/// [`Atomic::evict()`]: crate::reclaim::Atomic::evict
/// [`CollectorWeak`]: crate::reclaim::CollectorWeak
pub unsafe trait Collector: CollectorWeak {}

/// A memory reclamation strategy.
///
/// Evicted values may leak. A [`Collector`] frees them eventually. A value read
/// through [`Atomic::read()`] stays valid while its guard lives.
///
/// # Atomic size
///
/// `Atomic<T>` must have the size and alignment of `usize`, or the table fails
/// to compile:
///
/// ```compile_fail
/// use core::mem::MaybeUninit;
/// use core::sync::atomic::Ordering;
/// use ptab::memory::{Atomic, CollectorWeak, Shared};
/// use ptab::{PTab, Params};
///
/// struct Wide([usize; 4]);
/// struct Ptr;
/// struct Weak;
/// struct P;
///
/// impl<'guard, T> Shared<'guard, T> for Ptr {
///   fn is_null(&self) -> bool { true }
///   fn as_ref(&self) -> Option<&'guard T> { None }
/// }
///
/// impl<T> Atomic<T> for Wide {
///   type Guard = ();
///   type Shared<'guard> = Ptr where T: 'guard;
///   fn null() -> Self { Wide([0; 4]) }
///   fn read<'guard>(&self, _: Ordering, _: &'guard ()) -> Ptr { Ptr }
///   fn write(&self, _: Ordering, _: impl FnOnce(&mut MaybeUninit<T>)) where T: 'static {}
///   fn evict(&self, _: Ordering) -> bool { false }
///   unsafe fn clear(&mut self) -> bool { false }
/// }
///
/// impl CollectorWeak for Weak {
///   type Guard = ();
///   type Atomic<T> = Wide;
///   fn guard() {}
///   fn flush() {}
/// }
///
/// impl Params for P {
///   type Collector = Weak;
/// }
///
/// let _table: PTab<u8, P> = PTab::new();
/// ```
///
/// With a `usize`-sized `Atomic` it compiles:
///
/// ```
/// use core::mem::MaybeUninit;
/// use core::sync::atomic::Ordering;
/// use ptab::memory::{Atomic, CollectorWeak, Shared};
/// use ptab::{PTab, Params};
///
/// struct Wide(usize);
/// struct Ptr;
/// struct Weak;
/// struct P;
///
/// impl<'guard, T> Shared<'guard, T> for Ptr {
///   fn is_null(&self) -> bool { true }
///   fn as_ref(&self) -> Option<&'guard T> { None }
/// }
///
/// impl<T> Atomic<T> for Wide {
///   type Guard = ();
///   type Shared<'guard> = Ptr where T: 'guard;
///   fn null() -> Self { Wide(0) }
///   fn read<'guard>(&self, _: Ordering, _: &'guard ()) -> Ptr { Ptr }
///   fn write(&self, _: Ordering, _: impl FnOnce(&mut MaybeUninit<T>)) where T: 'static {}
///   fn evict(&self, _: Ordering) -> bool { false }
///   unsafe fn clear(&mut self) -> bool { false }
/// }
///
/// impl CollectorWeak for Weak {
///   type Guard = ();
///   type Atomic<T> = Wide;
///   fn guard() {}
///   fn flush() {}
/// }
///
/// impl Params for P {
///   type Collector = Weak;
/// }
///
/// let _table: PTab<u8, P> = PTab::new();
/// ```
///
/// [`Atomic::read()`]: crate::reclaim::Atomic::read
/// [`Collector`]: crate::reclaim::Collector
pub trait CollectorWeak {
  /// Ensure `Atomic<T>` has the same size and alignment as a raw pointer.
  #[doc(hidden)]
  const ASSERT_ATOMIC: () = {
    assert!(align_of::<Self::Atomic<()>>() == align_of::<usize>());
    assert!(size_of::<Self::Atomic<()>>() == size_of::<usize>());
  };

  /// A guard that keeps the current thread pinned.
  type Guard;

  /// The slot type.
  type Atomic<T>: Atomic<T, Guard = Self::Guard>;

  /// Creates a new `Guard`, pinning the current thread.
  fn guard() -> Self::Guard;

  /// Reclaims any evicted values it can, which may be none.
  fn flush();
}

// -----------------------------------------------------------------------------
// Atomic Ptr
// -----------------------------------------------------------------------------

/// An atomic pointer that can be safely shared between threads.
pub trait Atomic<T> {
  /// The guard [`read()`] takes.
  ///
  /// [`read()`]: Self::read
  type Guard;

  /// The pointer [`read()`] returns.
  ///
  /// [`read()`]: Self::read
  type Shared<'guard>: Shared<'guard, T>
  where
    T: 'guard;

  /// Creates a null atomic pointer.
  fn null() -> Self;

  /// Loads a value from the pointer.
  fn read<'guard>(&self, order: Ordering, guard: &'guard Self::Guard) -> Self::Shared<'guard>;

  /// Initializes and stores a value into the pointer.
  fn write(&self, order: Ordering, init: impl FnOnce(&mut MaybeUninit<T>))
  where
    T: 'static;

  /// Swaps in null and hands the previous value to the collector.
  ///
  /// Returns `true` if the previous value was non-null. Readers that hold a
  /// guard can still read the old value.
  fn evict(&self, order: Ordering) -> bool;

  /// Drops and frees the value, if any. Returns `true` if there was one.
  ///
  /// # Safety
  ///
  /// See [`ptr::drop_in_place`] for minimum safety requirements.
  ///
  /// [`ptr::drop_in_place`]: core::ptr::drop_in_place
  unsafe fn clear(&mut self) -> bool;
}

// -----------------------------------------------------------------------------
// Shared Ptr
// -----------------------------------------------------------------------------

/// A pointer read from an [`Atomic`], valid for `'guard`.
pub trait Shared<'guard, T> {
  /// Returns `true` if the pointer is null.
  fn is_null(&self) -> bool;

  /// Returns a shared reference to the value.
  fn as_ref(&self) -> Option<&'guard T>;
}
