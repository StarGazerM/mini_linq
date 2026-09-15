//! Graph-based CQ -> RA planning with a checked admitted domain.
use super::hypergraph;
use super::{JoinPlanReport, JoinPolicy, PlanningOptions, Rule, check_analysis, error, incidence};
use super::{check, rules::JoinState, strategy};
use crate::dependencies::ClauseDependencies;
use cq_ir::{cq, relational_plan as ra};

/// Compile the graph fragment using automatic acyclic/cyclic planning.
pub fn compile(source: &cq::Module) -> syn::Result<ra::Module> {
    explain(
        source,
        PlanningOptions {
            strategy: JoinPolicy::Auto,
            max_arity: None,
        },
    )
    .map(|(target, _)| target)
}

pub fn explain(
    source: &cq::Module,
    options: PlanningOptions,
) -> syn::Result<(ra::Module, JoinPlanReport)> {
    cq::contract::check(source)?;
    let facts = ClauseDependencies::analyze(source)?;
    if !facts.supports_graph_rules {
        return Err(syn::Error::new_spanned(
            source,
            "graph planning requires normalized atoms, a variable head and movable comparisons",
        ));
    }
    let (target, report) = plan(source, options)?;
    check(source, &target, &report)?;
    Ok((target, report))
}

fn plan(
    source: &cq::Module,
    options: PlanningOptions,
) -> syn::Result<(ra::Module, JoinPlanReport)> {
    let edges = incidence(source);
    let (analysis, join_tree) = if options.strategy == JoinPolicy::Auto {
        hypergraph::analyze(&edges)
    } else {
        (Vec::new(), None)
    };
    if let Some(tree) = &join_tree {
        tree.check(&edges).map_err(error)?;
    }
    let mut state = JoinState::new(source, join_tree.clone(), options.max_arity);
    let mut derivation = Vec::new();
    for occurrence in 0..edges.len() {
        apply(&mut state, &mut derivation, Rule::Introduce { occurrence })?;
    }
    while let Some(rule) = state.ready_selection() {
        apply(&mut state, &mut derivation, rule)?;
    }
    apply(&mut state, &mut derivation, Rule::Begin)?;
    if let Some(tree) = &join_tree {
        super::ya::reduce(tree, &mut state, &mut derivation)?;
        apply(&mut state, &mut derivation, Rule::Freeze)?;
    }
    select_and_trim(&mut state, &mut derivation)?;
    while state.frontier().len() > 1 {
        let (left, right) = strategy::choose_pair(&state, options.strategy, join_tree.as_ref());
        apply(&mut state, &mut derivation, Rule::Combine { left, right })?;
        select_and_trim(&mut state, &mut derivation)?;
    }
    let group = *state.frontier().first_key_value().unwrap().0;
    apply(&mut state, &mut derivation, Rule::Output { group })?;
    let ra = state.into_module()?;
    let peak_arity = ra::check_in(source.program.inputs(), &ra.plan)?
        .values()
        .map(|r| r.heading.len())
        .max()
        .unwrap_or(0);
    let planned = JoinPlanReport {
        options,
        analysis,
        join_tree,
        derivation,
        peak_arity,
    };
    Ok((ra, planned))
}

pub(super) fn apply(
    state: &mut JoinState<'_>,
    trace: &mut Vec<Rule>,
    rule: Rule,
) -> syn::Result<()> {
    state.apply(&rule)?;
    trace.push(rule);
    Ok(())
}

fn select_and_trim(state: &mut JoinState<'_>, trace: &mut Vec<Rule>) -> syn::Result<()> {
    while let Some(rule) = state.ready_selection().or_else(|| state.next_trim()) {
        apply(state, trace, rule)?;
    }
    Ok(())
}

/// Entry for a separately implemented scheduling policy. The caller supplies
/// administrative commands; the checked kernel constructs the residual RA.
pub fn replay(source: &cq::Module, report: &JoinPlanReport) -> syn::Result<ra::Module> {
    cq::contract::check(source)?;
    let facts = ClauseDependencies::analyze(source)?;
    if !facts.supports_graph_rules {
        return Err(syn::Error::new_spanned(
            source,
            "graph replay requires normalized atoms, a variable head and movable comparisons",
        ));
    }
    check_analysis(
        &incidence(source),
        report.options,
        &report.analysis,
        &report.join_tree,
    )?;
    let mut state = JoinState::new(source, report.join_tree.clone(), report.options.max_arity);
    for rule in &report.derivation {
        state.apply(rule)?;
    }
    let target = state.into_module()?;
    check(source, &target, report)?;
    Ok(target)
}
