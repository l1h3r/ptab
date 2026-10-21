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
/// # Atomic size
///
/// `Atomic<T>` must have the size and alignment of `usize`, because the table
/// sizes its arrays in `usize` slots. `PTab::new` rejects any other size when
/// the table is built. The check runs at codegen, so `cargo check` does not
/// report it; `cargo build` does. A four-word `Atomic` fails to build:
///
/// ```compile_fail
/// use core::mem::MaybeUninit;
/// use core::sync::atomic::Ordering;
/// use ptab::memory::{Atomic, CollectorWeak, Shared};
/// use ptab::{PTab, Params};
///
/// #[repr(C)]
/// struct Wide([usize; 4]);
/// struct Ptr;
/// struct Weak;
/// struct P;
///
/// unsafe impl<'guard, T> Shared<'guard, T> for Ptr {
///   fn is_null(&self) -> bool { true }
///   fn as_ref(&self) -> Option<&'guard T> { None }
/// }
///
/// unsafe impl<T> Atomic<T> for Wide {
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
/// The same collector with a `usize`-wide `Atomic` builds:
///
/// ```
/// use core::mem::MaybeUninit;
/// use core::sync::atomic::Ordering;
/// use ptab::memory::{Atomic, CollectorWeak, Shared};
/// use ptab::{PTab, Params};
///
/// #[repr(C)]
/// struct Wide(usize);
/// struct Ptr;
/// struct Weak;
/// struct P;
///
/// unsafe impl<'guard, T> Shared<'guard, T> for Ptr {
///   fn is_null(&self) -> bool { true }
///   fn as_ref(&self) -> Option<&'guard T> { None }
/// }
///
/// unsafe impl<T> Atomic<T> for Wide {
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
/// [`Atomic`]: crate::reclaim::CollectorWeak::Atomic
/// [`Atomic::read()`]: crate::reclaim::Atomic::read
/// [`Atomic::evict()`]: crate::reclaim::Atomic::evict
/// [`Collector`]: crate::reclaim::Collector
/// [`flush()`]: crate::reclaim::CollectorWeak::flush
/// [`Guard`]: crate::reclaim::CollectorWeak::Guard
/// [`Shared`]: crate::reclaim::Atomic::Shared
pub trait CollectorWeak {
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
///
/// # Safety
///
/// The table derives its `Send` and `Sync` impls from `T` alone, not from the
/// implementing type. It calls [`evict`] only when `T: Send`, so the collector
/// may drop an evicted value on any thread.
///
/// The table relies on the following. An implementation must uphold all of it.
///
/// - [`read`] returns either null or a pointer to a valid `T` that stays live
///   for the guard's lifetime.
/// - `Self` has the size and alignment of `usize`.
/// - [`evict`] hands the evicted entry to the collector, which does not drop it
///   while a guard that could have read it is live.
/// - When `T: Send + Sync`, every `&self` method is sound to call from several
///   threads at once, whatever auto traits `Self` has.
/// - When `T: Send`, a value of `Self` is sound to move to and drop on another
///   thread, whatever auto traits `Self` has. `Self` adds no thread affinity.
///
/// A safe `impl` is rejected, because the trait is `unsafe`:
///
/// ```compile_fail,E0200
/// use core::mem::MaybeUninit;
/// use core::sync::atomic::Ordering;
/// use ptab::memory::{Atomic, Shared};
///
/// struct Ptr;
/// struct Slot;
///
/// unsafe impl<'guard, T> Shared<'guard, T> for Ptr {
///   fn is_null(&self) -> bool { true }
///   fn as_ref(&self) -> Option<&'guard T> { None }
/// }
///
/// impl Atomic<u8> for Slot {
///   type Guard = ();
///   type Shared<'guard> = Ptr where T: 'guard;
///   fn null() -> Self { Slot }
///   fn read<'guard>(&self, _: Ordering, _: &'guard ()) -> Ptr { Ptr }
///   fn write(&self, _: Ordering, _: impl FnOnce(&mut MaybeUninit<T>)) where T: 'static {}
///   fn evict(&self, _: Ordering) -> bool { false }
///   unsafe fn clear(&mut self) -> bool { false }
/// }
/// ```
///
/// A bare [`AtomicPtr`] does not implement it:
///
/// ```compile_fail,E0277
/// use core::sync::atomic::{AtomicPtr, Ordering};
/// use ptab::memory::Atomic;
///
/// let ptr: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
/// let _ = Atomic::read(&ptr, Ordering::Acquire, &());
/// ```
///
/// [`read`]: Self::read
/// [`evict`]: Self::evict
/// [`AtomicPtr`]: core::sync::atomic::AtomicPtr
pub unsafe trait Atomic<T> {
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
///
/// # Safety
///
/// [`as_ref`] returns either `None` or a reference to a valid `T` that is live
/// for `'guard`, and it returns `None` exactly when [`is_null`] is `true`.
///
/// A safe `impl` is rejected, because the trait is `unsafe`:
///
/// ```compile_fail,E0200
/// use ptab::memory::Shared;
///
/// struct Ptr;
///
/// impl<'guard> Shared<'guard, u8> for Ptr {
///   fn is_null(&self) -> bool { true }
///   fn as_ref(&self) -> Option<&'guard u8> { None }
/// }
/// ```
///
/// [`as_ref`]: Self::as_ref
/// [`is_null`]: Self::is_null
pub unsafe trait Shared<'guard, T> {
  /// Returns `true` if the pointer is null.
  fn is_null(&self) -> bool;

  /// Returns a shared reference to the value.
  fn as_ref(&self) -> Option<&'guard T>;
}
