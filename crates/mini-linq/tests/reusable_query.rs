//! Macro aliases select the host while retaining the existing CQ/RA/IX stages.
use std::collections::HashSet;

mod callable {
    mod wiring {
        #[allow(unused_imports)]
        pub use ::mini_linq::{
            access_to_distinct_iter, access_to_set, choose_graph_or_prefix, cq_to_ra_clause_prefix,
            cq_to_ra_hypergraph, pipeline_to_distinct_iter, pipeline_to_set,
        };
        pub use ::mini_linq::{
            materialized_hash_callable as choose_relation_lowering,
            result_index_demands as choose_index_policy,
        };
    }
    use wiring as mini_linq;

    pub(super) fn query(
        left: &[(String, String)],
        right: &[(String, String)],
    ) -> HashSet<(String, String)> {
        let query = mini_linq::choose_graph_or_prefix! {
            struct CallerNamed;
            relation R(a:String,b:String); relation S(b:String,c:String);
            answer(a,c) :- R(a,b), S(b,c);
        };
        query(left.iter().cloned(), right.iter().cloned())
    }
    use super::HashSet;
}

mod standalone {
    mod wiring {
        #[allow(unused_imports)]
        pub use ::mini_linq::{
            access_to_distinct_iter, access_to_set, choose_graph_or_prefix, cq_to_ra_clause_prefix,
            cq_to_ra_hypergraph, pipeline_to_distinct_iter, pipeline_to_set,
        };
        pub use ::mini_linq::{
            materialized_hash_program as choose_relation_lowering,
            result_index_demands as choose_index_policy,
        };
    }
    use wiring as mini_linq;
    mini_linq::choose_graph_or_prefix! {
        pub struct CallerNamed;
        relation R(a:String,b:String); relation S(b:String,c:String);
        answer(a,c) :- R(a,b), S(b,c);
    }
}

#[test]
fn replacing_only_the_host_alias_allows_repeated_borrowed_evaluation() {
    let left = [
        ("a".to_owned(), "x".to_owned()),
        ("a".to_owned(), "x".to_owned()),
    ];
    let mut full = vec![("x".to_owned(), "b".to_owned())];
    let delta = [("x".to_owned(), "c".to_owned())];
    let initial = callable::query(&left, &full);
    let additions = callable::query(&left, &delta);
    full.extend(delta.iter().cloned());
    let all = callable::query(&left, &full);
    assert_eq!(all, initial.union(&additions).cloned().collect());
    assert_eq!(
        all,
        standalone::CallerNamed::run(left.iter().cloned(), full.iter().cloned())
            .into_iter()
            .collect()
    );
    assert_eq!(
        all,
        HashSet::from([("a".into(), "b".into()), ("a".into(), "c".into())])
    );
    assert!(callable::query(&left, &[]).is_empty());
    assert_eq!(left.len(), 2);
}

mod host_names {
    fn __ml_query(value: &i32) -> bool {
        *value > 0
    }
    fn __ml_query_(value: &i32) -> bool {
        *value < 3
    }
    macro_rules! allowed {
        ($value:expr) => {
            __ml_query($value) && __ml_query_($value)
        };
    }

    #[test]
    fn callable_preserves_direct_and_opaque_host_function_resolution() {
        let direct = mini_linq::materialized_hash_callable! {
            struct Direct;
            relation R(a:i32);
            answer(x) :- R(x), if (__ml_query(x) && __ml_query_(x));
            relational {
                r = rename R {a->x};
                s = select r if (__ml_query(x) && __ml_query_(x));
                output s as answer(x);
            }
            indexes {}
        };
        let opaque = mini_linq::materialized_hash_callable! {
            struct Opaque;
            relation R(a:i32);
            answer(x) :- R(x), if (allowed!(x));
            relational {
                r = rename R {a->x};
                s = select r if (allowed!(x));
                output s as answer(x);
            }
            indexes {}
        };
        let rows = [(-1,), (1,), (2,), (4,)];
        for _ in 0..2 {
            let expected = std::collections::HashSet::from([(1,), (2,)]);
            assert_eq!(direct(rows.iter().cloned()), expected);
            assert_eq!(opaque(rows.iter().cloned()), expected);
        }
    }
}

#[test]
fn callable_uses_native_local_capture_semantics() {
    let threshold = 1;
    let query = mini_linq::materialized_hash_callable! {
        struct Captured;
        relation R(a:i32);
        answer(x) :- R(x), if (*x > threshold);
        relational {
            r = rename R {a->x};
            s = select r if (*x > threshold);
            output s as answer(x);
        }
        indexes {}
    };
    let first = [(0,), (1,), (2,)];
    let second = [(1,), (3,)];
    assert_eq!(query(first.iter().cloned()), HashSet::from([(2,)]));
    assert_eq!(query(second.iter().cloned()), HashSet::from([(3,)]));
    assert_eq!(threshold, 1);
}
