//! Reduction erasure and obligations, without the reducer's schedule builder.

use super::{Options, Reductions};
use cq_lang_vo::{self as vo, Reduction, Step};
use std::collections::BTreeMap;

/// Check the source domain, unchanged query/steps, and reduction obligations.
/// A full tree reducer obeys dependencies, but need not use breadth-first order.
/// Formal rule CK-Reduce: Check branch-preserving erasure, no optional reducer for independent branches, otherwise filtered support versions, tree obligations and exact block cuts.
pub fn check(source: &vo::Program, target: &vo::Program, options: &Options) -> syn::Result<()> {
    let source_info = vo::contract::check(source)?;
    vo::contract::check(target)?;
    if target.cq != source.cq {
        return Err(syn::Error::new_spanned(
            &target.cq.name,
            format!(
                "query `{}`: reductions must preserve source CQ `{}`",
                target.cq.name, source.cq.name
            ),
        ));
    }
    if target.valuation.join_branches != source.valuation.join_branches {
        return Err(syn::Error::new_spanned(
            &target.valuation,
            "reductions must preserve independent join branches",
        ));
    }
    let branch_options = Options {
        guards: options.guards,
        semijoins: Reductions::None,
    };
    let options = if source.valuation.join_branches.is_some()
        && options.semijoins == Reductions::Hypergraph
    {
        &branch_options
    } else {
        options
    };
    check_steps(
        &source.cq.query.body,
        &source.valuation.steps,
        &target.valuation.steps,
        &source_info,
        &vo::cq::hypergraph::Analysis::of(&source.cq),
        options,
        &source.cq.name,
    )
}

