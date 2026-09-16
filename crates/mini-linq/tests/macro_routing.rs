//! Replacement macros must run before the decisions they replace are committed.
#![cfg(feature = "defaults")]

#[allow(unused_macros)]
macro_rules! forbidden {
    ($($source:tt)*) => {
        compile_error!("an unselected compilation route expanded");
    };
}
macro_rules! canonical {
    ($($source:tt)*) => {
        const PLANNER: &str = "canonical";
        ::mini_linq::canonical_query! { $($source)* }
    };
}
macro_rules! prefix {
    ($($source:tt)*) => {
        const PLANNER: &str = "prefix";
        ::mini_linq::prefix_query! { $($source)* }
    };
}
macro_rules! marked_input_indexes {
    ($($source:tt)*) => {
        const INDEX_POLICY: &str = "input";
        ::mini_linq::input_indexes! { $($source)* }
    };
}
macro_rules! marked_result_indexes {
    ($($source:tt)*) => {
        const INDEX_POLICY: &str = "result";
        ::mini_linq::result_indexes! { $($source)* }
    };
}
macro_rules! requirements {
    ($($source:tt)*) => {
        const DISPATCHED: &str = "indexes";
        ::mini_linq::choose_realization! { $($source)* }
    };
}
macro_rules! input_backend {
    ($($source:tt)*) => {
        const REALIZATION: &str = "input";
        ::mini_linq::hash_inputs! { $($source)* }
    };
}
macro_rules! lazy_backend {
    ($($source:tt)*) => {
        const REALIZATION: &str = "lazy";
        ::mini_linq::hash_prefix! { $($source)* }
    };
}
macro_rules! materialized_backend {
    ($($source:tt)*) => {
        const REALIZATION: &str = "materialized";
        ::mini_linq::hash_materialized! { $($source)* }
    };
}

mod basic {
    #[allow(unused_imports)]
    mod wiring {
        pub use ::mini_linq::*;
        pub(super) use canonical as canonical_query;
        pub(super) use forbidden as prefix_query;
        pub(super) use forbidden as result_indexes;
        pub(super) use forbidden as hash_prefix;
        pub(super) use forbidden as hash_materialized;
        pub(super) use input_backend as hash_inputs;
        pub(super) use marked_input_indexes as input_indexes;
        pub(super) use requirements as choose_realization;
    }
    use wiring as mini_linq;
    mini_linq::mini_linq! {
        struct Query;
        relation R(a:i32,b:i32); relation S(a:i32,b:i32);
        answer(x,z) :- R(x,y), S(y,z);
    }

    #[test]
    fn all_default_boundaries_are_caller_resolved() {
        assert_eq!(
            (PLANNER, INDEX_POLICY, REALIZATION),
            ("canonical", "input", "input")
        );
        assert_eq!(DISPATCHED, "indexes");
        assert_eq!(Query::run([(1, 2), (3, 4)], [(2, 5), (9, 8)]), vec![(1, 5)]);
    }
}

mod replaced_index_policy {
    #[allow(unused_imports)]
    mod wiring {
        pub use ::mini_linq::*;
        pub(super) use canonical as canonical_query;
        pub(super) use forbidden as hash_inputs;
        pub(super) use forbidden as hash_materialized;
        pub(super) use lazy_backend as hash_prefix;
        pub(super) use marked_result_indexes as input_indexes;
        pub(super) use requirements as choose_realization;
    }
    use wiring as mini_linq;
    mini_linq::mini_linq! {
        struct Query;
        relation R(a:i32,b:i32); relation S(a:i32,b:i32);
        answer(x,z) :- R(x,y), S(y,z);
    }

    #[test]
    fn realization_uses_actual_indexes_after_a_policy_replacement() {
        assert_eq!(
            (PLANNER, INDEX_POLICY, REALIZATION),
            ("canonical", "result", "lazy")
        );
        assert_eq!(DISPATCHED, "indexes");
        assert_eq!(Query::run([(1, 2), (3, 4)], [(2, 5), (9, 8)]), vec![(1, 5)]);
    }
}

