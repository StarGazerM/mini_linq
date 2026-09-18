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
        const PLANNER: &str = "written-order";
        ::mini_linq::cq_to_ra_written_order! { $($source)* }
    };
}
macro_rules! prefix {
    ($($source:tt)*) => {
        const PLANNER: &str = "clause-prefix";
        ::mini_linq::cq_to_ra_clause_prefix! { $($source)* }
    };
}
macro_rules! marked_input_indexes {
    ($($source:tt)*) => {
        const INDEX_POLICY: &str = "input";
        ::mini_linq::base_index_demands! { $($source)* }
    };
}
macro_rules! marked_result_indexes {
    ($($source:tt)*) => {
        const INDEX_POLICY: &str = "result";
        ::mini_linq::result_index_demands! { $($source)* }
    };
}
macro_rules! requirements {
    ($($source:tt)*) => {
        const DISPATCHED: &str = "indexes";
        ::mini_linq::choose_relation_lowering! { $($source)* }
    };
}
macro_rules! input_backend {
    ($($source:tt)*) => {
        const REALIZATION: &str = "input";
        ::mini_linq::lower_base_hash! { $($source)* }
    };
}
macro_rules! lazy_backend {
    ($($source:tt)*) => {
        const REALIZATION: &str = "lazy";
        ::mini_linq::lower_prefix_hash! { $($source)* }
    };
}
macro_rules! materialized_backend {
    ($($source:tt)*) => {
        const REALIZATION: &str = "materialized";
        ::mini_linq::materialized_hash_program! { $($source)* }
    };
}

mod basic {
    #[allow(unused_imports)]
    mod wiring {
        pub use ::mini_linq::*;
        pub(super) use canonical as cq_to_ra_written_order;
        pub(super) use forbidden as cq_to_ra_clause_prefix;
        pub(super) use forbidden as result_index_demands;
        pub(super) use forbidden as lower_prefix_hash;
        pub(super) use forbidden as materialized_hash_program;
        pub(super) use input_backend as lower_base_hash;
        pub(super) use marked_input_indexes as base_index_demands;
        pub(super) use requirements as choose_relation_lowering;
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
            ("written-order", "input", "input")
        );
        assert_eq!(DISPATCHED, "indexes");
        assert_eq!(Query::run([(1, 2), (3, 4)], [(2, 5), (9, 8)]), vec![(1, 5)]);
    }
}

mod replaced_index_policy {
    #[allow(unused_imports)]
    mod wiring {
        pub use ::mini_linq::*;
        pub(super) use canonical as cq_to_ra_written_order;
        pub(super) use forbidden as lower_base_hash;
        pub(super) use forbidden as materialized_hash_program;
        pub(super) use lazy_backend as lower_prefix_hash;
        pub(super) use marked_result_indexes as base_index_demands;
        pub(super) use requirements as choose_relation_lowering;
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
            ("written-order", "result", "lazy")
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
        pub(super) use forbidden as cq_to_ra_written_order;
        pub(super) use forbidden as base_index_demands;
        pub(super) use forbidden as lower_base_hash;
        pub(super) use forbidden as materialized_hash_program;
        pub(super) use lazy_backend as lower_prefix_hash;
        pub(super) use marked_result_indexes as result_index_demands;
        pub(super) use prefix as cq_to_ra_clause_prefix;
        pub(super) use requirements as choose_relation_lowering;
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
            ("clause-prefix", "result", "lazy")
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
        pub(super) use forbidden as base_index_demands;
        pub(super) use forbidden as lower_base_hash;
        pub(super) use forbidden as lower_prefix_hash;
        pub(super) use marked_result_indexes as result_index_demands;
        pub(super) use materialized_backend as materialized_hash_program;
        pub(super) use requirements as choose_relation_lowering;
    }
    use wiring as mini_linq;
    mini_linq::cq_to_ra_hypergraph! {
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
        pub(super) use canonical as cq_to_ra_written_order;
        pub(super) use entry as mini_linq;
        pub(super) use input_backend as lower_base_hash;
        pub(super) use marked_input_indexes as base_index_demands;
        pub(super) use requirements as choose_relation_lowering;
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
            ("written-order", "input", "input")
        );
        assert_eq!(Query::run([(2,), (1,), (2,)]), vec![(1,), (2,)]);
        assert_eq!(module().unwrap().program.name, "Query");
        assert!(SOURCE.contains("answer"));
        assert!(RUST_PATH.ends_with("macro_routing.rs"));
    }
}
