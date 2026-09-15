//! Independently selectable CQ -> RA implementations.
pub(crate) mod canonical;
pub(crate) mod check;
pub(crate) mod clause_rules;
pub(crate) mod dependencies;
pub(crate) mod graph;
mod hypergraph;
mod model;
mod names;
pub(crate) mod prefix;
mod rules;
mod strategy;
mod ya;

pub use hypergraph::{GyoRule, JoinTree};
pub(crate) use hypergraph::{check_analysis, incidence};
pub(crate) use model::{Heading, error};
pub use model::{JoinPlanReport, JoinPolicy, PlanningOptions, Rule};
pub(crate) use names::{fresh, reserved};
