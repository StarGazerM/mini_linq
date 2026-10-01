//! Add only relational semijoin reductions to a valuation plan.
//! The denotation is `T X = bottom + Set X`; the written Rust chain consists
//! of opaque partial functions. Relational reductions may move earlier across
//! it, but Rust steps retain their relative order and written relational
//! predecessors. I/O, mutation and evaluation counts are outside the meaning.
//! Erasure must preserve every non-reduction step exactly. Rust guards are
//! never pushed onto atom inputs, including under the legacy `Atoms` option.

use std::collections::BTreeMap;

use cq_lang_vo as valuation;
use cq_lang_vo::syntax as cq;
use cq_lang_vo::syntax::symbol_name;
use cq_lang_vo::{Column, Reduce, Reduction, Step, Subatom};
use proc_macro2::Span;
use syn::LitInt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub guards: Pushdown,
    pub semijoins: Reductions,
}

/// Compatibility spellings; Rust guards always remain in their chain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pushdown {
    #[default]
    None,
    /// Compatibility spelling; opaque Rust guards always stay in the chain.
    Atoms,
}

/// Which semijoin reductions the plan carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reductions {
    #[default]
    None,
    /// The Yannakakis full reducer over the join tree.
    Tree,
}

/// Insert the reductions `options` ask for, then check the VO contract.
/// Formal rule R-Guard: Retain every opaque Rust guard in the written chain.
/// Formal rule R-Tree: Insert the relational tree schedule before the first atom.
pub fn compile(source: &valuation::Program, options: &Options) -> syn::Result<valuation::Program> {
    let analysis = valuation::contract::check(source)?;
    if let Some(step) = source
        .valuation
        .steps
        .iter()
        .find(|step| matches!(step, Step::Reduce(..)))
    {
        return Err(syn::Error::new_spanned(
            step,
            "the plan already carries reductions",
        ));
    }
    let first_atom = source
        .valuation
        .steps
        .iter()
        .position(|step| matches!(step, Step::Atom(..)));
    let mut before: BTreeMap<usize, Vec<Step>> = BTreeMap::new();
    if options.semijoins == Reductions::Tree {
        let reducer = tree(source, &analysis)?;
        if let Some(first) = first_atom {
            before.entry(first).or_default().extend(reducer);
        }
    }
    let mut steps = Vec::with_capacity(
        source.valuation.steps.len() + before.values().map(Vec::len).sum::<usize>(),
    );
    for (position, step) in source.valuation.steps.iter().enumerate() {
        if let Some(reductions) = before.remove(&position) {
            steps.extend(reductions);
        }
        steps.push(step.clone());
    }
    let program = valuation::Program {
        cq: source.cq.clone(),
        valuation: valuation::Valuation {
            valuation_token: source.valuation.valuation_token,
            brace: Default::default(),
            steps,
        },
    };
    valuation::contract::check(&program)?;
    Ok(program)
}

mod correspondence;
pub use correspondence::check;

/// Erase only semijoin reductions, preserving all opaque Rust positions.
fn erase(module: &valuation::Program) -> Vec<Step> {
    module
        .valuation
        .steps
        .iter()
        .filter(|step| !matches!(step, Step::Reduce(..)))
        .cloned()
        .collect()
}

/// The Yannakakis full reducer: Y-UP over every tree edge, leaves first,
/// then Y-DOWN, root first: reverse BFS upward, BFS downward.
/// Formal rule Y-Up: Produce upward parent-with-child VO steps in reverse BFS order.
/// Formal rule Y-Down: Produce downward child-with-parent VO steps in BFS order.
fn tree(source: &valuation::Program, analysis: &valuation::Analysis) -> syn::Result<Vec<Step>> {
    let Some(join_tree) = valuation::cq::hypergraph::Analysis::of(&source.cq).join_tree else {
        return Err(syn::Error::new_spanned(
            source.valuation.valuation_token,
            "Yannakakis reduction requires an acyclic query",
        ));
    };
    let parent: BTreeMap<usize, Option<usize>> = join_tree.iter().copied().collect();
    let Some(root) = parent
        .iter()
        .find_map(|(&clause, p)| p.is_none().then_some(clause))
    else {
        return Ok(Vec::new());
    };
    let mut order = vec![root];
    let mut next = 0;
    while next < order.len() {
        let node = order[next];
        order.extend(
            parent
                .iter()
                .filter_map(|(&child, p)| (*p == Some(node)).then_some(child)),
        );
        next += 1;
    }
    let edge = |&child: &usize| (parent[&child].expect("a non-root node"), child);
    let up = order.iter().skip(1).rev().map(edge);
    let down = order.iter().skip(1).map(edge).map(|(p, c)| (c, p));
    Ok(up
        .chain(down)
        .map(|(target, support)| {
            semijoin(
                analysis,
                &valuation::cq::hypergraph::Analysis::of(&source.cq),
                target,
                support,
            )
        })
        .collect())
}

/// `reduce target with support(shared)`: the support's columns that are
/// variables of the target, `_` elsewhere.
fn semijoin(
    analysis: &valuation::Analysis,
    hypergraph: &valuation::cq::hypergraph::Analysis,
    target: usize,
    support: usize,
) -> Step {
    let atom = &analysis.atoms[&target];
    let shared: Vec<String> = atom.variables().map(symbol_name).collect();
    let subatom = subatom(support, &analysis.atoms[&support], |v| {
        shared.iter().any(|s| hypergraph.equivalent(s, v))
    });
    reduce(target, atom, Reduction::With(Default::default(), subatom))
}

fn subatom(clause: usize, atom: &cq::Atom, keep: impl Fn(&str) -> bool) -> Subatom {
    let columns = atom
        .variables()
        .map(|v| {
            if keep(&symbol_name(v)) {
                Column::Variable(v.clone())
            } else {
                Column::Dropped(Default::default())
            }
        })
        .collect();
    Subatom {
        relation: atom.relation.clone(),
        at: Default::default(),
        clause: index(clause),
        paren: Default::default(),
        columns,
    }
}

fn reduce(clause: usize, atom: &cq::Atom, reduction: Reduction) -> Step {
    Step::Reduce(
        Reduce {
            reduce_token: Default::default(),
            relation: atom.relation.clone(),
            at: Default::default(),
            clause: index(clause),
            reduction,
        },
        Default::default(),
    )
}

fn index(clause: usize) -> LitInt {
    LitInt::new(&clause.to_string(), Span::call_site())
}
