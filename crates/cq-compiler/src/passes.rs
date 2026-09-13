//! Checked compilation entry orchestration and the basic homework wrappers.
//! Concrete physical and execution strategies live in their own modules.

use cq_ir::{cq, index_requirements, relational_plan};

use crate::pass_contracts;

/// Alternative CQ -> RelationalPlan edge, with automatic graph planning.
pub fn compile_cq_planned(source: &cq::Module) -> syn::Result<relational_plan::Module> {
    compile_cq_with_policy(source, crate::JoinPolicy::Auto)
}

/// Select a planning policy without changing either language boundary.
pub fn compile_cq_with_policy(
    source: &cq::Module,
    policy: crate::JoinPolicy,
) -> syn::Result<relational_plan::Module> {
    explain_cq_plan(
        source,
        crate::PlanningOptions {
            strategy: policy,
            max_arity: None,
        },
    )
    .map(|(plan, _)| plan)
}

/// Diagnostic entry: the target IR and its derivation report are separate.
pub fn explain_cq_plan(
    source: &cq::Module,
    options: crate::PlanningOptions,
) -> syn::Result<(relational_plan::Module, crate::JoinPlanReport)> {
    cq::contract::check(source)?;
    let (target, report) = crate::cq_to_relational::plan(source, options)?;
    check_cq_plan(source, &target, &report)?;
    Ok((target, report))
}

/// Explicit source-to-target relation; also used by the planned pass gate.
pub fn check_cq_plan(
    source: &cq::Module,
    target: &relational_plan::Module,
    report: &crate::JoinPlanReport,
) -> syn::Result<()> {
    pass_contracts::planned_join::check(source, target, report)
}

/// Apply an independently supplied schedule through the same rule semantics.
pub fn compile_cq_derivation(
    source: &cq::Module,
    report: &crate::JoinPlanReport,
) -> syn::Result<relational_plan::Module> {
    cq::contract::check(source)?;
    let target = crate::cq_to_relational::from_derivation(source, report)?;
    check_cq_plan(source, &target, report)?;
    Ok(target)
}

/// RelationalPlan -> IndexRequirements on exact named operand versions.
pub fn compile_result_indexes(
    source: &relational_plan::Module,
) -> syn::Result<index_requirements::Module> {
    relational_plan::contract::check(source)?;
    let target = crate::relational_to_indexes::compile(source)?;
    if !pass_contracts::result_indexes_are_exact(source, &target) {
        return Err(syn::Error::new_spanned(
            source,
            "result-index pass violated its source-target contract",
        ));
    }
    Ok(target)
}

/// CP-Defer: preserve the complete RA and request no named indexed resources.
///
/// This alternative index-selection policy leaves auxiliary construction to the
/// selected physical implementation. It does not remove written requirements
/// from an existing IndexRequirements module.
pub fn compile_deferred_indexes(
    source: &relational_plan::Module,
) -> syn::Result<index_requirements::Module> {
    relational_plan::contract::check(source)?;
    let target = index_requirements::Module {
        relational_plan: source.clone(),
        indexes_token: Default::default(),
        indexes_brace: Default::default(),
        indexes: Default::default(),
    };
    pass_contracts::composed::deferred_indexes(source, &target)?;
    Ok(target)
}

/// CP-Realize: realize an empty index overlay through selected physical macros.
///
/// Every written index requirement is rejected. Each selected implementation
/// owns construction or borrowing of the auxiliary structures its algorithm
/// needs; canonical indexed backends retain their separate coverage contracts.
pub fn compile_composed(
    source: &index_requirements::Module,
    options: &crate::physical::composed::PhysicalOptions,
) -> syn::Result<syn::File> {
    pass_contracts::composed::domain(source)?;
    let target = crate::physical::composed::compile(&source.relational_plan, options)?;
    pass_contracts::composed::realization(source, &target, options)?;
    Ok(target)
}

/// Indexed set DAG -> staged Rust, with local set-valued Access holes.
pub fn compile_materialized(source: &index_requirements::Module) -> syn::Result<syn::File> {
    compile_materialized_with(source, &syn::parse_quote!(::mini_linq::collect_access))
}

