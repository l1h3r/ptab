#![allow(unused_imports, reason = "conditional")]
#![allow(unused_macros, reason = "conditional")]

mod models;

#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg(test)]
mod tests;

pub(crate) mod std;

pub(crate) use self::models::alloc;
pub(crate) use self::models::sync;

#[cfg(test)]
pub(crate) use self::tests::*;
