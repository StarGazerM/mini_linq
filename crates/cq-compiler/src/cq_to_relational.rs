//! CQ -> RelationalPlan: dependency judgments, checked rules, and scheduling policy.
pub(crate) mod canonical;
pub(crate) mod clause_rules;
pub(crate) mod dependencies;
mod hypergraph;
mod rules;
mod strategy;

use cq_ir::{cq, relational_plan as ra, symbol_name};
use dependencies::ClauseDependencies;
use hypergraph::GyoState;
pub use hypergraph::{GyoRule, JoinTree};
use proc_macro2::{TokenStream, TokenTree};
use rules::JoinState;
pub use rules::Rule;
use std::collections::BTreeSet;

type Heading = BTreeSet<String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinPolicy {
    Written,
    Connected,
    GreedyWidth,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanningOptions {
    pub strategy: JoinPolicy,
    /// Bound on every named RA heading, including occurrence renames and reductions.
    /// This is column arity, not fractional hypertree width or a cardinality bound.
    pub max_arity: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct JoinPlanReport {
    pub options: PlanningOptions,
    pub analysis: Vec<GyoRule>,
    pub join_tree: Option<JoinTree>,
    pub derivation: Vec<Rule>,
    pub peak_arity: usize,
}

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}

pub fn plan(
    source: &cq::Module,
    options: PlanningOptions,
) -> syn::Result<(ra::Module, JoinPlanReport)> {
    let clauses = ClauseDependencies::analyze(source)?;
    if !clauses.supports_graph_rules {
        return clause_rules::plan(source, &clauses, options);
    }
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
        let mut breadth_first = vec![tree.root()];
        let mut i = 0;
        while i < breadth_first.len() {
            breadth_first.extend(tree.children(breadth_first[i]));
            i += 1;
        }
        for &child in breadth_first.iter().skip(1).rev() {
            apply(&mut state, &mut derivation, Rule::Up { child })?;
        }
        for &child in breadth_first.iter().skip(1) {
            apply(&mut state, &mut derivation, Rule::Down { child })?;
        }
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

fn apply(state: &mut JoinState<'_>, trace: &mut Vec<Rule>, rule: Rule) -> syn::Result<()> {
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

pub fn incidence(source: &cq::Module) -> Vec<Heading> {
    source
        .program
        .query
        .body
        .iter()
        .filter_map(cq::BodyItem::atom)
        .map(|atom| atom.variables().map(symbol_name).collect())
        .collect()
}

/// Entry for a separately implemented scheduling policy. The caller supplies
/// administrative commands; the checked kernel constructs the residual RA.
pub fn from_derivation(source: &cq::Module, report: &JoinPlanReport) -> syn::Result<ra::Module> {
    let clauses = ClauseDependencies::analyze(source)?;
    if !clauses.supports_graph_rules {
        return clause_rules::replay(source, &clauses, report);
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
    state.into_module()
}

pub(crate) fn check_analysis(
    edges: &[Heading],
    options: PlanningOptions,
    analysis: &[GyoRule],
    tree: &Option<JoinTree>,
) -> syn::Result<()> {
    if options.strategy == JoinPolicy::Auto {
        let mut gyo = GyoState::new(edges);
        for rule in analysis {
            gyo.apply(rule).map_err(error)?;
        }
        if &gyo.finish().map_err(error)? != tree {
            return Err(error("GYO certificate does not match the chosen route"));
        }
    } else if !analysis.is_empty() || tree.is_some() {
        return Err(error(
            "binary route must not claim a Yannakakis certificate",
        ));
    }
    if let Some(tree) = tree {
        tree.check(edges).map_err(error)?;
    }
    Ok(())
}

pub(crate) fn reserved(source: &cq::Module) -> BTreeSet<String> {
    use quote::ToTokens;
    fn visit(tokens: TokenStream, names: &mut BTreeSet<String>) {
        for token in tokens {
            match token {
                TokenTree::Ident(id) => {
                    names.insert(symbol_name(&id));
                }
                TokenTree::Group(g) => visit(g.stream(), names),
                _ => (),
            }
        }
    }
    let mut names = BTreeSet::new();
    visit(source.to_token_stream(), &mut names);
    names
}

pub(crate) fn fresh(names: &mut BTreeSet<String>, clause: usize, arg: usize) -> proc_macro2::Ident {
    let mut name = format!("__cq_{clause}_{arg}");
    while !names.insert(name.clone()) {
        name.push('_');
    }
    quote::format_ident!("{name}")
}

/// Alternative CQ -> RelationalPlan edge, with automatic graph planning.
pub fn compile_cq_planned(source: &cq::Module) -> syn::Result<ra::Module> {
    compile_cq_with_policy(source, crate::JoinPolicy::Auto)
}

/// Select a planning policy without changing either language boundary.
pub fn compile_cq_with_policy(
    source: &cq::Module,
    policy: crate::JoinPolicy,
) -> syn::Result<ra::Module> {
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
) -> syn::Result<(ra::Module, crate::JoinPlanReport)> {
    cq::contract::check(source)?;
    let (target, report) = crate::cq_to_relational::plan(source, options)?;
    check_cq_plan(source, &target, &report)?;
    Ok((target, report))
}

/// Explicit source-to-target relation; also used by the planned pass gate.
pub fn check_cq_plan(
    source: &cq::Module,
    target: &ra::Module,
    report: &crate::JoinPlanReport,
) -> syn::Result<()> {
    crate::pass_contracts::planned_join::check(source, target, report)
}

/// Apply an independently supplied schedule through the same rule semantics.
pub fn compile_cq_derivation(
    source: &cq::Module,
    report: &crate::JoinPlanReport,
) -> syn::Result<ra::Module> {
    cq::contract::check(source)?;
    let target = crate::cq_to_relational::from_derivation(source, report)?;
    check_cq_plan(source, &target, report)?;
    Ok(target)
}
