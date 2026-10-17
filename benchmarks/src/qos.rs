//! Quality-of-service class of benchmark threads.
//!
//! On macOS, `USER_INTERACTIVE` asks the scheduler for performance cores. Elsewhere every
//! function here is a no-op.

// https://developer.apple.com/library/archive/documentation/Performance/Conceptual/EnergyGuide-iOS/PrioritizeWorkWithQoS.html
#[cfg(target_os = "macos")]
mod imp {
  use libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE;

  /// Moves the calling thread to `USER_INTERACTIVE`. Returns whether the call succeeded.
  #[inline]
  pub fn set_realtime() -> bool {
    // SAFETY: the call only changes the QoS class of the calling thread and takes no pointers.
    unsafe { libc::pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) == 0 }
  }

  /// Name of the calling thread's QoS class, as reported by the kernel.
  pub fn current() -> &'static str {
    // The kernel writes a raw `u32`; reading it through `qos_class_t` would need every
    // value to be a valid discriminant, so the out-pointer is the raw integer instead.
    let mut class: u32 = 0;
    let mut priority: libc::c_int = 0;

    // SAFETY: `pthread_self()` is the calling thread, and both out-pointers refer to live
    // locals of the size the call writes (`qos_class_t` is `repr(u32)`).
    let status = unsafe {
      libc::pthread_get_qos_class_np(
        libc::pthread_self(),
        (&mut class as *mut u32).cast(),
        &mut priority,
      )
    };

    if status != 0 {
      return "unknown";
    }

    match class {
      0x21 => "user-interactive",
      0x19 => "user-initiated",
      0x15 => "default",
      0x11 => "utility",
      0x09 => "background",
      0x00 => "unspecified",
      _ => "other",
    }
  }
}

#[cfg(not(target_os = "macos"))]
mod imp {
  /// Always returns `false` off macOS.
  #[inline]
  pub fn set_realtime() -> bool {
    false
  }

  /// Always returns `"n/a"` off macOS.
  pub fn current() -> &'static str {
    "n/a"
  }
}

pub use self::imp::current;
pub use self::imp::set_realtime;

/// Pins the calling thread the first time it is called, and does nothing after that.
///
/// Thread pools that create their workers without a QoS attribute (divan's, for one) leave
/// them at `DEFAULT`, so each benchmark closure calls this on the thread that runs it.
pub fn pin_once() {
  thread_local! {
    static PINNED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
  }

  PINNED.with(|pinned| {
    if !pinned.replace(true) {
      set_realtime();
    }
  });
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
  use super::*;

  #[test]
  fn set_realtime_reaches_the_calling_thread() {
    std::thread::spawn(|| {
      assert!(set_realtime());
      assert_eq!(current(), "user-interactive");
    })
    .join()
    .unwrap();
  }
}
