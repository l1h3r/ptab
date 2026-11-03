#![allow(unused_imports, reason = "conditional")]
#![allow(unused_macros, reason = "conditional")]

mod models;

#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(any(test, kani))]
mod tests;

pub(crate) mod std;

pub(crate) use self::models::alloc;
pub(crate) use self::models::sync;

#[cfg(any(test, kani))]
pub(crate) use self::tests::*;
