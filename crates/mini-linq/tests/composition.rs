//! The same CQ runs through independent macro choices at three boundaries.
use std::collections::HashSet;
static SELECTED_ACCESSES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
macro_rules! selected_pull {
    ($($tokens:tt)*) => {{
        crate::SELECTED_ACCESSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ::mini_linq::access_to_distinct_iter! { $($tokens)* }
    }};
}

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
                pub use ::mini_linq::$indexes as choose_index_policy;
                pub use ::mini_linq::$physical as choose_relation_lowering;
                route!(@access $host);
            }
            use wiring as mini_linq;
            route!(@host $host);
        }
    };
    (@access standalone) => {
        pub use ::mini_linq::pipeline_to_distinct_iter;
        pub(super) use selected_pull as access_to_distinct_iter;
    };
    (@access callable) => {
        pub use ::mini_linq::{access_to_set, pipeline_to_set};
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

mod defaults {
    mod wiring {
        pub use ::mini_linq::{
            base_index_demands, choose_index_policy, choose_relation_lowering,
            cq_to_ra_written_order, lower_base_hash, mini_linq, pipeline_to_distinct_iter,
        };
        pub(super) use selected_pull as access_to_distinct_iter;
    }
    use wiring as mini_linq;
    source!();
    pub(super) fn run(left: &[(i32, i32)], right: &[(i32, i32)]) -> super::HashSet<(i32, i32)> {
        Query::run(left.iter().copied(), right.iter().copied())
            .into_iter()
            .collect()
    }
}
route!(
    input,
    cq_to_ra_written_order,
    base_index_demands,
    lower_base_hash,
    standalone
);
route!(
    lazy,
    cq_to_ra_written_order,
    result_index_demands,
    lower_prefix_hash,
    standalone
);
route!(
    materialized,
    cq_to_ra_written_order,
    result_index_demands,
    materialized_hash_callable,
    callable
);
route!(
    prefix,
    cq_to_ra_clause_prefix,
    result_index_demands,
    materialized_hash_callable,
    callable
);
route!(
    graph,
    cq_to_ra_hypergraph,
    result_index_demands,
    materialized_hash_callable,
    callable
);

#[test]
fn explicit_routes_preserve_the_same_query_on_every_small_database() {
    SELECTED_ACCESSES.store(0, std::sync::atomic::Ordering::Relaxed);
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
                defaults::run,
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
    assert_eq!(
        SELECTED_ACCESSES.load(std::sync::atomic::Ordering::Relaxed),
        1536,
        "all three standalone routes must use the selected Access macro on each run"
    );
}
