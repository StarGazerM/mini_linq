//! Selected CQ-to-RA graph implementation and its independent checker.
mod check {
    pub(crate) mod graph;
}
// The shared analysis also derives facts consumed by the omitted clause route.
#[allow(dead_code)]
mod dependencies;
// The selected entry does not expose manual derivation replay.
#[allow(dead_code)]
pub(crate) mod graph;
mod hypergraph;
mod model;
mod rules;
mod strategy;
mod ya;

pub use hypergraph::{GyoRule, JoinTree};
pub(crate) use hypergraph::{check_analysis, incidence};
pub(crate) use model::{Heading, error};
pub use model::{JoinPlanReport, JoinPolicy, PlanningOptions, Rule};
