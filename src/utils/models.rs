#[cfg(all(loom, shuttle))]
compile_error!("`--cfg loom` and `--cfg shuttle` are mutually exclusive");

#[cfg(loom)]
pub(crate) mod alloc {
  pub(crate) use crate::utils::std::Box;
  pub(crate) use crate::utils::std::handle_alloc_error;
  pub(crate) use ::loom::alloc::Layout;
  pub(crate) use ::loom::alloc::alloc;
  pub(crate) use ::loom::alloc::dealloc;
}

#[cfg(not(loom))]
pub(crate) mod alloc {
  pub(crate) use crate::utils::std::Box;
  pub(crate) use crate::utils::std::Layout;
  pub(crate) use crate::utils::std::alloc;
  pub(crate) use crate::utils::std::dealloc;
  pub(crate) use crate::utils::std::handle_alloc_error;
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
