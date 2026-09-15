//! End-to-end tests compose the two independent assignment boundaries.
mod wiring {
    pub use ::mini_linq::{
        collect_access, collect_pipeline, graph_query as query, materialized as index_requirements,
        result_indexes as relational_plan,
    };
}
use std::collections::BTreeSet;
use wiring as mini_linq;

mini_linq::query! {
    struct Chain;
    relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(c:i32,d:i32);
    answer(a,d) :- R(a,b), S(b,c), T(c,d);
}

mini_linq::query! {
    struct Disconnected;
    relation R(a:i32); relation S(b:i32);
    answer(a,b) :- R(a), S(b);
}

#[test]
fn reduced_chain_matches_independent_join_enumeration() {
    let rows = [(0, 0), (0, 1), (1, 0), (1, 1)];
    let subset = |mask: usize| {
        rows.into_iter()
            .enumerate()
            .filter_map(move |(i, row)| (mask & (1 << i) != 0).then_some(row))
            .collect::<Vec<_>>()
    };
    for r_mask in 0..16 {
        for s_mask in 0..16 {
            for t_mask in 0..16 {
                let (r, s, t) = (subset(r_mask), subset(s_mask), subset(t_mask));
                let mut expected = BTreeSet::new();
                for &(a, b) in &r {
                    for &(sb, c) in &s {
                        for &(tc, d) in &t {
                            if b == sb && c == tc {
                                expected.insert((a, d));
                            }
                        }
                    }
                }
                assert_eq!(
                    Chain::run(r, s, t).into_iter().collect::<BTreeSet<_>>(),
                    expected
                );
            }
        }
    }
}

#[test]
fn empty_separator_propagates_empty_components() {
    assert_eq!(
        Disconnected::run([(1,), (1,)], [(2,), (3,)]),
        vec![(1, 2), (1, 3)]
    );
    assert!(Disconnected::run([(1,)], []).is_empty());
    assert!(Disconnected::run([], [(2,)]).is_empty());
}
