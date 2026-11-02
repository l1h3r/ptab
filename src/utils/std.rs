#[cfg(all(feature = "alloc", not(feature = "std")))]
mod rust_alloc {
  pub(crate) use ::rust_alloc::alloc::Layout;
  pub(crate) use ::rust_alloc::alloc::alloc;
  pub(crate) use ::rust_alloc::alloc::dealloc;
  pub(crate) use ::rust_alloc::alloc::handle_alloc_error;
  pub(crate) use ::rust_alloc::boxed::Box;
  pub(crate) use ::rust_alloc::collections::BTreeSet;
  pub(crate) use ::rust_alloc::format;
  pub(crate) use ::rust_alloc::string::String;
}

#[cfg(feature = "std")]
mod rust_alloc {
  pub(crate) use ::std::alloc::Layout;
  pub(crate) use ::std::alloc::alloc;
  pub(crate) use ::std::alloc::dealloc;
  pub(crate) use ::std::alloc::handle_alloc_error;
  pub(crate) use ::std::boxed::Box;
  pub(crate) use ::std::collections::BTreeSet;
  pub(crate) use ::std::format;
  pub(crate) use ::std::string::String;
}

pub(crate) use self::rust_alloc::*;
