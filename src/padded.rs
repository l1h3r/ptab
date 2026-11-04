use core::ops::Deref;
use core::ops::DerefMut;

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(all(test, not(any(loom, shuttle))))]
mod tests;

// -----------------------------------------------------------------------------
// Compile-time Checks
// -----------------------------------------------------------------------------

const CACHE_LINE_OVERRIDE: Option<usize> = if cfg!(ptab_cache_line = "16") {
  Some(16)
} else if cfg!(ptab_cache_line = "32") {
  Some(32)
} else if cfg!(ptab_cache_line = "64") {
  Some(64)
} else if cfg!(ptab_cache_line = "128") {
  Some(128)
} else if cfg!(ptab_cache_line = "256") {
  Some(256)
} else {
  None
};

const _: () = if let Some(size) = CACHE_LINE_OVERRIDE {
  assert!(
    align_of::<CachePadded<u8>>() == size,
    "`CachePadded` must match the `ptab_cache_line` override",
  );
};

// -----------------------------------------------------------------------------
// Implementation
// -----------------------------------------------------------------------------

/// Pads and aligns a value to the length of a cache line.
///
/// Alignment varies by architecture; see [`CACHE_LINE`] for the effective size.
///
/// Based on [`crossbeam-utils`].
///
/// [`CACHE_LINE`]: crate::params::CACHE_LINE
/// [`crossbeam-utils`]: https://crates.io/crates/crossbeam-utils
#[cfg_attr(ptab_cache_line = "16", repr(align(16)))]
#[cfg_attr(ptab_cache_line = "32", repr(align(32)))]
#[cfg_attr(ptab_cache_line = "64", repr(align(64)))]
#[cfg_attr(ptab_cache_line = "128", repr(align(128)))]
#[cfg_attr(ptab_cache_line = "256", repr(align(256)))]
#[cfg_attr(
  not(any(
    ptab_cache_line = "16",
    ptab_cache_line = "32",
    ptab_cache_line = "64",
    ptab_cache_line = "128",
    ptab_cache_line = "256",
  )),
  cfg_attr(
    any(
      target_arch = "x86_64",
      target_arch = "aarch64",
      target_arch = "arm64ec",
      target_arch = "powerpc64",
    ),
    repr(align(128))
  ),
  cfg_attr(
    any(
      target_arch = "arm",
      target_arch = "mips",
      target_arch = "mips32r6",
      target_arch = "mips64",
      target_arch = "mips64r6",
      target_arch = "sparc",
      target_arch = "hexagon",
    ),
    repr(align(32))
  ),
  cfg_attr(target_arch = "m68k", repr(align(16))),
  cfg_attr(target_arch = "s390x", repr(align(256))),
  cfg_attr(
    not(any(
      target_arch = "x86_64",
      target_arch = "aarch64",
      target_arch = "arm64ec",
      target_arch = "powerpc64",
      target_arch = "arm",
      target_arch = "mips",
      target_arch = "mips32r6",
      target_arch = "mips64",
      target_arch = "mips64r6",
      target_arch = "sparc",
      target_arch = "hexagon",
      target_arch = "m68k",
      target_arch = "s390x",
    )),
    repr(align(64))
  )
)]
pub(crate) struct CachePadded<T> {
  value: T,
}

// SAFETY: Transparent wrapper; inherits `Send` from `T`.
unsafe impl<T: Send> Send for CachePadded<T> {}

// SAFETY: Transparent wrapper; inherits `Sync` from `T`.
unsafe impl<T: Sync> Sync for CachePadded<T> {}

impl<T> CachePadded<T> {
  #[inline]
  pub(crate) const fn new(value: T) -> Self {
    Self { value }
  }
}

impl<T> Deref for CachePadded<T> {
  type Target = T;

  #[inline]
  fn deref(&self) -> &T {
    &self.value
  }
}

impl<T> DerefMut for CachePadded<T> {
  #[inline]
  fn deref_mut(&mut self) -> &mut T {
    &mut self.value
  }
}
