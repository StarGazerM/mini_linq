//! Add only relational semijoin reductions to a valuation plan.
//! The denotation is `T X = bottom + Set X`; the written Rust chain consists
//! of opaque partial functions. Relational reductions may move earlier across
//! it, but Rust steps retain their relative order and written relational
//! predecessors. I/O, mutation and evaluation counts are outside the meaning.
//! Erasure must preserve every non-reduction step exactly. Rust guards are
//! never pushed onto atom inputs, including under the legacy `Atoms` option.

//! Reduction recursively retains block paths.
//! Positive semijoins may distribute into union arms and fallback positive arms.
//! Negative operands stay original named branch results.
//! Hypergraph uses a tree reducer only on acyclic scopes, but exact atom-to-block
//! cuts on any scope. Each block branch is analyzed as its own scope.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
//!
use std::collections::{BTreeMap, BTreeSet};

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
    /// Filter-supported tree reductions on acyclic scopes, plus exact
    /// atom-to-block cuts on any scope. Branches are independent scopes.
    Hypergraph,
}

/// Insert the reductions `options` ask for, then check the VO contract.
/// Formal rule R-Guard: Retain every opaque Rust guard in the written chain.
/// Formal rule R-Tree: Insert the relational tree schedule before the first atom.
pub fn compile(source: &valuation::Program, options: &Options) -> syn::Result<valuation::Program> {
    let analysis = valuation::contract::check(source)?;
    let graph = valuation::cq::hypergraph::Analysis::of(&source.cq);
    // Independent branches are exact build-side joins, not fact-dependent
    // semijoin supports. The optional Hypergraph policy does not interpose a
    // reducer across this recorded physical decomposition.
    let branch_options = Options { guards: options.guards, semijoins: Reductions::None };
    let options = if source.valuation.join_branches.is_some() && options.semijoins == Reductions::Hypergraph {
        &branch_options
    } else {
        options
    };
    let steps = compile_steps(
        &source.cq.query.body,
        &source.valuation.steps,
        &analysis,
        &graph,
        None,
        options,
    )?;
    let program = valuation::Program {
        cq: source.cq.clone(),
        valuation: valuation::Valuation {
            valuation_token: source.valuation.valuation_token,
            join_branches: source.valuation.join_branches.clone(),
            brace: Default::default(),
            steps,
        },
    };
    valuation::contract::check(&program)?;
    Ok(program)
}

