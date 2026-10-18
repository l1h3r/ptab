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
/// # Atomic Size
///
/// Every `Atomic<T>` is stored in a table array sized for `usize` elements, so
/// it must have the same size and alignment as `usize`. A collector whose
/// `Atomic<T>` is wider fails to build a table:
///
/// ```compile_fail
/// use core::marker::PhantomData;
/// use core::mem::MaybeUninit;
/// use core::ptr;
/// use core::sync::atomic::AtomicPtr;
/// use core::sync::atomic::Ordering;
///
/// use ptab::PTab;
/// use ptab::config::Capacity;
/// use ptab::config::Params;
/// use ptab::memory::Atomic;
/// use ptab::memory::CollectorWeak;
/// use ptab::memory::Shared;
///
/// pub struct Ptr<'guard, T> {
///   pointer: *mut T,
///   marker: PhantomData<&'guard T>,
/// }
///
/// // SAFETY: `pointer` is null or a leaked `Box<T>`, so it stays valid for
/// // `'guard`.
/// unsafe impl<'guard, T> Shared<'guard, T> for Ptr<'guard, T> {
///   fn is_null(&self) -> bool {
///     self.pointer.is_null()
///   }
///
///   fn as_ref(&self) -> Option<&'guard T> {
///     // SAFETY: see the impl-level comment above.
///     unsafe { self.pointer.as_ref() }
///   }
/// }
///
/// #[allow(dead_code, reason = "only the size matters")]
/// pub struct Wide<T>(AtomicPtr<T>, [usize; 3]);
///
/// // SAFETY: this collector never frees a pointer it has stored.
/// unsafe impl<T> Atomic<T> for Wide<T> {
///   type Guard = ();
///
///   type Shared<'guard> = Ptr<'guard, T>
///   where
///     T: 'guard;
///
///   fn null() -> Self {
///     Self(AtomicPtr::new(ptr::null_mut()), [0; 3])
///   }
///
///   fn read<'guard>(&self, order: Ordering, _guard: &'guard Self::Guard) -> Self::Shared<'guard> {
///     Ptr { pointer: self.0.load(order), marker: PhantomData }
///   }
///
///   fn write(&self, order: Ordering, init: impl FnOnce(&mut MaybeUninit<T>))
///   where
///     T: 'static,
///   {
///     let mut uninit: Box<MaybeUninit<T>> = Box::new_uninit();
///
///     init(&mut uninit);
///
///     // SAFETY: `init` is required to fully initialize `uninit`.
///     self.0.store(Box::into_raw(unsafe { uninit.assume_init() }), order);
///   }
///
///   fn evict(&self, order: Ordering) -> bool {
///     !self.0.swap(ptr::null_mut(), order).is_null()
///   }
///
///   unsafe fn clear(&mut self) -> bool {
///     !self.0.swap(ptr::null_mut(), Ordering::Relaxed).is_null()
///   }
/// }
///
/// pub enum Coll {}
///
/// impl CollectorWeak for Coll {
///   type Guard = ();
///   type Atomic<T> = Wide<T>;
///
///   fn guard() -> Self::Guard {}
///
///   fn flush() {}
/// }
///
/// pub enum Custom {}
///
/// impl Params for Custom {
///   type Collector = Coll;
///
///   const LENGTH: Capacity = Capacity::MIN;
/// }
///
/// let table: PTab<u32, Custom> = PTab::new();
/// assert!(table.is_empty());
/// ```
///
/// The same collector with a one-word `Atomic<T>` builds and runs:
///
/// ```
/// use core::marker::PhantomData;
/// use core::mem::MaybeUninit;
/// use core::ptr;
/// use core::sync::atomic::AtomicPtr;
/// use core::sync::atomic::Ordering;
///
/// use ptab::PTab;
/// use ptab::config::Capacity;
/// use ptab::config::Params;
/// use ptab::memory::Atomic;
/// use ptab::memory::CollectorWeak;
/// use ptab::memory::Shared;
///
/// pub struct Ptr<'guard, T> {
///   pointer: *mut T,
///   marker: PhantomData<&'guard T>,
/// }
///
/// // SAFETY: `pointer` is null or a leaked `Box<T>`, so it stays valid for
/// // `'guard`.
/// unsafe impl<'guard, T> Shared<'guard, T> for Ptr<'guard, T> {
///   fn is_null(&self) -> bool {
///     self.pointer.is_null()
///   }
///
///   fn as_ref(&self) -> Option<&'guard T> {
///     // SAFETY: see the impl-level comment above.
///     unsafe { self.pointer.as_ref() }
///   }
/// }
///
/// pub struct Narrow<T>(AtomicPtr<T>);
///
/// // SAFETY: this collector never frees a pointer it has stored.
/// unsafe impl<T> Atomic<T> for Narrow<T> {
///   type Guard = ();
///
///   type Shared<'guard> = Ptr<'guard, T>
///   where
///     T: 'guard;
///
///   fn null() -> Self {
///     Self(AtomicPtr::new(ptr::null_mut()))
///   }
///
///   fn read<'guard>(&self, order: Ordering, _guard: &'guard Self::Guard) -> Self::Shared<'guard> {
///     Ptr { pointer: self.0.load(order), marker: PhantomData }
///   }
///
///   fn write(&self, order: Ordering, init: impl FnOnce(&mut MaybeUninit<T>))
///   where
///     T: 'static,
///   {
///     let mut uninit: Box<MaybeUninit<T>> = Box::new_uninit();
///
///     init(&mut uninit);
///
///     // SAFETY: `init` is required to fully initialize `uninit`.
///     self.0.store(Box::into_raw(unsafe { uninit.assume_init() }), order);
///   }
///
///   fn evict(&self, order: Ordering) -> bool {
///     !self.0.swap(ptr::null_mut(), order).is_null()
///   }
///
///   unsafe fn clear(&mut self) -> bool {
///     !self.0.swap(ptr::null_mut(), Ordering::Relaxed).is_null()
///   }
/// }
///
/// pub enum Coll {}
///
/// impl CollectorWeak for Coll {
///   type Guard = ();
///   type Atomic<T> = Narrow<T>;
///
///   fn guard() -> Self::Guard {}
///
///   fn flush() {}
/// }
///
/// pub enum Custom {}
///
/// impl Params for Custom {
///   type Collector = Coll;
///
///   const LENGTH: Capacity = Capacity::MIN;
/// }
///
/// let table: PTab<u32, Custom> = PTab::new();
/// assert!(table.is_empty());
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
/// Safe code cannot implement this trait, and cannot call its methods on the
/// raw `AtomicPtr` or `sdd::AtomicOwned` types; the collectors built into
/// `ptab` implement it on their own newtypes.
///
/// ```compile_fail,E0200
/// use ptab::memory::Atomic;
///
/// struct Custom;
///
/// impl<T> Atomic<T> for Custom {}
/// ```
///
/// ```compile_fail,E0277
/// use core::sync::atomic::AtomicPtr;
/// use core::sync::atomic::Ordering;
///
/// use ptab::memory::Atomic;
///
/// let ptr: AtomicPtr<u64> = AtomicPtr::new(0x10 as *mut u64);
/// Atomic::read(&ptr, Ordering::Relaxed, &());
/// ```
///
#[cfg_attr(
  feature = "sdd",
  doc = r#"
