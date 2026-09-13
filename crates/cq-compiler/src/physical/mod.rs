//! Alternative physical realizations of relational programs.
//! Each module owns its admitted domain, representation and lowering.

pub(crate) mod composed;
pub(crate) mod joins;
pub(crate) mod lazy;
pub(crate) mod materialized;
pub(crate) mod options;
pub(crate) mod providers;
mod rust;
pub(crate) mod semijoin;
