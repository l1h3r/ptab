#[cfg(doctest)]
mod remove_send {
  /// `remove` rejects a `T` that lacks `Send`.
  ///
  /// ```compile_fail,E0277
  /// use std::sync::{Mutex, MutexGuard};
  /// use ptab::PTab;
  ///
  /// static M: Mutex<u8> = Mutex::new(0);
  ///
  /// let table: PTab<MutexGuard<'static, u8>> = PTab::new();
  /// let idx = table.insert(M.lock().unwrap()).unwrap();
  /// table.remove(idx);
  /// ```
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// let table: PTab<u8> = PTab::new();
  /// let idx = table.insert(0).unwrap();
  /// table.remove(idx);
  /// ```
  #[cfg(feature = "sdd")]
  const REMOVE_NEEDS_SEND: () = ();
}

#[cfg(doctest)]
mod unwind_safe {
  /// `RefCell<u32>` lacks `RefUnwindSafe`.
  ///
  /// ```compile_fail,E0277
  /// use std::cell::RefCell;
  /// use ptab::PTab;
  ///
  /// fn assert_ref_unwind_safe<T: std::panic::RefUnwindSafe>() {}
  ///
  /// assert_ref_unwind_safe::<PTab<RefCell<u32>>>();
  /// ```
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// fn assert_ref_unwind_safe<T: std::panic::RefUnwindSafe>() {}
  ///
  /// assert_ref_unwind_safe::<PTab<u32>>();
  /// ```
  const PTAB_REF_UNWIND_SAFE: () = ();

  /// `RefCell<u32>` lacks `RefUnwindSafe`.
  ///
  /// ```compile_fail,E0277
  /// use std::cell::RefCell;
  /// use ptab::PTab;
  ///
  /// fn assert_unwind_safe<T: std::panic::UnwindSafe>() {}
  ///
  /// assert_unwind_safe::<PTab<RefCell<u32>>>();
  /// ```
  ///
  /// ```
  /// use ptab::PTab;
  ///
  /// fn assert_unwind_safe<T: std::panic::UnwindSafe>() {}
  ///
  /// assert_unwind_safe::<PTab<u32>>();
  /// ```
  const PTAB_UNWIND_SAFE: () = ();

  /// `RefCell<u32>` lacks `RefUnwindSafe`.
  ///
  /// ```compile_fail,E0277
  /// use std::cell::RefCell;
  /// use ptab::{DefaultParams, WeakKeys};
  ///
  /// fn assert_ref_unwind_safe<T: std::panic::RefUnwindSafe>() {}
  ///
  /// assert_ref_unwind_safe::<WeakKeys<'static, RefCell<u32>, DefaultParams>>();
  /// ```
  ///
  /// ```
  /// use ptab::{DefaultParams, WeakKeys};
  ///
  /// fn assert_ref_unwind_safe<T: std::panic::RefUnwindSafe>() {}
  ///
  /// assert_ref_unwind_safe::<WeakKeys<'static, u32, DefaultParams>>();
  /// ```
  const WEAK_KEYS_REF_UNWIND_SAFE: () = ();

  /// `RefCell<u32>` lacks `RefUnwindSafe`.
  ///
  /// ```compile_fail,E0277
  /// use std::cell::RefCell;
  /// use ptab::{DefaultParams, WeakKeys};
  ///
  /// fn assert_unwind_safe<T: std::panic::UnwindSafe>() {}
  ///
  /// assert_unwind_safe::<WeakKeys<'static, RefCell<u32>, DefaultParams>>();
  /// ```
  ///
  /// ```
  /// use ptab::{DefaultParams, WeakKeys};
  ///
  /// fn assert_unwind_safe<T: std::panic::UnwindSafe>() {}
  ///
  /// assert_unwind_safe::<WeakKeys<'static, u32, DefaultParams>>();
  /// ```
  const WEAK_KEYS_UNWIND_SAFE: () = ();
}
