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
/// This trait defines the minimal interface required to safely manage
/// concurrent access to shared atomic pointers. It does *not* guarantee that
/// entries removed via [`Atomic::evict()`] are eventually reclaimed.
///
/// Implementations are permitted to permanently leak evicted entries. If
/// eventual reclamation is required, see [`Collector`].
///
/// # Overview
///
/// A `CollectorWeak` provides:
///
/// - A [`Guard`] type that pins the current thread.
/// - An [`Atomic`] pointer abstraction tied to that guard.
/// - A mechanism ([`flush()`]) to attempt reclamation.
///
/// The collector is responsible for ensuring that:
///
/// - A value read through [`Atomic::read()`] remains valid for the lifetime of
///   the associated guard.
/// - Values are not reclaimed while any guard that could observe them remains
///   active.
///
/// # Guard
///
/// A guard represents a critical section during which shared pointers may be
/// safely dereferenced. While a guard is alive:
///
/// - Reclamation of objects reachable by that guard must be deferred.
/// - Returned [`Shared`] pointers are guaranteed valid for the guard lifetime.
///
/// [`Atomic`]: crate::reclaim::CollectorWeak::Atomic
/// [`Atomic::read()`]: crate::reclaim::Atomic::read
/// [`Atomic::evict()`]: crate::reclaim::Atomic::evict
/// [`Collector`]: crate::reclaim::Collector
/// [`flush()`]: crate::reclaim::CollectorWeak::flush
/// [`Guard`]: crate::reclaim::CollectorWeak::Guard
/// [`Shared`]: crate::reclaim::Atomic::Shared
pub trait CollectorWeak {
  #[doc(hidden)]
  const ASSERT_ATOMIC: () = {
    assert!(align_of::<Self::Atomic<()>>() == align_of::<usize>());
    assert!(size_of::<Self::Atomic<()>>() == size_of::<usize>());
  };

  /// A guard that keeps the current thread pinned.
  type Guard;

  /// An atomic pointer that can be safely shared between threads.
  type Atomic<T>: Atomic<T, Guard = Self::Guard>;

  /// Creates a new `Guard`, pinning the current thread.
  fn guard() -> Self::Guard;

  /// Attempts to trigger memory reclamation.
  ///
  /// This function may:
  ///
  /// - Advance global epochs.
  /// - Drain deferred reclamation queues.
  /// - Execute destructors of previously evicted objects.
  /// - Perform no work at all.
  ///
  /// # Guarantees
  ///
  /// - This function is best-effort.
  /// - It does **not** guarantee that reclamation will occur.
  /// - It does **not** guarantee progress.
  fn flush();
}

// -----------------------------------------------------------------------------
// Atomic Ptr
// -----------------------------------------------------------------------------

/// An atomic pointer that can be safely shared between threads.
pub trait Atomic<T> {
  /// A guard that keeps the current thread pinned.
  type Guard;

  /// A pointer to an object protected by the epoch GC.
  ///
  /// The pointer is valid for use only during the lifetime `'guard`.
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

  /// Swaps `null` into the pointer and schedules the previous entry for
  /// reclamation.
  ///
  /// If the pointer was non-null, its previous value is removed and handed to
  /// the collector. Reclamation is deferred according to the collector's
  /// strategy.
  ///
  /// Returns `true` if a non-null value was removed, or `false` if the pointer
  /// was already null.
  ///
  /// # Reclamation
  ///
  /// - For collectors implementing [`Collector`], a successfully evicted value
  ///   is guaranteed to be eventually reclaimed.
  /// - For [`CollectorWeak`] implementations, the value may be permanently
  ///   leaked.
  ///
  /// # Concurrency
  ///
  /// The removed value may remain accessible to readers that hold a live
  /// `Guard`. Implementations must ensure that reclamation is deferred until
  /// deemed safe to destroy the value.
  fn evict(&self, order: Ordering) -> bool;

  /// Executes the destructor (if any) of the pointed-to value.
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

/// A pointer to an object protected by the epoch GC.
///
/// The pointer is valid for use only during the lifetime `'guard`.
pub trait Shared<'guard, T> {
  /// Returns `true` if the pointer is null.
  fn is_null(&self) -> bool;

  /// Returns a shared reference to the value.
  fn as_ref(&self) -> Option<&'guard T>;
}
