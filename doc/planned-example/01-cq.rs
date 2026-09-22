mod wiring {
    #[allow(unused_imports)]
    pub use ::mini_linq::{
        choose_graph_or_prefix, cq_to_ra_hypergraph, cq_to_ra_clause_prefix,
        access_to_set, pipeline_to_set, access_to_distinct_iter,
        pipeline_to_distinct_iter,
    };
    pub use ::mini_linq::{
        result_index_demands as choose_index_policy,
        materialized_hash_program as choose_relation_lowering,
    };
}
use wiring as mini_linq;
mini_linq::choose_graph_or_prefix! {
    pub struct PlannedExample; relation R(a : i32, b : i32); relation S(b : i32, c :
    i32); answer(a) : - R(a, b), S(b, c), if (c > 0);
}
