//! Canonical written-order CQ -> RA, independently buildable and checked.
mod check;
mod lower;

pub use check::check;
pub use lower::compile;
