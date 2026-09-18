//! Independently selectable realizations of the existing Pipeline language.
#[cfg(feature = "collect-set")]
pub mod collect_set;
#[cfg(feature = "distinct-iter")]
pub mod distinct_iter;
#[cfg(feature = "exists")]
pub mod exists;
#[cfg(any(
    feature = "distinct-iter",
    feature = "collect-set",
    feature = "support-iter"
))]
mod observations;
#[cfg(any(
    feature = "distinct-iter",
    feature = "collect-set",
    feature = "support-iter",
    feature = "exists"
))]
mod operators;
#[cfg(feature = "support-iter")]
pub mod support_iter;
#[cfg(all(
    test,
    feature = "distinct-iter",
    feature = "collect-set",
    feature = "support-iter"
))]
mod tests;
