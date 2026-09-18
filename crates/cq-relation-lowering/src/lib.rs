//! Physical implementations of indexed and configured relational lowering.
#[cfg(feature = "base-hash")]
pub mod base_hash;
#[cfg(feature = "borrowed-nested")]
pub mod borrowed_nested;
#[cfg(any(
    feature = "base-hash",
    feature = "prefix-hash",
    feature = "materialized-hash"
))]
mod check_support;
#[cfg(feature = "configured")]
pub mod configured;
#[cfg(feature = "materialized-hash")]
pub mod materialized_hash;
#[cfg(feature = "prefix-hash")]
pub mod prefix_hash;
#[cfg(any(
    feature = "prefix-hash",
    feature = "materialized-hash",
    feature = "configured",
    feature = "borrowed-nested"
))]
mod rust_bindings;
