//! Default compiler compositions. Explicit implementation entries never call this module.
//! Choices here preserve the established convenience routes; callers can select
//! another checked edge directly or bind its macro through a Rust module alias.
use cq_ir::{cq, relational_plan as ra};
mod indexed;
use cq_plan::{
    JoinPlanReport, JoinPolicy, PlanningOptions, Rule, dependencies::ClauseDependencies,
};
pub use indexed::{compile_indexed, select_indexes, use_input_indexes};

pub fn compile_cq(source: &cq::Module) -> syn::Result<ra::Module> {
    if source.program.query.is_basic() {
        cq_plan::canonical::compile(source)
    } else {
        cq_plan::prefix::compile(source)
    }
}

pub fn compile_cq_planned(source: &cq::Module) -> syn::Result<ra::Module> {
    compile_cq_with_policy(source, JoinPolicy::Auto)
}

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

pub fn explain_cq_plan(
    source: &cq::Module,
    options: PlanningOptions,
) -> syn::Result<(ra::Module, JoinPlanReport)> {
    cq::contract::check(source)?;
    let facts = ClauseDependencies::analyze(source)?;
    let (target, report) = if facts.supports_graph_rules {
        cq_plan::graph::explain(source, options)?
    } else {
        cq_plan::prefix::explain(source, options)?
    };
    Ok((target, report))
}

pub fn check_cq_plan(
    source: &cq::Module,
    target: &ra::Module,
    report: &JoinPlanReport,
) -> syn::Result<()> {
    cq::contract::check(source)?;
    // The existing derivation grammar identifies the proof being checked.
    // A graph-eligible source can also have a valid clause-prefix derivation.
    if report.derivation.last() == Some(&Rule::Finish) {
        cq_plan::prefix::check(source, target, report)
    } else {
        cq_plan::graph::check(source, target, report)
    }
}

pub fn compile_cq_derivation(
    source: &cq::Module,
    report: &JoinPlanReport,
) -> syn::Result<ra::Module> {
    cq::contract::check(source)?;
    let target = if report.derivation.last() == Some(&Rule::Finish) {
        cq_plan::prefix::replay(source, report)?
    } else {
        cq_plan::graph::replay(source, report)?
    };
    Ok(target)
}

/// The fixed typed composition. Public macros bind their own continuations.
pub fn compile_program(
    source: &cq::Module,
    iterator_access: &syn::Path,
    set_access: &syn::Path,
) -> syn::Result<syn::File> {
    let plan = compile_cq(source)?;
    compile_indexed(&plan, iterator_access, set_access).map(|(_, program)| program)
}
