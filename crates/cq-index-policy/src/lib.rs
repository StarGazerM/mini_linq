//! Replaceable equality-demand policies over the existing relational language.
//! Policies emit IndexRequirements and do not choose their physical continuation.

#[cfg(feature = "base-demands")]
pub mod base_demands;
#[cfg(feature = "result-demands")]
pub mod result_demands;

#[cfg(feature = "no-demands")]
pub mod no_demands;