mod expressions {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn observe(_: i32) -> bool {
        CALLS.fetch_add(1, Ordering::Relaxed);
        true
    }
    #[allow(unused_imports)]
    mod wiring {
        pub use ::mini_linq::*;
        pub(super) use forbidden as canonical_query;
        pub(super) use forbidden as input_indexes;
        pub(super) use forbidden as hash_inputs;
        pub(super) use forbidden as hash_materialized;
        pub(super) use lazy_backend as hash_prefix;
        pub(super) use marked_result_indexes as result_indexes;
        pub(super) use prefix as prefix_query;
        pub(super) use requirements as choose_realization;
    }
    use wiring as mini_linq;
    mini_linq::mini_linq! {
        struct Query;
        relation R(a:i32);
        answer(x) :- R(x), if observe(*x);
    }

    #[test]
    fn expression_route_honors_aliases_without_making_evaluation_eager() {
        assert_eq!(
            (PLANNER, INDEX_POLICY, REALIZATION),
            ("prefix", "result", "lazy")
        );
        assert_eq!(DISPATCHED, "indexes");
        CALLS.store(0, Ordering::Relaxed);
        let storage = Query::build([(1,), (2,), (3,)]);
        assert_eq!(CALLS.load(Ordering::Relaxed), 0);
        assert!(storage.query().next().is_some());
        assert_eq!(CALLS.load(Ordering::Relaxed), 1);
        assert_eq!(storage.materialize(), vec![(1,), (2,), (3,)]);
        assert_eq!(CALLS.load(Ordering::Relaxed), 4);
    }
}

mod graph {
    #[allow(unused_imports)]
    mod wiring {
        pub use ::mini_linq::*;
        pub(super) use forbidden as input_indexes;
        pub(super) use forbidden as hash_inputs;
        pub(super) use forbidden as hash_prefix;
        pub(super) use marked_result_indexes as result_indexes;
        pub(super) use materialized_backend as hash_materialized;
        pub(super) use requirements as choose_realization;
    }
    use wiring as mini_linq;
    mini_linq::graph_query! {
        struct Query;
        relation R(a:i32,b:i32); relation S(a:i32,b:i32);
        answer(x,z) :- R(x,y), S(y,z);
    }

    #[test]
    fn graph_plan_selects_a_realization_that_supports_semijoins() {
        assert_eq!((INDEX_POLICY, REALIZATION), ("result", "materialized"));
        assert_eq!(DISPATCHED, "indexes");
        assert_eq!(Query::run([(1, 2), (3, 4)], [(2, 5), (9, 8)]), vec![(1, 5)]);
    }
}

#[cfg(feature = "compiled-workloads")]
mod workload {
    macro_rules! entry {
        ($($source:tt)*) => {
            const ENTERED: &str = "query";
            ::mini_linq::mini_linq! { $($source)* }
        };
    }
    #[allow(unused_imports)]
    mod wiring {
        pub use ::mini_linq::*;
        pub(super) use canonical as canonical_query;
        pub(super) use entry as mini_linq;
        pub(super) use input_backend as hash_inputs;
        pub(super) use marked_input_indexes as input_indexes;
        pub(super) use requirements as choose_realization;
    }
    use wiring as mini_linq;
    mini_linq::workload_query! {
        struct Query;
        relation R(a:i32);
        answer(x) :- R(x);
    }

    #[test]
    fn catalog_compilation_honors_the_same_route_as_an_ordinary_query() {
        assert_eq!((ENTERED, DISPATCHED), ("query", "indexes"));
        assert_eq!(
            (PLANNER, INDEX_POLICY, REALIZATION),
            ("canonical", "input", "input")
        );
        assert_eq!(Query::run([(2,), (1,), (2,)]), vec![(1,), (2,)]);
        assert_eq!(module().unwrap().program.name, "Query");
        assert!(SOURCE.contains("answer"));
        assert!(RUST_PATH.ends_with("macro_routing.rs"));
    }
}
