//! CQ-only choices between the supplied logical planners.
//!
//! These helpers produce or check RA. They neither analyze index requirements
//! nor select providers, execution strategies, or physical lowering routes.

use cq_ir::{cq, relational_plan as ra};

#[cfg(feature = "hypergraph")]
use crate::{JoinPlanReport, JoinPolicy, PlanningOptions, Rule, dependencies::ClauseDependencies};

/// Choose written-order lowering for a basic CQ and clause-prefix otherwise.
#[cfg(feature = "written-order")]
pub fn compile_cq(source: &cq::Module) -> syn::Result<ra::Module> {
    if source.program.query.is_basic() {
        crate::written_order::compile(source)
    } else {
        crate::clause_prefix::compile(source)
    }
}

/// Plan through the automatic logical strategy.
#[cfg(feature = "hypergraph")]
pub fn compile_cq_planned(source: &cq::Module) -> syn::Result<ra::Module> {
    compile_cq_with_policy(source, JoinPolicy::Auto)
}

/// Apply a logical scheduling policy within the admitted planner domain.
#[cfg(feature = "hypergraph")]
pub fn compile_cq_with_policy(source: &cq::Module, policy: JoinPolicy) -> syn::Result<ra::Module> {
    explain_cq_plan(
        source,
        PlanningOptions {
            strategy: policy,
            max_arity: None,
        },
    )
    .map(|(target, _)| target)
}

/// Select graph or expression-preserving prefix planning and expose its proof.
#[cfg(feature = "hypergraph")]
pub fn explain_cq_plan(
    source: &cq::Module,
    options: PlanningOptions,
) -> syn::Result<(ra::Module, JoinPlanReport)> {
    cq::contract::check(source)?;
    let dependencies = ClauseDependencies::analyze(source)?;
    if dependencies.supports_graph_rules {
        crate::hypergraph::explain(source, options)
    } else {
        crate::clause_prefix::explain(source, options)
    }
}

/// Check the derivation grammar supplied by the caller, without replanning.
#[cfg(feature = "hypergraph")]
pub fn check_cq_plan(
    source: &cq::Module,
    target: &ra::Module,
    report: &JoinPlanReport,
) -> syn::Result<()> {
    cq::contract::check(source)?;
    // A graph-eligible source can also have a valid clause-prefix derivation.
    if report.derivation.last() == Some(&Rule::Finish) {
        crate::clause_prefix::check(source, target, report)
    } else {
        crate::hypergraph::check(source, target, report)
    }
}

/// Replay the caller's derivation and validate its report.
#[cfg(feature = "hypergraph")]
pub fn compile_cq_derivation(
    source: &cq::Module,
    report: &JoinPlanReport,
) -> syn::Result<ra::Module> {
    cq::contract::check(source)?;
    if report.derivation.last() == Some(&Rule::Finish) {
        crate::clause_prefix::replay(source, report)
    } else {
        crate::hypergraph::replay(source, report)
    }
}