fn check_steps(
    body: &vo::syntax::CommaList<vo::cq::BodyItem>,
    source_steps: &[Step],
    target_steps: &[Step],
    source_info: &vo::Analysis,
    graph: &vo::cq::hypergraph::Analysis,
    options: &Options,
    name: &syn::Ident,
) -> syn::Result<()> {
    for step in source_steps {
        if let Step::Reduce(r, _) = step {
            return Err(syn::Error::new_spanned(
                r,
                format!(
                    "clause #{} (`{}`): source already carries reductions",
                    r.clause(),
                    r.relation
                ),
            ));
        }
    }
    let graph = graph.clone();
    let tree = if options.semijoins == Reductions::Tree
        || options.semijoins == Reductions::Hypergraph && graph.acyclic
    {
        Some(graph.clone().join_tree.ok_or_else(|| {
            syn::Error::new_spanned(
                body,
                format!("query `{}`: Tree reduction requires an acyclic query", name),
            )
        })?)
    } else {
        None
    };
    let steps = super::erase_steps(target_steps);
    let retained = source_steps;
    if steps.as_slice() != retained {
        let position = steps
            .iter()
            .zip(retained)
            .position(|(a, b)| a != b)
            .unwrap_or(steps.len().min(retained.len()));
        let construct = steps.get(position).or_else(|| retained.get(position));
        return Err(syn::Error::new_spanned(
            body,
            format!(
                "query `{}`: reduction erasure changes source step {position}: {}",
                name,
                construct
                    .map(|s| match s {
                        Step::Block(b, _) => format!("block #{}", b.source.clause()),
                        Step::Atom(a, _) => format!("clause #{} (`{}`)", a.clause(), a.relation),
                        Step::Equality(_, _) => "relational equality".into(),
                        Step::Constraint(_, _) => "relational constraint".into(),
                        Step::Guard(_, _) => "guard clause".into(),
                        Step::Let(_, _) => "binding clause".into(),
                        Step::Reduce(r, _) => format!("reduction of clause #{}", r.clause()),
                    })
                    .unwrap_or_default()
            ),
        ));
    }
    let mut edges = BTreeMap::new();
    let parents: BTreeMap<usize, usize> = tree
        .into_iter()
        .flatten()
        .filter_map(|(child, parent)| parent.map(|p| (child, p)))
        .collect();
    let local_ranks = vo::cq::hypergraph::filter_ranks(body, &graph);
    let mut subtree_ranks = local_ranks.clone();
    // Independent ancestor closure, rather than the constructor's DFS.
    for (&node, &rank) in &local_ranks {
        let mut node = node;
        while let Some(&parent) = parents.get(&node) {
            subtree_ranks
                .entry(parent)
                .and_modify(|r| *r = (*r).min(rank));
            node = parent;
        }
    }
    let filtered = options.semijoins == Reductions::Hypergraph;
    let component_rank = |target: usize, support: usize| {
        // Independently inspect the component after cutting this edge.
        let mut pending = vec![(support, target)];
        let mut rank = 3;
        while let Some((node, previous)) = pending.pop() {
            rank = rank.min(local_ranks[&node]);
            for (&child, &parent) in &parents {
                if parent == node && child != previous {
                    pending.push((child, node));
                } else if child == node && parent != previous {
                    pending.push((parent, node));
                }
            }
        }
        rank
    };
    let required = |target: usize, support: usize| {
        if !filtered {
            return true;
        }
        let rank = component_rank(target, support);
        rank < 3 && !(rank == 1 && component_rank(support, target) == 0)
    };
    let mut versions = local_ranks.clone();
    let mut previous_support: BTreeMap<usize, u8> = BTreeMap::new();
    let mut downward = false;
    for (position, step) in target_steps.iter().enumerate() {
        let Step::Reduce(r, _) = step else { continue };
        let clause = r.clause();
        let fail = |message: &str| {
            syn::Error::new_spanned(r, format!("clause #{clause} (`{}`): {message}", r.relation))
        };
        match &r.reduction {
            Reduction::With(_, support) => {
                if options.semijoins == Reductions::None {
                    return Err(fail("semijoins=None forbids `reduce ... with`"));
                }
                let other = support.clause();
                let up = parents.get(&other) == Some(&clause);
                let down = parents.get(&clause) == Some(&other);
                let block_cut = options.semijoins == Reductions::Hypergraph
                    && body[clause].atom().is_some()
                    && matches!(&body[other], vo::cq::BodyItem::Block(_))
                    && graph.clauses.contains_key(&other)
                    && !graph.shared(clause, other).is_empty();
                if !up && !down && !block_cut {
                    return Err(fail(&format!(
                        "semijoin with clause #{other} is not a join-tree edge"
                    )));
                }
                if filtered {
                    let rank = versions[&other];
                    if rank == 3 {
                        return Err(fail(
                            "semijoin support has no relational filter or filtered reduction",
                        ));
                    }
                    if !block_cut && !required(clause, other) {
                        return Err(fail(
                            "support edge is not selected by independent component filter ranks",
                        ));
                    }
                    if up && !block_cut {
                        let rank = subtree_ranks[&other];
                        if previous_support
                            .insert(clause, rank)
                            .is_some_and(|previous| previous > rank)
                        {
                            return Err(fail("upward supports must run strongest filtered first"));
                        }
                    }
                    versions.entry(clause).and_modify(|r| *r = (*r).min(rank));
                }
                if edges.insert((clause, other), position).is_some() {
                    return Err(fail(&format!(
                        "duplicate join-tree reduction with clause #{other}"
                    )));
                }
                if up && downward && !block_cut {
                    return Err(fail(
                        "upward child-to-parent reductions must precede every downward reduction",
                    ));
                }
                downward |= down && !block_cut;
            }
        }
    }
    if options.semijoins == Reductions::Hypergraph {
        for (&block, _) in graph
            .clauses
            .iter()
            .filter(|(c, _)| matches!(&body[**c], vo::cq::BodyItem::Block(_)))
        {
            for (&clause, _) in graph
                .clauses
                .iter()
                .filter(|(c, _)| body[**c].atom().is_some())
            {
                if !graph.shared(clause, block).is_empty() && !edges.contains_key(&(clause, block))
                {
                    return Err(syn::Error::new_spanned(
                        body,
                        format!("atom #{clause} lacks its block #{block} reduction"),
                    ));
                }
            }
        }
    }
    for (&child, &parent) in &parents {
        for edge in [(parent, child), (child, parent)] {
            if required(edge.0, edge.1) && !edges.contains_key(&edge) {
                return Err(syn::Error::new_spanned(
                    &source_info.atoms[&edge.0],
                    format!(
                        "clause #{}: missing full-reducer edge with clause #{}",
                        edge.0, edge.1
                    ),
                ));
            }
        }
    }
    for (&child, &parent) in &parents {
        if let Some(&grandparent) = parents.get(&parent) {
            if edges
                .get(&(parent, child))
                .zip(edges.get(&(grandparent, parent)))
                .is_some_and(|(a, b)| a > b)
            {
                return Err(syn::Error::new_spanned(
                    &source_info.atoms[&parent],
                    format!("clause #{parent}: upward reduction must follow child clause #{child}"),
                ));
            }
            if edges
                .get(&(parent, grandparent))
                .zip(edges.get(&(child, parent)))
                .is_some_and(|(a, b)| a > b)
            {
                return Err(syn::Error::new_spanned(
                    &source_info.atoms[&child],
                    format!(
                        "clause #{child}: downward reduction must follow parent clause #{parent}"
                    ),
                ));
            }
        }
    }
    for source_step in source_steps {
        let Step::Block(source_block, _) = source_step else {
            continue;
        };
        let target_block = target_steps
            .iter()
            .find_map(|step| match step {
                Step::Block(block, _) if block.source.clause == source_block.source.clause => {
                    Some(block)
                }
                _ => None,
            })
            .expect("erasure preserves blocks");
        let clause = source_block.source.clause();
        let vo::cq::BodyItem::Block(written) = &body[clause] else {
            unreachable!("checked block")
        };
        for (j, (source_branch, target_branch)) in source_block
            .branches
            .iter()
            .zip(&target_block.branches)
            .enumerate()
        {
            let info = &source_info.branches[&clause][j];
            let context = info
                .bound_before
                .first()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect();
            let inner = vo::cq::hypergraph::Analysis::of_body(&written.branches[j].body, &context);
            let options = Options {
                guards: options.guards,
                semijoins: if options.semijoins == Reductions::Tree && !inner.acyclic {
                    Reductions::None
                } else {
                    options.semijoins
                },
            };
            check_steps(
                &written.branches[j].body,
                &source_branch.steps,
                &target_branch.steps,
                info,
                &inner,
                &options,
                name,
            )?;
        }
    }
    Ok(())
}
