//! Physical implementations of indexed and configured relational lowering.
#[cfg(any(feature = "input", feature = "lazy", feature = "materialized"))]
mod check_support;
#[cfg(feature = "direct")]
pub mod direct;
#[cfg(feature = "input")]
pub mod input;
#[cfg(feature = "lazy")]
pub mod lazy;
#[cfg(feature = "materialized")]
pub mod materialized;
#[cfg(any(feature = "lazy", feature = "materialized"))]
mod rust_bindings;
