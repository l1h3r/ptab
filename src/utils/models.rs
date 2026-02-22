#[cfg(all(loom, shuttle))]
compile_error!("expected one of `--cfg loom` or `--cfg shuttle`.");

#[cfg(all(feature = "alloc", not(feature = "std")))]
mod rust_alloc {
  pub(crate) use ::rust_alloc::alloc::Layout;
  pub(crate) use ::rust_alloc::alloc::alloc;
  pub(crate) use ::rust_alloc::alloc::dealloc;
  pub(crate) use ::rust_alloc::alloc::handle_alloc_error;
  pub(crate) use ::rust_alloc::boxed::Box;
}

#[cfg(feature = "std")]
mod rust_alloc {
  pub(crate) use ::std::alloc::Layout;
  pub(crate) use ::std::alloc::alloc;
  pub(crate) use ::std::alloc::dealloc;
  pub(crate) use ::std::alloc::handle_alloc_error;
  pub(crate) use ::std::boxed::Box;
}

#[cfg(loom)]
pub(crate) mod alloc {
  pub(crate) use super::rust_alloc::Box;
  pub(crate) use super::rust_alloc::handle_alloc_error;
  pub(crate) use ::loom::alloc::Layout;
  pub(crate) use ::loom::alloc::alloc;
  pub(crate) use ::loom::alloc::dealloc;
}

#[cfg(not(loom))]
pub(crate) mod alloc {
  pub(crate) use super::rust_alloc::Box;
  pub(crate) use super::rust_alloc::Layout;
  pub(crate) use super::rust_alloc::alloc;
  pub(crate) use super::rust_alloc::dealloc;
  pub(crate) use super::rust_alloc::handle_alloc_error;
}

#[cfg(not(any(loom, shuttle)))]
pub(crate) mod sync {
  pub(crate) mod atomic {
    pub(crate) use ::core::sync::atomic::AtomicU32;
    pub(crate) use ::core::sync::atomic::AtomicUsize;
    pub(crate) use ::core::sync::atomic::Ordering;
  }
}

#[cfg(loom)]
pub(crate) mod sync {
  pub(crate) mod atomic {
    pub(crate) use ::loom::sync::atomic::AtomicU32;
    pub(crate) use ::loom::sync::atomic::AtomicUsize;
    pub(crate) use ::loom::sync::atomic::Ordering;
  }
}

#[cfg(shuttle)]
pub(crate) mod sync {
  pub(crate) mod atomic {
    #[repr(transparent)]
    pub(crate) struct AtomicUsize {
      inner: Box<::shuttle::sync::atomic::AtomicUsize>,
    }

    impl AtomicUsize {
      #[inline]
      pub(crate) fn new(value: usize) -> Self {
        Self {
          inner: Box::new(::shuttle::sync::atomic::AtomicUsize::new(value)),
        }
      }
    }

    impl ::core::ops::Deref for AtomicUsize {
      type Target = ::shuttle::sync::atomic::AtomicUsize;

      #[inline]
      fn deref(&self) -> &Self::Target {
        &self.inner
      }
    }

    pub(crate) use ::shuttle::sync::atomic::AtomicU32;
    pub(crate) use ::shuttle::sync::atomic::Ordering;
  }
}
