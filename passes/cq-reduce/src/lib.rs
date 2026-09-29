//! VO → VO: the reductions a plan applies to a clause's relation before the
//! clause is joined.
//!
//! This pass decides one thing: which `reduce` steps the plan carries, and
//! where. The order and the granularity of the input plan are kept as they
//! are. A reduction is a semijoin or a selection that, under set semantics,
//! changes nothing the plan denotes, so [`check`] erases them — drops the
//! semijoins and returns each pushed guard to a step — and requires the
//! input plan back. The checker validates tree edges and dependency order
//! independently of the scheduler; sibling orders and guard hosts are free.
//!
//! - `Pushdown::Atoms` turns every guard whose variables all belong to one
//!   atom clause into `reduce C@k if (e);` on the earliest such clause,
//!   placed before that clause's first step; the guard's `if` step goes. A
//!   guard the lexical analysis cannot see through (`opaque`) stays a step.
//! - `Reductions::Tree` is the Yannakakis full reducer over the GYO join tree
//!   computed from the source program's CQ, so the query must be acyclic: an upward
//!   pass, leaves toward the root, `reduce parent with child(shared)`, then a
//!   downward pass, root toward the leaves, `reduce child with parent(shared)`,
//!   the edges visited in breadth-first order from the root. Pushed guards
//!   precede the reducer, and the reducer precedes the first atom step.
//!
//! `None`/`None` is the identity. A plan that already carries reductions is
//! refused rather than reduced twice.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_vo as valuation;
use cq_lang_vo::syntax as cq;
use cq_lang_vo::syntax::rust;
use cq_lang_vo::syntax::symbol_name;
use cq_lang_vo::{Column, Reduce, Reduction, Step, Subatom};
use proc_macro2::Span;
use syn::LitInt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub guards: Pushdown,
    pub semijoins: Reductions,
}

/// Which guards become reductions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pushdown {
    #[default]
    None,
    /// Every guard over the variables of one atom clause.
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
    let first_step =
        |clause: usize| {
            source.valuation.steps.iter().position(
                |step| matches!(step, Step::Atom(subatom, _) if subatom.clause() == clause),
            )
        };
    // The reductions to insert before each step, and the guard steps that
    // become reductions.
    let mut before: BTreeMap<usize, Vec<Step>> = BTreeMap::new();
    let mut pushed: BTreeSet<usize> = BTreeSet::new();
    if options.guards == Pushdown::Atoms {
        for (position, step) in source.valuation.steps.iter().enumerate() {
            let Step::Guard(filter, _) = step else {
                continue;
            };
            let free = rust::free_variables(&filter.condition);
            if free.opaque {
                continue;
            }
            let host = analysis.atoms.iter().find(|(_, atom)| {
                free.names
                    .iter()
                    .all(|v| atom.variables().any(|c| symbol_name(c) == *v))
            });
            let Some((&clause, atom)) = host else {
                continue;
            };
            let at = match options.semijoins {
                Reductions::Tree => first_atom,
                Reductions::None => first_step(clause),
            }
            .expect("the contract binds every atom clause");
            before.entry(at).or_default().push(reduce(
                clause,
                atom,
                Reduction::Guard(filter.clone()),
            ));
            pushed.insert(position);
        }
    }
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
        if !pushed.contains(&position) {
            steps.push(step.clone());
        }
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

/// The plan's steps without its reductions, and the guards those placed.
fn erase(module: &valuation::Program) -> (Vec<Step>, Vec<cq::Filter>) {
    let mut steps = Vec::new();
    let mut guards = Vec::new();
    for step in &module.valuation.steps {
        match step {
            Step::Reduce(
                Reduce {
                    reduction: Reduction::Guard(filter),
                    ..
                },
                _,
            ) => guards.push(filter.clone()),
            Step::Reduce(..) => {}
            step => steps.push(step.clone()),
        }
    }
    (steps, guards)
}

/// `steps` without one guard step per placed guard.
fn without_guards(steps: &[Step], placed: &[cq::Filter]) -> Vec<Step> {
    let mut pending: Vec<&cq::Filter> = placed.iter().collect();
    steps
        .iter()
        .filter(|step| {
            let Step::Guard(filter, _) = step else {
                return true;
            };
            match pending.iter().position(|p| p.condition == filter.condition) {
                Some(i) => {
                    pending.remove(i);
                    false
                }
                None => true,
            }
        })
        .cloned()
        .collect()
}

/// The Yannakakis full reducer: Y-UP over every tree edge, leaves first,
/// then Y-DOWN, root first, both in breadth-first order from the root.
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
        .map(|(target, support)| semijoin(analysis, target, support))
        .collect())
}

/// `reduce target with support(shared)`: the support's columns that are
/// variables of the target, `_` elsewhere.
fn semijoin(analysis: &valuation::Analysis, target: usize, support: usize) -> Step {
    let atom = &analysis.atoms[&target];
    let shared: Vec<String> = atom.variables().map(symbol_name).collect();
    let subatom = subatom(support, &analysis.atoms[&support], |v| {
        shared.iter().any(|s| s == v)
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
