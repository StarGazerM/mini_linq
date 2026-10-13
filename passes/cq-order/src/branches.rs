//! Leaf-side dimension grouping without statistics or schema key assumptions.
use cq_lang_cq::{self as cq, BodyItem};
use cq_lang_vo::{JoinBranch, JoinBranches};
use std::collections::{BTreeMap, BTreeSet};

/// Formal rule S-JoinBranch: Pick the widest shared-key occurrence as the fact spine and absorb connected dimension ears through one external key set.
pub(super) fn choose(source: &cq::Program) -> Option<JoinBranches> {
    if source.query.body.iter().any(|item| !matches!(item, BodyItem::Positive { .. } | BodyItem::Equality(_) | BodyItem::Constraint(_))) { return None; }
    let graph = cq::hypergraph::Analysis::of(source);
    let atoms: BTreeMap<_, BTreeSet<_>> = graph.clauses.iter().map(|(&c, names)| (c, names.iter().cloned().collect())).collect();
    let shared: BTreeSet<_> = graph.incidence.iter().filter(|(_, occurrences)| occurrences.len() > 1).map(|(v, _)| v.clone()).collect();
    let (&fact, fact_columns) = atoms.iter().max_by_key(|(c, columns)| (columns.intersection(&shared).count(), std::cmp::Reverse(**c)))?;
    if fact_columns.intersection(&shared).count() < 3 { return None; }
    let filters = cq::hypergraph::filter_ranks(&source.query.body, &graph);
    // A locally filtered unary fact-key dimension remains an independent early
    // probe. This keeps selective Part-like filters out of a broad stock branch.
    let pinned: BTreeSet<_> = atoms.iter().filter(|(c, columns)| **c == fact ||
        (filters.get(c).copied().unwrap_or(3) < 3 && columns.intersection(fact_columns).count() == 1
         && columns.difference(fact_columns).all(|v| !shared.contains(v))))
        .map(|(&c, _)| c).collect();
    let mut groups: Vec<BTreeSet<usize>> = atoms.keys().filter(|c| !pinned.contains(c)).map(|&c| BTreeSet::from([c])).collect();
    loop {
        let headings: Vec<BTreeSet<_>> = groups.iter().map(|g| g.iter().flat_map(|c| atoms[c].iter().cloned()).collect()).collect();
        let mut candidates = Vec::new();
        for (leaf, columns) in headings.iter().enumerate() {
            for (parent, parent_columns) in headings.iter().enumerate() {
                if leaf == parent || columns.is_disjoint(parent_columns) { continue; }
                let leaf_keys: BTreeSet<_> = columns.intersection(fact_columns).cloned().collect();
                let parent_keys: BTreeSet<_> = parent_columns.intersection(fact_columns).cloned().collect();
                if !leaf_keys.is_subset(&parent_keys) { continue; }
                // Every connection away from the fact/pinned probes must leave
                // this ear through the same neighbor heading. A cycle spanning
                // Customer and Supplier therefore cannot be collapsed here.
                let external: BTreeSet<_> = headings.iter().enumerate().filter(|(i, _)| *i != leaf)
                    .flat_map(|(_, h)| columns.intersection(h).cloned()).collect();
                if !external.is_subset(parent_columns) { continue; }
                let union: BTreeSet<_> = groups[leaf].union(&groups[parent]).copied().collect();
                let interface = cq_lang_vo::join_branches::interface(source, &union);
                let canonical: BTreeSet<_> = interface.iter().map(|v| graph.canonical_names.get(v).unwrap_or(v)).collect();
                candidates.push((canonical.len(), *groups[parent].first().unwrap(), *groups[leaf].first().unwrap(), leaf, parent));
            }
        }
        let Some((_, _, _, leaf, parent)) = candidates.into_iter().min() else { break; };
        let members = groups[leaf].clone();
        groups[parent].extend(members);
        groups.remove(leaf);
    }
    let branches: Vec<_> = groups.into_iter().filter(|g| g.len() > 1).map(|members| JoinBranch {
        branch_token: Default::default(), bracket: Default::default(),
        clauses: members.iter().map(|c| syn::LitInt::new(&c.to_string(), proc_macro2::Span::call_site())).collect(),
        paren: Default::default(),
        columns: cq_lang_vo::join_branches::interface(source, &members).into_iter().map(|v| syn::Ident::new(&v, proc_macro2::Span::call_site())).collect(),
        semi: Default::default(),
    }).collect();
    (!branches.is_empty()).then_some(JoinBranches { branches_token: Default::default(), brace: Default::default(), branches })
}
