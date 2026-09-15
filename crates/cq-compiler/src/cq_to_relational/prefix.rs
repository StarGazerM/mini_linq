//! Expression-preserving CQ -> RA lowering with explicit clause scheduling.
use super::{
    JoinPlanReport, JoinPolicy, PlanningOptions, check, clause_rules,
    dependencies::ClauseDependencies,
};
use cq_ir::{cq, relational_plan as ra};

pub fn compile_cq_prefix(source: &cq::Module) -> syn::Result<ra::Module> {
    cq::contract::check(source)?;
    let facts = ClauseDependencies::analyze(source)?;
    let (target, _) = clause_rules::plan(
        source,
        &facts,
        PlanningOptions {
            strategy: JoinPolicy::Written,
            max_arity: None,
        },
    )?;
    check::ordered::check(source, &target)?;
    Ok(target)
}

pub fn explain_cq_prefix(
    source: &cq::Module,
    options: PlanningOptions,
) -> syn::Result<(ra::Module, JoinPlanReport)> {
    cq::contract::check(source)?;
    let facts = ClauseDependencies::analyze(source)?;
    let (target, report) = clause_rules::plan(source, &facts, options)?;
    check::prefix::check(source, &target, &report)?;
    Ok((target, report))
}
