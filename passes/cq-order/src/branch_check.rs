//! Declarative obligations on recorded dimension groups, not selector output.
use cq_lang_cq::{self as cq, BodyItem};
use cq_lang_vo::JoinBranches;
use std::collections::{BTreeMap, BTreeSet};

/// Formal rule CK-OrderBranches: Validate widest-spine exclusion, pinned probes, legal ear proofs and saturated recorded groups without invoking branch selection.
pub(super) fn check(
    source: &cq::Program,
    recorded: Option<&JoinBranches>,
    enabled: bool,
) -> syn::Result<()> {
    let fail = |message| syn::Error::new_spanned(&source.query.body, message);
    let flat = source.query.body.iter().all(|item| {
        matches!(
            item,
            BodyItem::Positive { .. } | BodyItem::Equality(_) | BodyItem::Constraint(_)
        )
    });
    if !enabled || !flat {
        return if recorded.is_none() {
            Ok(())
        } else {
            Err(fail(
                "dimension branches require a flat Filtered/Coarse scope",
            ))
        };
    }
    let graph = cq::hypergraph::Analysis::of(source);
    let headings: BTreeMap<_, BTreeSet<_>> = graph
        .clauses
        .iter()
        .map(|(&clause, columns)| (clause, columns.iter().cloned().collect()))
        .collect();
    let shared: BTreeSet<_> = graph
        .incidence
        .iter()
        .filter(|(_, occurrences)| occurrences.len() > 1)
        .map(|(column, _)| column.clone())
        .collect();
    let width = |clause: usize| headings[&clause].intersection(&shared).count();
    // Check the defining maximum premise directly against every competitor.
    let spine = headings.keys().copied().find(|&candidate| {
        width(candidate) >= 3
            && headings.keys().copied().all(|other| {
                width(other) < width(candidate)
                    || width(other) == width(candidate) && candidate <= other
            })
    });
    let Some(spine) = spine else {
        return if recorded.is_none() {
            Ok(())
        } else {
            Err(fail(
                "dimension branches require a spine with at least three shared columns",
            ))
        };
    };
    let fact = &headings[&spine];
    let filters = cq::hypergraph::filter_ranks(&source.query.body, &graph);
    let pinned = |clause: usize| {
        clause == spine
            || (filters[&clause] < 3
                && headings[&clause].intersection(fact).count() == 1
                && headings[&clause]
                    .difference(fact)
                    .all(|name| !shared.contains(name)))
    };
    // Unlisted occurrences are singleton groups. WF-JoinBranch has already
    // checked disjointness, full atoms, connectivity and exact interfaces.
    let mut owner: BTreeMap<_, _> = headings.keys().map(|&clause| (clause, clause)).collect();
    if let Some(recorded) = recorded {
        for branch in &recorded.branches {
            let members = branch.clauses();
            let first = *members.first().expect("WF-JoinBranch requires members");
            for clause in members {
                if pinned(clause) {
                    return Err(fail(
                        "a fact spine or filtered unary probe cannot belong to a dimension group",
                    ));
                }
                owner.insert(clause, first);
            }
        }
    }
    let mut proof: BTreeMap<_, BTreeSet<_>> = headings
        .keys()
        .copied()
        .filter(|&clause| !pinned(clause))
        .map(|clause| (clause, BTreeSet::from([clause])))
        .collect();
    loop {
        let columns: BTreeMap<_, BTreeSet<_>> = proof
            .iter()
            .map(|(&id, members)| {
                (
                    id,
                    members
                        .iter()
                        .flat_map(|member| headings[member].iter().cloned())
                        .collect(),
                )
            })
            .collect();
        let ear = |leaf: usize, parent: usize| {
            leaf != parent
                && !columns[&leaf].is_disjoint(&columns[&parent])
                && columns[&leaf]
                    .intersection(fact)
                    .all(|key| columns[&parent].contains(key))
                && columns
                    .iter()
                    .filter(|(id, _)| **id != leaf)
                    .all(|(_, external)| {
                        columns[&leaf]
                            .intersection(external)
                            .all(|key| columns[&parent].contains(key))
                    })
        };
        // Discharge only within a recorded group: no expected JoinBranches is
        // constructed. Contracting a proven ear preserves its external heading.
        let mut witness = None;
        for &leaf in proof.keys() {
            for &parent in proof.keys() {
                if owner[&leaf] != owner[&parent] || !ear(leaf, parent) {
                    continue;
                }
                let members = proof[&leaf].union(&proof[&parent]).copied().collect();
                let interface = cq_lang_vo::join_branches::interface(source, &members);
                let width = interface
                    .iter()
                    .map(|name| graph.canonical_names.get(name).unwrap_or(name))
                    .collect::<BTreeSet<_>>()
                    .len();
                let rank = (
                    width,
                    *proof[&parent].first().unwrap(),
                    *proof[&leaf].first().unwrap(),
                    leaf,
                    parent,
                );
                if witness.is_none_or(|best| rank < best) {
                    witness = Some(rank);
                }
            }
        }
        if let Some((_, _, _, leaf, parent)) = witness {
            let members = proof.remove(&leaf).unwrap();
            proof.get_mut(&parent).unwrap().extend(members);
            continue;
        }
        let mut discharged = BTreeSet::new();
        for &component in proof.keys() {
            if !discharged.insert(owner[&component]) {
                return Err(fail(
                    "recorded dimension group has no legal connected-ear proof",
                ));
            }
            if proof.keys().any(|&parent| ear(component, parent)) {
                return Err(fail("dimension groups omit a remaining admissible ear"));
            }
        }
        return Ok(());
    }
}
