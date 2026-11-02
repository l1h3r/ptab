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