```compile_fail,E0277
use ptab::memory::Atomic;

let raw: sdd::AtomicOwned<u64> = sdd::AtomicOwned::null();
let guard: sdd::Guard = sdd::Guard::new();
Atomic::read(&raw, core::sync::atomic::Ordering::Relaxed, &guard);
```
"#
)]
///
/// Entries are not `Sync` unless the value is `Send + Sync`, for both built-in
/// collectors:
///
/// ```compile_fail,E0277
/// use core::cell::Cell;
///
/// use ptab::memory::CollectorWeak;
///
/// fn assert_sync<S: Sync>() {}
///
/// assert_sync::<<ptab::memory::collector::Leak as CollectorWeak>::Atomic<Cell<u8>>>();
/// ```
///
#[cfg_attr(
  feature = "sdd",
  doc = r#"
```compile_fail,E0277
use core::cell::Cell;

use ptab::memory::CollectorWeak;

fn assert_sync<S: Sync>() {}

assert_sync::<<ptab::memory::collector::Sdd as CollectorWeak>::Atomic<Cell<u8>>>();
```
"#
)]
///
/// # Safety
///
/// Implementing this trait asserts the following, which the table relies on:
///
/// - [`Atomic::read()`] returns a pointer that is null or refers to a live `T`
///   for the whole `'guard` lifetime.
/// - [`Atomic::write()`] stores a `T` the implementation owns. [`Atomic::evict()`]
///   hands it to the collector, which may leak it, and [`Atomic::clear()`]
///   drops it.
/// - [`Atomic::clear()`] leaves the slot null, so a second call drops nothing.
/// - The table derives `Send` and `Sync` from `T` alone, whatever the auto
///   traits of the implementing type are. When `T: Send`, a value of it may be
///   moved to and dropped on another thread. When `T: Send + Sync`, every
///   `&self` method may be called from several threads at once.
///
/// [`Atomic::clear()`]: Atomic::clear
/// [`Atomic::evict()`]: Atomic::evict
/// [`Atomic::read()`]: Atomic::read
/// [`Atomic::write()`]: Atomic::write
pub unsafe trait Atomic<T> {
  /// A guard that keeps the current thread pinned.
  type Guard;

  /// A pointer to a value the collector keeps alive while the guard it was
  /// read under is held.
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

  /// Drops the stored value, if any, at once instead of deferring it.
  ///
  /// Returns `true` if a value was dropped. The slot is left null, so a second
  /// call returns `false`.
  ///
  /// # Safety
  ///
  /// No reader may still hold a pointer read from this slot, because the value
  /// is dropped without waiting for guards.
  unsafe fn clear(&mut self) -> bool;
}

// -----------------------------------------------------------------------------
// Shared Ptr
// -----------------------------------------------------------------------------

/// A pointer to a value the collector keeps alive while the guard it was read
/// under is held.
///
/// The pointer is valid for use only during the lifetime `'guard`.
///
/// Like [`Atomic`], this trait must be implemented with `unsafe impl`. A safe
/// impl is rejected:
///
/// ```compile_fail,E0200
/// use ptab::memory::Shared;
///
/// struct Custom;
///
/// impl<'guard, T> Shared<'guard, T> for Custom {
///   fn is_null(&self) -> bool {
///     true
///   }
///
///   fn as_ref(&self) -> Option<&'guard T> {
///     None
///   }
/// }
/// ```
///
/// # Safety
///
/// This is an **unsafe trait**. Implementing it asserts that [`Shared::as_ref()`]
/// returns `None` for null, and otherwise a reference to a live `T` that
/// remains valid for `'guard`.
///
/// [`Shared::as_ref()`]: Shared::as_ref
pub unsafe trait Shared<'guard, T> {
  /// Returns `true` if the pointer is null.
  fn is_null(&self) -> bool;

  /// Returns a shared reference to the value.
  fn as_ref(&self) -> Option<&'guard T>;
}
