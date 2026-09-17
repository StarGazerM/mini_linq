//! Physical implementations of indexed and configured relational lowering.
#[cfg(feature = "borrowed-scans")]
pub mod borrowed_scans;
#[cfg(any(
    feature = "hash-inputs",
    feature = "hash-prefix",
    feature = "hash-materialized"
))]
mod check_support;
#[cfg(feature = "hash-inputs")]
pub mod hash_inputs;
#[cfg(feature = "hash-materialized")]
pub mod hash_materialized;
#[cfg(feature = "hash-prefix")]
pub mod hash_prefix;
#[cfg(feature = "relations")]
pub mod relations;
#[cfg(any(
    feature = "hash-prefix",
    feature = "hash-materialized",
    feature = "relations",
    feature = "borrowed-scans"
))]
mod rust_bindings;
