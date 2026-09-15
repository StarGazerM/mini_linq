//! Policy selects legal combinations; it never emits algebra.
use super::{JoinPolicy, JoinTree, rules::JoinState};

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
