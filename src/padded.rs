//! Cache-line padding to prevent false sharing.

use core::fmt::Debug;
use core::fmt::Display;
use core::fmt::Formatter;
use core::fmt::Result;
use core::ops::Deref;
use core::ops::DerefMut;

/// Pads and aligns a value to the length of a cache line.
///
/// Taken from [`crossbeam-utils`]
///
/// [`crossbeam-utils`]: https://crates.io/crates/crossbeam-utils
#[cfg_attr(
  any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "powerpc64",
  ),
  repr(align(128))
)]
#[cfg_attr(
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
)]
#[cfg_attr(target_arch = "m68k", repr(align(16)))]
#[cfg_attr(target_arch = "s390x", repr(align(256)))]
#[cfg_attr(
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
)]
pub struct CachePadded<T> {
  value: T,
}

unsafe impl<T: Send> Send for CachePadded<T> {}
unsafe impl<T: Sync> Sync for CachePadded<T> {}

impl<T> CachePadded<T> {
  #[inline]
  pub const fn new(value: T) -> Self {
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

impl<T> Debug for CachePadded<T>
where
  T: Debug,
{
  fn fmt(&self, f: &mut Formatter<'_>) -> Result {
    Debug::fmt(&self.value, f)
  }
}

impl<T> Display for CachePadded<T>
where
  T: Display,
{
  fn fmt(&self, f: &mut Formatter<'_>) -> Result {
    Display::fmt(&self.value, f)
  }
}
