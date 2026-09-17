//! Independently selectable realizations of the existing Pipeline language.
#[cfg(feature = "collect")]
pub mod collect;
#[cfg(feature = "exists")]
pub mod exists;
#[cfg(feature = "iterator")]
pub mod iterator;
#[cfg(any(feature = "iterator", feature = "collect", feature = "stream"))]
mod observations;
#[cfg(any(
    feature = "iterator",
    feature = "collect",
    feature = "stream",
    feature = "exists"
))]
mod operators;
#[cfg(feature = "stream")]
pub mod stream;
#[cfg(all(test, feature = "iterator", feature = "collect", feature = "stream"))]
mod tests;
