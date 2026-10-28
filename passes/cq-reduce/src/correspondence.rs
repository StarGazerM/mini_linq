//! Reduction erasure and obligations, without the reducer's schedule builder.

use super::{Options, Reductions};
use cq_lang_vo::{self as vo, Reduction, Step};
use std::collections::{BTreeMap, BTreeSet};

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
    let effective = options.effective(source.valuation.join_branches.is_some(), false);
    let options = &effective;
    check_steps(
        &source.cq.query.body,
        &source.valuation.steps,
        &target.valuation.steps,
        &source_info,
        &super::scope_graph(
            &source.cq.query.body,
            &source.valuation.steps,
            &BTreeSet::new(),
        ),
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
    let tree = if options.semijoins == Reductions::Hypergraph && graph.acyclic {
        Some(graph.reduction_tree.clone())
    } else if options.semijoins == Reductions::Tree {
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
                        Step::Negation(_, _) => "relational negation".into(),
                        Step::Guard(_, _) => "guard clause".into(),
                        Step::Let(_, _) => "binding clause".into(),
                        Step::For(_, _) => "generator clause".into(),
                        Step::Reduce(r, _) => format!("reduction of clause #{}", r.clause()),
                    })
                    .unwrap_or_default()
            ),
        ));
    }
    if options.semijoins == Reductions::Masks {
        check_masks(body, source_steps, target_steps, source_info, &graph)?;
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
    let scheduled: &[Step] = if options.semijoins == Reductions::Masks {
        &[]
    } else {
        target_steps
    };
    for (position, step) in scheduled.iter().enumerate() {
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
            let inner =
                super::scope_graph(&written.branches[j].body, &source_branch.steps, &context);
            let options = options.effective(false, !inner.acyclic);
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

/// Formal rule CK-Masks: Check exact eligible edges, outer exclusion, filtered components, child dependencies and strongest sibling order independently of mask construction.
fn check_masks(
    body: &vo::syntax::CommaList<vo::cq::BodyItem>,
    source: &[Step],
    target: &[Step],
    source_info: &vo::Analysis,
    graph: &vo::cq::hypergraph::Analysis,
) -> syn::Result<()> {
    let fail = |message| syn::Error::new_spanned(body, message);
    let root = source.iter().find_map(|step| match step {
        Step::Atom(atom, _) => Some(atom.clause()),
        Step::Block(block, _) if graph.clauses.contains_key(&block.source.clause()) => {
            Some(block.source.clause())
        }
        _ => None,
    });
    let required: BTreeSet<_> = source_info.domain_requires.values().flatten().collect();
    let boundary = if required.is_empty() {
        source
            .iter()
            .position(|step| matches!(step, Step::Atom(..) | Step::Block(..)))
    } else {
        source_info
            .bound_before
            .iter()
            .position(|bound| required.iter().all(|name| bound.contains(name)))
    };
    let mut consumed = 0;
    let mut actual = BTreeMap::new();
    for (position, step) in target.iter().enumerate() {
        if let Step::Reduce(reduction, _) = step {
            if boundary.is_some_and(|boundary| consumed > boundary) {
                return Err(fail(
                    "Masks reductions must precede the stream or its first ready domain boundary",
                ));
            }
            let Reduction::With(_, support) = &reduction.reduction;
            let edge = (reduction.clause(), support.clause());
            if Some(edge.0) == root || Some(edge.1) == root {
                return Err(fail("Masks forbids the outer as target or support"));
            }
            if actual.insert(edge, position).is_some() {
                return Err(fail("Masks forbids duplicate reduction edges"));
            }
        } else {
            consumed += 1;
        }
    }
    let Some(root) = root else {
        return if actual.is_empty() {
            Ok(())
        } else {
            Err(fail("Masks requires a scheduled outer occurrence"))
        };
    };
    let mut adjacent: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(node, parent) in &graph.reduction_tree {
        if let Some(parent) = parent {
            adjacent.entry(node).or_default().push(parent);
            adjacent.entry(parent).or_default().push(node);
        }
    }
    // Distance labels orient the undirected tree without using the reducer's
    // rooting or traversal helpers.
    let mut distance = BTreeMap::from([(root, 0usize)]);
    let mut queue = vec![root];
    let mut cursor = 0;
    while cursor < queue.len() {
        let node = queue[cursor];
        cursor += 1;
        for &neighbor in adjacent.get(&node).into_iter().flatten() {
            if !distance.contains_key(&neighbor) {
                distance.insert(neighbor, distance[&node] + 1);
                queue.push(neighbor);
            }
        }
    }
    let cuts: BTreeSet<_> = graph
        .clauses
        .keys()
        .copied()
        .filter(|&atom| atom != root && body[atom].atom().is_some())
        .flat_map(|atom| {
            graph
                .clauses
                .keys()
                .copied()
                .filter(move |&block| {
                    block != root
                        && matches!(&body[block], vo::cq::BodyItem::Block(_))
                        && !graph.shared(atom, block).is_empty()
                })
                .map(move |block| (atom, block))
        })
        .collect();
    let mut local = vo::cq::hypergraph::filter_ranks(body, graph);
    for &(atom, _) in &cuts {
        local.entry(atom).and_modify(|rank| *rank = (*rank).min(2));
    }
    let mut required = cuts.clone();
    let mut ranks = BTreeMap::new();
    for (&child, &depth) in &distance {
        if child == root {
            continue;
        }
        let parent = adjacent[&child]
            .iter()
            .copied()
            .find(|node| distance.get(node) == Some(&(depth - 1)))
            .expect("reachable tree node has a parent");
        // Inspect the support component after removing its parent edge. This
        // is a declarative filtered-subtree test, not a schedule traversal.
        let mut pending = vec![(child, parent)];
        let mut rank = 3;
        while let Some((node, previous)) = pending.pop() {
            rank = rank.min(local.get(&node).copied().unwrap_or(3));
            for &neighbor in adjacent.get(&node).into_iter().flatten() {
                if neighbor != previous {
                    pending.push((neighbor, node));
                }
            }
        }
        if parent != root && body[parent].atom().is_some() && rank < 3 {
            required.insert((parent, child));
            if !cuts.contains(&(parent, child)) {
                ranks.insert((parent, child), rank);
            }
        }
    }
    if actual.keys().any(|edge| !required.contains(edge)) {
        return Err(fail(
            "Masks edge lacks an inner filtered-child or exact block-cut premise",
        ));
    }
    if required.iter().any(|edge| !actual.contains_key(edge)) {
        return Err(fail(
            "Masks is missing a filtered-child edge or exact block cut",
        ));
    }
    for (&(parent, child), &position) in &actual {
        for (&(target, support), &earlier) in &actual {
            if target == child && earlier >= position {
                return Err(fail(
                    "Masks support must include all child reductions before use",
                ));
            }
            if cuts.contains(&(target, support))
                && !cuts.contains(&(parent, child))
                && earlier >= position
            {
                return Err(fail("Masks block cuts must precede tree reductions"));
            }
        }
        if let Some(&rank) = ranks.get(&(parent, child)) {
            for (&(other_parent, sibling), &other_rank) in &ranks {
                if parent == other_parent
                    && (other_rank, sibling) < (rank, child)
                    && actual[&(other_parent, sibling)] >= position
                {
                    return Err(fail(
                        "Masks siblings must run strongest first, then written order",
                    ));
                }
            }
        }
    }
    Ok(())
}
