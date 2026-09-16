//! Physical implementations of indexed and configured relational lowering.
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
#[cfg(feature = "nested-pull")]
pub mod nested_pull;
#[cfg(feature = "nested-push")]
pub mod nested_push;
#[cfg(any(feature = "nested-push", feature = "nested-pull"))]
mod nested_support;
#[cfg(feature = "relations")]
pub mod relations;
#[cfg(any(
    feature = "hash-prefix",
    feature = "hash-materialized",
    feature = "relations",
    feature = "nested-push",
    feature = "nested-pull"
))]
mod rust_bindings;
