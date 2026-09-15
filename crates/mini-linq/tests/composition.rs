//! The same CQ runs through independent macro choices at three boundaries.
use std::collections::HashSet;

macro_rules! source {
    () => {
        mini_linq::mini_linq! {
            pub struct Query;
            relation R(a:i32,b:i32); relation S(b:i32,c:i32);
            answer(a,c) :- R(a,b), S(b,c);
        }
    };
}

macro_rules! route {
    ($name:ident, $logical:ident, $indexes:ident, $physical:ident, $host:ident) => {
        mod $name {
            mod wiring {
                pub use ::mini_linq::$logical as mini_linq;
                pub use ::mini_linq::$indexes as relational_plan;
                pub use ::mini_linq::$physical as index_requirements;
                route!(@access $host);
            }
            use wiring as mini_linq;
            route!(@host $host);
        }
    };
    (@access standalone) => {
        pub use ::mini_linq::iterator_pipeline;
    };
    (@access callable) => {
        pub use ::mini_linq::{collect_access, collect_pipeline};
    };
    (@host standalone) => {
        source!();
        pub(super) fn run(left: &[(i32,i32)], right: &[(i32,i32)]) -> super::HashSet<(i32,i32)> {
            Query::run(left.iter().copied(), right.iter().copied()).into_iter().collect()
        }
    };
    (@host callable) => {
        pub(super) fn run(left: &[(i32,i32)], right: &[(i32,i32)]) -> super::HashSet<(i32,i32)> {
            let query = source!();
            query(left.iter().copied(), right.iter().copied())
        }
    };
}

route!(
    input,
    canonical_query,
    input_indexes,
    input_indexed,
    standalone
);
route!(
    lazy,
    canonical_query,
    result_indexes,
    lazy_indexed,
    standalone
);
route!(
    materialized,
    canonical_query,
    result_indexes,
    materialized_query,
    callable
);
route!(
    prefix,
    prefix_query,
    result_indexes,
    materialized_query,
    callable
);
route!(
    graph,
    graph_query,
    result_indexes,
    materialized_query,
    callable
);

#[test]
fn explicit_routes_preserve_the_same_query_on_every_small_database() {
    let universe = [(0, 0), (0, 1), (1, 0), (1, 1)];
    let rows = |mask: usize| {
        universe
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, r)| *r)
            .collect::<Vec<_>>()
    };
    for left in 0..16 {
        for right in 0..16 {
            let left = rows(left);
            let right = rows(right);
            let expected: HashSet<_> = left
                .iter()
                .flat_map(|&(a, b)| {
                    right
                        .iter()
                        .filter_map(move |&(key, c)| (b == key).then_some((a, c)))
                })
                .collect();
            for execute in [
                input::run,
                lazy::run,
                materialized::run,
                prefix::run,
                graph::run,
            ] {
                assert_eq!(execute(&left, &right), expected, "{left:?} join {right:?}");
                let duplicated: Vec<_> = left.iter().chain(&left).copied().collect();
                assert_eq!(execute(&duplicated, &right), expected);
            }
        }
    }
}
