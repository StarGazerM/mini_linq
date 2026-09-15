//! A key resource owns construction and every operation interpreting it.
#[cfg(feature = "keys-exact")]
pub mod exact;
#[cfg(feature = "keys-zone")]
pub mod zone;
