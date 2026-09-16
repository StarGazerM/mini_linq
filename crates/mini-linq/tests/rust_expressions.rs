mod materialized {
    mod wiring {
        #[allow(unused_imports)]
        pub use ::mini_linq::{
            collect_pipeline, graph_query, iterator_pipeline, optimized_query, prefix_query, pull,
            pull_set,
        };
        pub use ::mini_linq::{
            hash_materialized as choose_realization, result_indexes as choose_indexes,
        };
    }
    use mini_linq::optimized_query as query;
    use wiring as mini_linq;
    include!("support/rust_expressions.rs");
}

mod lazy {
    use mini_linq::mini_linq as query;
    include!("support/rust_expressions.rs");
}
