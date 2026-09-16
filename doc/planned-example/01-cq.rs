mod wiring {
    #[allow(unused_imports)]
    pub use ::mini_linq::{
        optimized_query, graph_query, prefix_query, pull_set, collect_pipeline, pull,
        iterator_pipeline,
    };
    pub use ::mini_linq::{
        result_indexes as choose_indexes, hash_materialized as choose_realization,
    };
}
use wiring as mini_linq;
mini_linq::optimized_query! {
    pub struct PlannedExample; relation R(a : i32, b : i32); relation S(b : i32, c :
    i32); answer(a) : - R(a, b), S(b, c), if (c > 0);
}
