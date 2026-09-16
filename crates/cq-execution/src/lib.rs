//! Independently selected execution models for a checked Access region.

#[cfg(feature = "pull")]
pub mod pull;
#[cfg(feature = "push")]
pub mod push;
