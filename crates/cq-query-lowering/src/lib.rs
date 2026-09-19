//! Independently selected CQ-to-RA implementations and their proof vocabulary.
//! Each implementation owns its admitted domain, scheduling and correspondence.

#[cfg(feature = "clause-prefix")]
pub mod clause_prefix;
#[cfg(feature = "hypergraph")]
pub mod hypergraph;
#[cfg(feature = "written-order")]
pub mod written_order;

#[cfg(all(
    feature = "clause-prefix",
    any(feature = "written-order", feature = "hypergraph")
))]
pub mod composition;

#[cfg(any(feature = "clause-prefix", feature = "hypergraph"))]
pub mod dependencies;
#[cfg(any(feature = "clause-prefix", feature = "hypergraph"))]
mod model;
#[cfg(any(feature = "clause-prefix", feature = "hypergraph"))]
mod proof;

#[cfg(any(feature = "clause-prefix", feature = "hypergraph"))]
pub use model::{Heading, JoinPlanReport, JoinPolicy, PlanningOptions, Rule};
#[cfg(any(feature = "clause-prefix", feature = "hypergraph"))]
pub use proof::{GyoRule, JoinTree};
