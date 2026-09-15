//! Default compiler compositions. Explicit implementation entries never call this module.
//! Choices here preserve the established convenience routes; callers can select
//! another checked edge directly or bind its macro through a Rust module alias.
use crate::{cq_to_relational as logical, physical, relational_to_indexes as indexes};
use cq_ir::{cq, index_requirements as ix, relational_plan as ra};
use logical::{JoinPlanReport, JoinPolicy, PlanningOptions, dependencies::ClauseDependencies};

pub fn compile_cq(source: &cq::Module) -> syn::Result<ra::Module> {
    if source.program.query.is_basic() {
        logical::canonical::compile_cq_canonical(source)
    } else {
        logical::prefix::compile_cq_prefix(source)
    }
}

fn needs_result_indexes(source: &ra::Module) -> bool {
    !source.plan.output.head.is_normalized()
        || source.plan.definitions.iter().any(|d| {
            matches!(
                d.operator,
                ra::Operator::Select(_) | ra::Operator::Extend(_) | ra::Operator::Unit(_)
            )
        })
}

pub fn compile_relational_plan(source: &ra::Module) -> syn::Result<ix::Module> {
    if needs_result_indexes(source) {
        ra::contract::check(source)?;
        if !crate::analysis::linear::is_linear_prefix(source) {
            return Err(syn::Error::new_spanned(
                source,
                "the lazy backend requires a linear relational prefix",
            ));
        }
        indexes::result::compile_result_indexes(source)
    } else {
        indexes::input::compile_input_indexes(source)
    }
}

pub fn compile_index_requirements(source: &ix::Module) -> syn::Result<syn::File> {
    if needs_result_indexes(&source.relational_plan) {
        physical::lazy::compile_lazy_indexed(source)
    } else {
        physical::input_indexed::compile_input_indexed(source)
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
        logical::graph::plan(source, options)?
    } else {
        logical::clause_rules::plan(source, &facts, options)?
    };
    check_cq_plan(source, &target, &report)?;
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
    if report.derivation.last() == Some(&logical::Rule::Finish) {
        logical::check::prefix::check(source, target, report)
    } else {
        logical::check::graph::check(source, target, report)
    }
}

pub fn compile_cq_derivation(
    source: &cq::Module,
    report: &JoinPlanReport,
) -> syn::Result<ra::Module> {
    cq::contract::check(source)?;
    let facts = ClauseDependencies::analyze(source)?;
    let target = if report.derivation.last() == Some(&logical::Rule::Finish) {
        logical::clause_rules::replay(source, &facts, report)?
    } else {
        logical::graph::from_derivation(source, report)?
    };
    check_cq_plan(source, &target, report)?;
    Ok(target)
}

/// The default complete composition, also used by the workload catalog.
pub fn compile_program(source: &cq::Module) -> syn::Result<syn::File> {
    let plan = compile_cq(source)?;
    let indexes = compile_relational_plan(&plan)?;
    compile_index_requirements(&indexes)
}
