use core::mem::MaybeUninit;
use core::sync::atomic::Ordering;

// -----------------------------------------------------------------------------
// Collector API
// -----------------------------------------------------------------------------

/// A memory reclamation strategy that frees every evicted entry eventually.
///
/// # Safety
///
/// This trait extends [`CollectorWeak`]. An implementation must run the
/// destructor of any value removed via [`Atomic::evict()`] and free its memory,
/// assuming the program makes progress.
///
/// An implementation must not permanently leak an entry that
/// [`Atomic::evict()`] removed successfully. An implementation may defer
/// reclamation for a pinned thread or an epoch lag, and reclaims once
/// reclamation is safe.
///
/// # Examples
///
/// ```
/// # #[cfg(feature = "sdd")]
/// # {
/// use ptab::memory::collector::Sdd;
/// use ptab::memory::{Collector, CollectorWeak};
///
/// struct Mine;
///
/// impl CollectorWeak for Mine {
///   type Guard = <Sdd as CollectorWeak>::Guard;
///   type Atomic<T> = <Sdd as CollectorWeak>::Atomic<T>;
///   fn guard() -> Self::Guard { Sdd::guard() }
///   fn flush() { Sdd::flush() }
/// }
///
/// // SAFETY: `Mine` delegates every operation to `Sdd`, which frees evicted
/// //         entries.
/// unsafe impl Collector for Mine {}
/// # }
/// ```
///
/// [`Atomic::evict()`]: crate::reclaim::Atomic::evict
/// [`CollectorWeak`]: crate::reclaim::CollectorWeak
pub unsafe trait Collector: CollectorWeak {}

/// A memory reclamation strategy.
///
/// Evicted values may leak. A [`Collector`] frees them eventually. A value read
/// through [`Atomic::read()`] is valid until its guard drops.
///
/// `Atomic<T>` must have the size and alignment of `usize`, or the table fails
/// to compile:
///
/// ```compile_fail,E0080
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
/// # Examples
///
/// ```
/// use core::sync::atomic::AtomicPtr;
/// use ptab::memory::CollectorWeak;
/// use ptab::{PTab, Params};
///
/// struct Mine;
///
/// impl CollectorWeak for Mine {
///   type Guard = ();
///   type Atomic<T> = AtomicPtr<T>;
///   fn guard() {}
///   fn flush() {}
/// }
///
/// struct P;
///
/// impl Params for P {
///   type Collector = Mine;
/// }
///
/// let table: PTab<u8, P> = PTab::new();
/// assert!(table.insert(1).is_some());
/// ```
///
/// [`Atomic::read()`]: crate::reclaim::Atomic::read
/// [`Collector`]: crate::reclaim::Collector
pub trait CollectorWeak {
  /// Asserts that `Atomic<T>` has the same size and alignment as a raw pointer.
  #[doc(hidden)]
  const ASSERT_ATOMIC: () = {
    assert!(align_of::<Self::Atomic<()>>() == align_of::<usize>());
    assert!(size_of::<Self::Atomic<()>>() == size_of::<usize>());
  };

  /// A guard that pins the current thread.
  type Guard;

  /// The slot type.
  type Atomic<T>: Atomic<T, Guard = Self::Guard>;

  /// Creates a new `Guard`, pinning the current thread.
  fn guard() -> Self::Guard;

  /// Reclaims any evicted values it can, which may be none.
  fn flush();
}

// -----------------------------------------------------------------------------
// Atomic ptr
// -----------------------------------------------------------------------------

/// An atomic pointer that can be safely shared between threads.
///
/// # Examples
///
/// ```
/// use core::mem::MaybeUninit;
/// use core::sync::atomic::Ordering;
/// use ptab::memory::{Atomic, Shared};
///
/// struct Null;
///
/// impl<'guard, T> Shared<'guard, T> for Null {
///   fn is_null(&self) -> bool { true }
///   fn as_ref(&self) -> Option<&'guard T> { None }
/// }
///
/// struct Never(usize);
///
/// impl<T> Atomic<T> for Never {
///   type Guard = ();
///   type Shared<'guard> = Null where T: 'guard;
///   fn null() -> Self { Never(0) }
///   fn read<'guard>(&self, _: Ordering, _: &'guard ()) -> Null { Null }
///   fn write(&self, _: Ordering, _: impl FnOnce(&mut MaybeUninit<T>)) where T: 'static {}
///   fn evict(&self, _: Ordering) -> bool { false }
///   unsafe fn clear(&mut self) -> bool { false }
/// }
///
/// let never: Never = Atomic::<u8>::null();
/// let shared = Atomic::<u8>::read(&never, Ordering::Relaxed, &());
/// assert!(Shared::<u8>::is_null(&shared));
/// ```
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
  /// Returns `true` if the previous value was non-null. A reader with a guard
  /// can still read the old value.
  fn evict(&self, order: Ordering) -> bool;

  /// Drops and frees the value, if any. Returns `true` if there was one.
  ///
  /// # Safety
  ///
  /// See [`ptr::drop_in_place()`] for the safety requirements.
  ///
  /// [`ptr::drop_in_place()`]: core::ptr::drop_in_place
  unsafe fn clear(&mut self) -> bool;
}

// -----------------------------------------------------------------------------
// Shared ptr
// -----------------------------------------------------------------------------

/// A pointer read from an [`Atomic`], valid for `'guard`.
///
/// # Examples
///
/// ```
/// use ptab::memory::Shared;
///
/// struct Null;
///
/// impl<'guard, T> Shared<'guard, T> for Null {
///   fn is_null(&self) -> bool { true }
///   fn as_ref(&self) -> Option<&'guard T> { None }
/// }
///
/// assert!(Shared::<u8>::is_null(&Null));
/// ```
///
/// [`Atomic`]: crate::reclaim::Atomic
pub trait Shared<'guard, T> {
  /// Returns `true` if the pointer is null.
  fn is_null(&self) -> bool;

  /// Returns a shared reference to the value.
  fn as_ref(&self) -> Option<&'guard T>;
}