/// The same checked hash realization with a caller-selected Access continuation.
/// The continuation must return the exact `HashSet` denoted by each Access plan;
/// it owns distinctness and collection. Its path changes neither concrete
/// storage checks nor local relational access checks.
pub fn compile_materialized_with(
    source: &index_requirements::Module,
    access: &syn::Path,
) -> syn::Result<syn::File> {
    index_requirements::contract::check(source)?;
    crate::relational_to_indexes::check_coverage(source)?;
    let target = crate::physical::materialized::compile(source, access)?;
    pass_contracts::physical::materialized_with(source, &target, access)?;
    Ok(target)
}

#[path = "staff_r1.rs"]
mod staff_r1;

/// Basic Homework 1: lower a CQ to canonical named relational algebra.
#[allow(unreachable_code, clippy::diverging_sub_expression)]
#[contracts::contract(
    debug_ensures(
        cq::contract::well_formed(source) -> ret.as_ref()
            .is_ok_and(|target| pass_contracts::relational_plan_is_exact(source, target))
    ),
)]
pub fn compile_cq(source: &cq::Module) -> syn::Result<relational_plan::Module> {
    cq::contract::check(source)?;
    // BASIC HOMEWORK 1: construct the complete canonical RelationalPlan.
    // OPTIONAL HW4 — STUDENT 3/6 (LOGICAL): extend this lowering as specified
    // in doc/HW4-OPTIONAL.md.
    staff_r1::compile_cq(source)
}

/// Basic Homework 2: add the exact equality-index overlay to a logical plan.
#[allow(unreachable_code, clippy::diverging_sub_expression)]
#[contracts::contract(
    debug_ensures(
        pass_contracts::basic_left_deep_plan(source)
            -> ret.as_ref().is_ok_and(index_requirements::contract::well_formed)
    ),
    debug_ensures(
        pass_contracts::basic_left_deep_plan(source)
            -> ret.as_ref().is_ok_and(|target| pass_contracts::indexes_are_exact(source, target))
    ),
    debug_ensures(
        !pass_contracts::basic_left_deep_plan(source) -> ret.is_err()
    ),
)]
pub fn compile_relational_plan(
    source: &relational_plan::Module,
) -> syn::Result<index_requirements::Module> {
    relational_plan::contract::check(source)?;
    // BASIC HOMEWORK 2: preserve the full logical plan and derive its keys.
    // OPTIONAL HW4 — STUDENT 4/6 (INDEX): extend canonical key derivation as
    // specified in doc/HW4-OPTIONAL.md.
    staff_r1::compile_relational_plan(source)
}

/// Basic Homework 3: lower the required indexed join to staged, ordinary Rust.
///
/// The returned file must contain the complete query API and concrete Rust
/// storage-building statements. Its core join is one
/// `::mini_linq::pull!` call whose body and head occurrences are paired with
/// their exact Rust patterns, sources, predicates, and output expression.
/// Every written index requirement, including an unused extra one, is built as
/// a concrete Rust collection. During its one source walk, this pass returns a
/// span-aware error if a canonical access requirement is missing.
#[allow(unreachable_code, clippy::diverging_sub_expression)]
#[contracts::contract(
    debug_ensures(
        index_requirements::contract::well_formed(source)
            && pass_contracts::physical_input_is_supported(source) -> ret.is_ok()
    ),
    debug_ensures(!pass_contracts::physical_input_is_supported(source) -> ret.is_err()),
)]
pub fn compile_index_requirements(source: &index_requirements::Module) -> syn::Result<syn::File> {
    index_requirements::contract::check(source)?;
    // BASIC HOMEWORK 3: lower the indexed logical plan to staged Rust.
    // OPTIONAL HW4 — STUDENT 6/6 (PHYSICAL): emit the extended accesses as
    // specified in doc/HW4-OPTIONAL.md.
    let target = staff_r1::compile_index_requirements(source)?;
    if crate::relational_to_indexes::uses_result_index_route(&source.relational_plan) {
        pass_contracts::physical::lazy(source, &target)?;
    } else {
        pass_contracts::physical::basic(source, &target)?;
    }
    Ok(target)
}
