//! Independently selected CQ-to-RA implementations and their proof vocabulary.
//! Each implementation owns its admitted domain, scheduling and correspondence.

#[cfg(feature = "canonical")]
pub mod canonical;
#[cfg(feature = "graph")]
pub mod graph;
#[cfg(feature = "prefix")]
pub mod prefix;

#[cfg(any(feature = "prefix", feature = "graph"))]
pub mod dependencies;
#[cfg(any(feature = "prefix", feature = "graph"))]
mod model;
#[cfg(any(feature = "prefix", feature = "graph"))]
mod proof;

#[cfg(any(feature = "prefix", feature = "graph"))]
pub use model::{Heading, JoinPlanReport, JoinPolicy, PlanningOptions, Rule};
#[cfg(any(feature = "prefix", feature = "graph"))]
pub use proof::{GyoRule, JoinTree};