fn compile_steps(
    body: &cq::CommaList<valuation::cq::BodyItem>,
    source_steps: &[Step],
    analysis: &valuation::Analysis,
    graph: &valuation::cq::hypergraph::Analysis,
    parent: Option<(valuation::Occurrence, usize)>,
    options: &Options,
) -> syn::Result<Vec<Step>> {
    if let Some(step) = source_steps
        .iter()
        .find(|step| matches!(step, Step::Reduce(..)))
    {
        return Err(syn::Error::new_spanned(
            step,
            "the plan already carries reductions",
        ));
    }
    let first_atom = source_steps
        .iter()
        .position(|step| matches!(step, Step::Atom(..) | Step::Block(..)));
    let mut before: BTreeMap<usize, Vec<Step>> = BTreeMap::new();
    if options.semijoins == Reductions::Tree
        || options.semijoins == Reductions::Hypergraph && graph.acyclic
    {
        let reducer = if options.semijoins == Reductions::Hypergraph {
            filtered_tree(graph, analysis, body)?
        } else {
            tree(graph, analysis, body)?
        };
        if let Some(first) = first_atom {
            before.entry(first).or_default().extend(reducer);
        }
    }
    if options.semijoins == Reductions::Hypergraph {
        let mut cuts = Vec::new();
        for (&block, _) in graph
            .clauses
            .iter()
            .filter(|(c, _)| matches!(&body[**c], valuation::cq::BodyItem::Block(_)))
        {
            for (&clause, _) in graph
                .clauses
                .iter()
                .filter(|(c, _)| body[**c].atom().is_some())
            {
                if !graph.shared(clause, block).is_empty() {
                    cuts.push(semijoin(analysis, graph, clause, block));
                }
            }
        }
        if let Some(first) = first_atom {
            let existing = before.entry(first).or_default();
            cuts.retain(|cut| !existing.contains(cut));
            cuts.extend(std::mem::take(existing));
            *existing = cuts;
        }
    }
    let required: BTreeSet<_> = analysis
        .domain_requires
        .values()
        .flatten()
        .cloned()
        .collect();
    if !required.is_empty() {
        let position = analysis
            .bound_before
            .iter()
            .position(|names| required.iter().all(|name| names.contains(name)))
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    body,
                    "block finite domain requires its written Rust context",
                )
            })?;
        let reductions = std::mem::take(&mut before)
            .into_values()
            .flatten()
            .collect();
        before.insert(position, reductions);
    }
    let mut steps =
        Vec::with_capacity(source_steps.len() + before.values().map(Vec::len).sum::<usize>());
    for (position, step) in source_steps.iter().enumerate() {
        if let Some(reductions) = before.remove(&position) {
            steps.extend(reductions.into_iter().map(|mut step| {
                if let Step::Reduce(reduce, _) = &mut step {
                    let target = reduce.clause();
                    let Reduction::With(_, support) = &mut reduce.reduction;
                    let clause = support.clause();
                    reduce.clause = parent.as_ref().map_or_else(
                        || valuation::Occurrence::new(target),
                        |(path, j)| path.child(*j, target),
                    );
                    support.clause = parent.as_ref().map_or_else(
                        || valuation::Occurrence::new(clause),
                        |(path, j)| path.child(*j, clause),
                    );
                }
                step
            }));
        }
        let mut step = step.clone();
        if let Step::Block(block, _) = &mut step {
            let clause = block.source.clause();
            let valuation::cq::BodyItem::Block(written) = &body[clause] else {
                unreachable!("checked block")
            };
            for (j, branch) in block.branches.iter_mut().enumerate() {
                let info = &analysis.branches[&clause][j];
                let context = info
                    .bound_before
                    .first()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                let inner = valuation::cq::hypergraph::Analysis::of_body(
                    &written.branches[j].body,
                    &context,
                );
                let options = Options {
                    guards: options.guards,
                    semijoins: if options.semijoins == Reductions::Tree && !inner.acyclic {
                        Reductions::None
                    } else {
                        options.semijoins
                    },
                };
                branch.steps = compile_steps(
                    &written.branches[j].body,
                    &branch.steps,
                    info,
                    &inner,
                    Some((block.source.clause.clone(), j)),
                    &options,
                )?;
            }
        }
        steps.push(step);
    }
    Ok(steps)
}

mod correspondence;
pub use correspondence::check;

/// Erase only semijoin reductions, preserving all opaque Rust positions.
fn erase_steps(steps: &[Step]) -> Vec<Step> {
    steps
        .iter()
        .filter(|step| !matches!(step, Step::Reduce(..)))
        .cloned()
        .map(|mut step| {
            if let Step::Block(block, _) = &mut step {
                for branch in &mut block.branches {
                    branch.steps = erase_steps(&branch.steps);
                }
            }
            step
        })
        .collect()
}

