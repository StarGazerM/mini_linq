//! Policy selects legal combinations; it never emits algebra.
use super::{JoinPolicy, JoinTree, rules::Kernel};

pub(super) fn choose_pair(
    state: &Kernel<'_>,
    strategy: JoinPolicy,
    tree: Option<&JoinTree>,
) -> (usize, usize) {
    let f = state.frontier();
    if let Some(tree) = tree {
        let root = tree.root();
        let g = &f[&root];
        let right = *f
            .keys()
            .find(|&&i| i != root && tree.parent[i].is_some_and(|p| g.coverage.contains(&p)))
            .unwrap();
        return (root, right);
    }
    let left = *f.first_key_value().unwrap().0;
    match strategy {
        JoinPolicy::Written => (left, *f.keys().nth(1).unwrap()),
        JoinPolicy::Connected => {
            let right = f
                .iter()
                .find(|(i, g)| **i != left && !f[&left].heading.is_disjoint(&g.heading))
                .map(|(&i, _)| i)
                .unwrap_or_else(|| *f.keys().nth(1).unwrap());
            (left, right)
        }
        JoinPolicy::GreedyWidth | JoinPolicy::Auto => {
            // Minimize the actual next join's materialized arity. No statistics
            // or claims of optimal/global/fractional width are hidden here.
            f.iter()
                .flat_map(|(&a, l)| {
                    f.iter()
                        .filter(move |(b, _)| **b > a)
                        .map(move |(&b, r)| (l.heading.union(&r.heading).count(), a, b))
                })
                .min()
                .map(|(_, a, b)| (a, b))
                .unwrap()
        }
    }
}
