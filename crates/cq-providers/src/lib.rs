//! Independently selected row representation protocols.
//! Unsupported operations fail at expansion; Rust checks the resulting types.

#[cfg(feature = "block")]
pub mod block;
#[cfg(feature = "hash")]
pub mod hash;
#[cfg(feature = "sorted")]
pub mod sorted;

#[cfg(any(feature = "hash", feature = "sorted"))]
mod native;

#[cfg(all(test, any(feature = "hash", feature = "sorted")))]
mod native_tests;
