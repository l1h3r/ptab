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
    use ::core::cell::RefCell;

    ::std::thread_local! {
      /// The boxes behind every `AtomicUsize` this thread made since its last
      /// [`free_atomics`], as addresses.
      static BOXES: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }

    /// A pointer-sized handle to shuttle's wider atomic. `Array::drop` never
    /// drops its elements, and [`free_atomics`] frees the box.
    #[repr(transparent)]
    pub(crate) struct AtomicUsize {
      inner: *mut ::shuttle::sync::atomic::AtomicUsize,
    }

    // SAFETY: the pointee, a shuttle atomic, implements `Send` and `Sync` and
    // stays allocated until `free_atomics`.
    unsafe impl Send for AtomicUsize {}
    // SAFETY: as for `Send`.
    unsafe impl Sync for AtomicUsize {}

    impl AtomicUsize {
      #[inline]
      pub(crate) fn new(value: usize) -> Self {
        let inner: *mut ::shuttle::sync::atomic::AtomicUsize =
          Box::into_raw(Box::new(::shuttle::sync::atomic::AtomicUsize::new(value)));

        BOXES.with_borrow_mut(|boxes: &mut Vec<usize>| boxes.push(inner as usize));

        Self { inner }
      }
    }

    impl ::core::ops::Deref for AtomicUsize {
      type Target = ::shuttle::sync::atomic::AtomicUsize;

      #[inline]
      fn deref(&self) -> &Self::Target {
        // SAFETY: `inner` came from `Box::into_raw` in `new`. Only `free_atomics`
        // frees it, and its caller promises that no handle remains in use.
        unsafe { &*self.inner }
      }
    }

    /// Frees the box behind every `AtomicUsize` the calling thread made so far.
    ///
    /// # Safety
    ///
    /// After the call, no handle made before it may run a method, and no table
    /// that contains one may drop.
    pub(crate) unsafe fn free_atomics() {
      let boxes: Vec<usize> = BOXES.with_borrow_mut(::core::mem::take);

      for address in boxes {
        // SAFETY: `address` came from `Box::into_raw` in `new` and appears once
        // in the list, so this frees it once.
        drop(unsafe { Box::from_raw(address as *mut ::shuttle::sync::atomic::AtomicUsize) });
      }
    }

    pub(crate) use ::shuttle::sync::atomic::AtomicU32;
    pub(crate) use ::shuttle::sync::atomic::Ordering;
  }
}
