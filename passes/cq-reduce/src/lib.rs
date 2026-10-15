//! Add only relational semijoin reductions to a valuation plan.
//! The denotation is `T X = bottom + Set X`; the written Rust chain consists
//! of opaque partial functions. Relational reductions may move earlier across
//! it, but Rust steps retain their relative order and written relational
//! predecessors. I/O, mutation and evaluation counts are outside the meaning.
//! Erasure must preserve every non-reduction step exactly. Rust guards are
//! never pushed onto atom inputs.

//! Reduction recursively retains block paths.
//! Positive semijoins may distribute into union arms and fallback positive arms.
//! Negative operands stay original named branch results.
//! Hypergraph uses a tree reducer only on acyclic scopes, but exact atom-to-block
//! cuts on any scope. Masks reduces along the reduction tree on every scope.
//! Each block branch is analyzed as its own scope.
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
    pub semijoins: Reductions,
}

impl Options {
    fn effective(self, suppress_hypergraph: bool, suppress_tree: bool) -> Self {
        if suppress_hypergraph && self.semijoins == Reductions::Hypergraph
            || suppress_tree && self.semijoins == Reductions::Tree
        {
            Self {
                semijoins: Reductions::None,
            }
        } else {
            self
        }
    }
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
    /// Masks for a left-deep pipeline: the reduction tree is rooted at the
    /// scope's first scheduled clause (the outer stream). Each inner clause is
    /// reduced by its filtered child subtrees, children first, so every support
    /// is complete before the stream starts. The outer is never reduced or a
    /// support: its probes filter it in written order, and no reduced stream is
    /// stored. Exact atom-to-block cuts reduce inner atoms sharing a Rust-free
    /// block's columns. Branches are independent scopes.
    Masks,
}

/// The first scheduled atom or Rust-free block of a scope: its outer stream.
pub fn outer(steps: &[Step], graph: &valuation::cq::hypergraph::Analysis) -> Option<usize> {
    steps.iter().find_map(|step| match step {
        Step::Atom(a, _) if graph.clauses.contains_key(&a.clause()) => Some(a.clause()),
        Step::Block(b, _) if graph.clauses.contains_key(&b.source.clause()) => {
            Some(b.source.clause())
        }
        _ => None,
    })
}

/// The reduction tree rerooted at `root`: each other reachable clause's parent.
pub fn rooted(tree: &[(usize, Option<usize>)], root: usize) -> BTreeMap<usize, usize> {
    let mut adjacent: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for &(child, parent) in tree {
        if let Some(parent) = parent {
            adjacent.entry(child).or_default().insert(parent);
            adjacent.entry(parent).or_default().insert(child);
        }
    }
    let mut parents = BTreeMap::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        for &next in adjacent.get(&node).into_iter().flatten() {
            if next != root && !parents.contains_key(&next) {
                parents.insert(next, node);
                pending.push(next);
            }
        }
    }
    parents
}

/// `(target, support)` mask edges, children first: each inner clause with each
/// child whose subtree carries a relational filter or a block cut. Siblings run
/// strongest first. Supports are always children and the outer (root) is
/// neither a target nor a support.
/// Formal rule R-Masks: Reduce each inner clause by its filtered child subtrees of the outer-rooted reduction tree.
pub fn mask_edges(
    graph: &valuation::cq::hypergraph::Analysis,
    body: &cq::CommaList<valuation::cq::BodyItem>,
    root: usize,
) -> Vec<(usize, usize)> {
    let parents = rooted(&graph.reduction_tree, root);
    let mut children: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (&child, &parent) in &parents {
        children.entry(parent).or_default().push(child);
    }
    let mut local = valuation::cq::hypergraph::filter_ranks(body, graph);
    for (atom, _) in block_cuts(graph, body) {
        local.entry(atom).and_modify(|rank| *rank = (*rank).min(2));
    }
    fn subtree(
        node: usize,
        children: &BTreeMap<usize, Vec<usize>>,
        local: &BTreeMap<usize, u8>,
        ranks: &mut BTreeMap<usize, u8>,
    ) -> u8 {
        let mut rank = local.get(&node).copied().unwrap_or(3);
        for &child in children.get(&node).into_iter().flatten() {
            rank = rank.min(subtree(child, children, local, ranks));
        }
        ranks.insert(node, rank);
        rank
    }
    let mut ranks = BTreeMap::new();
    subtree(root, &children, &local, &mut ranks);
    for siblings in children.values_mut() {
        siblings.sort_by_key(|c| (ranks[c], *c));
    }
    fn visit(
        node: usize,
        root: usize,
        children: &BTreeMap<usize, Vec<usize>>,
        ranks: &BTreeMap<usize, u8>,
        edges: &mut Vec<(usize, usize)>,
    ) {
        for &child in children.get(&node).into_iter().flatten() {
            visit(child, root, children, ranks, edges);
            if node != root && ranks[&child] < 3 {
                edges.push((node, child));
            }
        }
    }
    let mut edges = Vec::new();
    visit(root, root, &children, &ranks, &mut edges);
    edges
}

/// Exact atom-to-block cuts `(atom, block)` on shared columns.
pub fn block_cuts(
    graph: &valuation::cq::hypergraph::Analysis,
    body: &cq::CommaList<valuation::cq::BodyItem>,
) -> Vec<(usize, usize)> {
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
                cuts.push((clause, block));
            }
        }
    }
    cuts
}

/// The Masks schedule of one scope: block cuts, then outer-rooted mask edges.
fn mask_steps(
    body: &cq::CommaList<valuation::cq::BodyItem>,
    steps: &[Step],
    analysis: &valuation::Analysis,
    graph: &valuation::cq::hypergraph::Analysis,
) -> Vec<Step> {
    let Some(root) = outer(steps, graph) else {
        return Vec::new();
    };
    let mut edges: Vec<_> = block_cuts(graph, body)
        .into_iter()
        .filter(|&(atom, _)| atom != root)
        .collect();
    for edge in mask_edges(graph, body, root) {
        if body[edge.0].atom().is_some() && !edges.contains(&edge) {
            edges.push(edge);
        }
    }
    edges
        .into_iter()
        .map(|(target, support)| semijoin(analysis, graph, target, support))
        .collect()
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
    let effective = options.effective(source.valuation.join_branches.is_some(), false);
    let options = &effective;
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
    if options.semijoins == Reductions::Masks {
        if let Some(first) = first_atom {
            before.entry(first).or_default().extend(mask_steps(
                body,
                source_steps,
                analysis,
                graph,
            ));
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
                let options = options.effective(false, !inner.acyclic);
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
    let tree = &graph.reduction_tree;
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
