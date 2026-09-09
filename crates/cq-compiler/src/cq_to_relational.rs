//! CQ -> RelationalPlan: analysis, administrative rules, and scheduling policy.
pub(crate) mod analysis;
mod rules;

pub use analysis::{GyoRule, JoinTree};
use analysis::{GyoState, Heading};
use cq_ir::{cq, relational_plan as ra, symbol_name};
pub use rules::Rule;
pub(crate) mod ordered;
mod strategy;
use rules::Kernel;

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
    cq::contract::check(source)?;
    if ordered::required(source) {
        return ordered::plan(source, options);
    }
    let edges = incidence(source);
    let (analysis, join_tree) = if options.strategy == JoinPolicy::Auto {
        analysis::analyze(&edges)
    } else {
        (Vec::new(), None)
    };
    if let Some(tree) = &join_tree {
        tree.check(&edges).map_err(error)?;
    }
    let mut state = Kernel::new(source, join_tree.clone(), options.max_arity);
    let mut derivation = Vec::new();
    for occurrence in 0..edges.len() {
        apply(&mut state, &mut derivation, Rule::Introduce { occurrence })?;
    }
    while let Some(rule) = state.ready_selection() {
        apply(&mut state, &mut derivation, rule)?;
    }
    apply(&mut state, &mut derivation, Rule::Begin)?;
    if let Some(tree) = &join_tree {
        let mut preorder = vec![tree.root()];
        let mut i = 0;
        while i < preorder.len() {
            preorder.extend(tree.children(preorder[i]));
            i += 1;
        }
        for &child in preorder.iter().skip(1).rev() {
            apply(&mut state, &mut derivation, Rule::Up { child })?;
        }
        for &child in preorder.iter().skip(1) {
            apply(&mut state, &mut derivation, Rule::Down { child })?;
        }
        apply(&mut state, &mut derivation, Rule::Freeze)?;
    }
    normalize(&mut state, &mut derivation)?;
    while state.frontier().len() > 1 {
        let (left, right) = strategy::choose_pair(&state, options.strategy, join_tree.as_ref());
        apply(&mut state, &mut derivation, Rule::Combine { left, right })?;
        normalize(&mut state, &mut derivation)?;
    }
    let group = *state.frontier().first_key_value().unwrap().0;
    apply(&mut state, &mut derivation, Rule::Output { group })?;
    let ra = state.finish()?;
    let peak_arity = ra::check_in(&source.program.inputs, &ra.plan)?
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

fn apply(state: &mut Kernel<'_>, trace: &mut Vec<Rule>, rule: Rule) -> syn::Result<()> {
    state.apply(&rule)?;
    trace.push(rule);
    Ok(())
}

fn normalize(state: &mut Kernel<'_>, trace: &mut Vec<Rule>) -> syn::Result<()> {
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
    if ordered::required(source) {
        return ordered::replay(source, report);
    }
    check_analysis(
        &incidence(source),
        report.options,
        &report.analysis,
        &report.join_tree,
    )?;
    let mut state = Kernel::new(source, report.join_tree.clone(), report.options.max_arity);
    for rule in &report.derivation {
        state.apply(rule)?;
    }
    state.finish()
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