/// The Yannakakis full reducer: Y-UP over every tree edge, leaves first,
/// then Y-DOWN, root first: reverse BFS upward, BFS downward.
/// Formal rule Y-Up: Produce upward parent-with-child VO steps in reverse BFS order.
/// Formal rule Y-Down: Produce downward child-with-parent VO steps in BFS order.
fn tree(
    graph: &valuation::cq::hypergraph::Analysis,
    analysis: &valuation::Analysis,
    body: &cq::CommaList<valuation::cq::BodyItem>,
) -> syn::Result<Vec<Step>> {
    let Some(join_tree) = &graph.join_tree else {
        return Err(syn::Error::new_spanned(
            body,
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
        .map(|(target, support)| semijoin(analysis, graph, target, support))
        .collect())
}

/// Formal rule R-Filtered: Propagate independently filtered components, omitting range-only support against a point-filtered component; order strongest siblings first.
fn filtered_tree(
    graph: &valuation::cq::hypergraph::Analysis,
    analysis: &valuation::Analysis,
    body: &cq::CommaList<valuation::cq::BodyItem>,
) -> syn::Result<Vec<Step>> {
    let Some(tree) = &graph.join_tree else {
        return Err(syn::Error::new_spanned(
            body,
            "filtered tree reduction requires an acyclic query",
        ));
    };
    let mut children: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut root = None;
    for &(node, parent) in tree {
        if let Some(parent) = parent {
            children.entry(parent).or_default().push(node);
        } else {
            root = Some(node);
        }
    }
    let Some(root) = root else {
        return Ok(Vec::new());
    };
    let local = valuation::cq::hypergraph::filter_ranks(body, graph);
    let mut ranks = local.clone();
    fn summarize(
        node: usize,
        children: &BTreeMap<usize, Vec<usize>>,
        ranks: &mut BTreeMap<usize, u8>,
    ) -> u8 {
        let mut rank = ranks[&node];
        for &child in children.get(&node).into_iter().flatten() {
            rank = rank.min(summarize(child, children, ranks));
        }
        ranks.insert(node, rank);
        rank
    }
    summarize(root, &children, &mut ranks);
    for siblings in children.values_mut() {
        siblings.sort_by_key(|c| (ranks[c], *c));
    }
    fn up(
        node: usize,
        children: &BTreeMap<usize, Vec<usize>>,
        ranks: &BTreeMap<usize, u8>,
        edges: &mut Vec<(usize, usize)>,
    ) {
        for &child in children.get(&node).into_iter().flatten() {
            up(child, children, ranks, edges);
            if ranks[&child] < 3 {
                edges.push((node, child));
            }
        }
    }
    fn down(
        node: usize,
        outside: u8,
        children: &BTreeMap<usize, Vec<usize>>,
        local: &BTreeMap<usize, u8>,
        ranks: &BTreeMap<usize, u8>,
        edges: &mut Vec<(usize, usize)>,
        outside_ranks: &mut BTreeMap<usize, u8>,
    ) {
        for &child in children.get(&node).into_iter().flatten() {
            let support = children
                .get(&node)
                .into_iter()
                .flatten()
                .filter(|&&sibling| sibling != child)
                .fold(outside.min(local[&node]), |rank, sibling| {
                    rank.min(ranks[sibling])
                });
            // A filter learned only from this child cannot prune it on return.
            outside_ranks.insert(child, support);
            if support < 3 {
                edges.push((child, node));
            }
            down(child, support, children, local, ranks, edges, outside_ranks);
        }
    }
    let mut edges = Vec::new();
    up(root, &children, &ranks, &mut edges);
    let mut outside = BTreeMap::new();
    down(root, 3, &children, &local, &ranks, &mut edges, &mut outside);
    edges.retain(|&(target, support)| {
        let upward = tree
            .iter()
            .any(|&(child, parent)| child == support && parent == Some(target));
        let (support_rank, target_rank) = if upward {
            (ranks[&support], outside[&support])
        } else {
            (outside[&target], ranks[&target])
        };
        // A range-only branch should stream against the point-filtered driving
        // branch, not force a full range index merely to prune that branch.
        !(support_rank == 1 && target_rank == 0)
    });
    Ok(edges
        .into_iter()
        .map(|(target, support)| semijoin(analysis, graph, target, support))
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
        clause: index(clause).into(),
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
            clause: index(clause).into(),
            reduction,
        },
        Default::default(),
    )
}

fn index(clause: usize) -> LitInt {
    LitInt::new(&clause.to_string(), Span::call_site())
}
