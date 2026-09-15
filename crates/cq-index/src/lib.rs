//! Replaceable equality-demand policies over the existing relational language.
//! Policies emit IndexRequirements and do not choose their physical continuation.

#[cfg(feature = "input")]
pub mod input;
#[cfg(feature = "result")]
pub mod result;
