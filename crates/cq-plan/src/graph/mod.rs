//! Graph CQ -> RA planning with independent correspondence checks and rule replay.
mod check;
mod hypergraph;
mod plan;
mod rules;
mod strategy;
mod ya;

use crate::graph::hypergraph::{check_analysis, incidence};
use crate::{Heading, JoinPlanReport, JoinPolicy, JoinTree, PlanningOptions, Rule};
pub use check::check;
pub use plan::{compile, explain, replay};

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}
