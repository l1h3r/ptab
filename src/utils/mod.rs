#![allow(unused_imports, reason = "conditional")]
#![allow(unused_macros, reason = "conditional")]

#[cfg(test)]
mod macros;
mod models;

pub(crate) mod std;

#[cfg(test)]
pub(crate) use self::macros::each_capacity;
#[cfg(test)]
pub(crate) use self::macros::refute;
pub(crate) use self::models::alloc;
pub(crate) use self::models::sync;
