//! Expression-preserving CQ -> RA lowering with explicit clause scheduling.
mod check;
mod names;
mod rules;
mod strategy;

use crate::{
    Heading, JoinPlanReport, JoinPolicy, PlanningOptions, Rule, dependencies::ClauseDependencies,
};
pub use check::check;
use cq_ir::{cq, relational_plan as ra};
use names::{fresh, reserved};

pub fn compile(source: &cq::Module) -> syn::Result<ra::Module> {
    cq::contract::check(source)?;
    let facts = ClauseDependencies::analyze(source)?;
    let (target, _) = rules::plan(
        source,
        &facts,
        PlanningOptions {
            strategy: JoinPolicy::Written,
            max_arity: None,
        },
    )?;
    check::clauses::check(source, &target)?;
    Ok(target)
}

pub fn explain(
    source: &cq::Module,
    options: PlanningOptions,
) -> syn::Result<(ra::Module, JoinPlanReport)> {
    cq::contract::check(source)?;
    let facts = ClauseDependencies::analyze(source)?;
    let (target, report) = rules::plan(source, &facts, options)?;
    check(source, &target, &report)?;
    Ok((target, report))
}

/// Replay caller-supplied clause rules, then independently check their emitted RA.
pub fn replay(source: &cq::Module, report: &JoinPlanReport) -> syn::Result<ra::Module> {
    cq::contract::check(source)?;
    let facts = ClauseDependencies::analyze(source)?;
    let target = rules::replay(source, &facts, report)?;
    check(source, &target, report)?;
    Ok(target)
}

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}
