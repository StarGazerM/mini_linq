//! Independently selected row representation protocols.
//! Unsupported operations fail at expansion; Rust checks the resulting types.

#[cfg(feature = "rows-block")]
pub mod block;
#[cfg(feature = "rows-hash")]
pub mod hash;
#[cfg(feature = "rows-sorted")]
pub mod sorted;
#[cfg(feature = "rows-tuple-hash")]
pub mod tuple_hash;
#[cfg(feature = "rows-tuple-sorted")]
pub mod tuple_sorted;

#[cfg(any(
    feature = "rows-hash",
    feature = "rows-sorted",
    feature = "rows-tuple-hash",
    feature = "rows-tuple-sorted"
))]
mod native;
#[cfg(any(
    feature = "rows-hash",
    feature = "rows-sorted",
    feature = "rows-block",
    feature = "rows-tuple-hash",
    feature = "rows-tuple-sorted"
))]
mod schema;

#[cfg(all(test, any(feature = "rows-hash", feature = "rows-sorted")))]
mod native_tests;
