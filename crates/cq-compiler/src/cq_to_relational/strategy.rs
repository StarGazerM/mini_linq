//! Policy selects legal combinations; it never emits algebra.
use super::{JoinPolicy, JoinTree, clause_rules::ClauseState, error, rules::JoinState};

/// Choose among C-READY clauses. Written order uses the first legal clause;
/// other policies prefer a ready guard/binding, then connected atoms, then width.
pub(super) fn choose_clause(state: &ClauseState<'_>, policy: JoinPolicy) -> syn::Result<usize> {
    state
        .ready_clauses()
        .map(|(clause, facts)| {
            let rank = if policy == JoinPolicy::Written {
                (false, false, 0, clause)
            } else if let Some(arity) = facts.atom_arity {
                let shared = facts.provided.intersection(state.heading()).count()
                    + facts.literal_arguments.len();
                let width = if matches!(policy, JoinPolicy::GreedyWidth | JoinPolicy::Auto) {
                    state.heading().len() + arity - shared
                } else {
                    0
                };
                (true, shared == 0, width, clause)
            } else {
                (false, false, 0, clause)
            };
            (rank, clause)
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, clause)| clause)
        .ok_or_else(|| error("no pending clause satisfies C-READY"))
}

pub(super) fn choose_pair(
    state: &JoinState<'_>,
    strategy: JoinPolicy,
    tree: Option<&JoinTree>,
) -> (usize, usize) {
    let frontier = state.frontier();
    if let Some(tree) = tree {
        let root = tree.root();
        let root_group = &frontier[&root];
        let right = *frontier
            .keys()
            .find(|&&i| {
                i != root && tree.parent[i].is_some_and(|p| root_group.coverage.contains(&p))
            })
            .unwrap();
        return (root, right);
    }
    let left = *frontier.first_key_value().unwrap().0;
    match strategy {
        JoinPolicy::Written => (left, *frontier.keys().nth(1).unwrap()),
        JoinPolicy::Connected => {
            let right = frontier
                .iter()
                .find(|(i, group)| {
                    **i != left && !frontier[&left].heading.is_disjoint(&group.heading)
                })
                .map(|(&i, _)| i)
                .unwrap_or_else(|| *frontier.keys().nth(1).unwrap());
            (left, right)
        }
        JoinPolicy::GreedyWidth | JoinPolicy::Auto => {
            // Minimize the actual next join's materialized arity. No statistics
            // or claims of optimal/global/fractional width are hidden here.
            frontier
                .iter()
                .flat_map(|(&a, l)| {
                    frontier
                        .iter()
                        .filter(move |(b, _)| **b > a)
                        .map(move |(&b, r)| (l.heading.union(&r.heading).count(), a, b))
                })
                .min()
                .map(|(_, a, b)| (a, b))
                .unwrap()
        }
    }
}
