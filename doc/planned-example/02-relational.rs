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
mini_linq::choose_indexes! {
    pub struct PlannedExample; relation R(a : i32, b : i32); relation S(b : i32, c :
    i32); answer(a) : - R(a, b), S(b, c), if (c > 0); relational { r0 = rename R { a - >
    a, b - > b }; r1 = rename S { b - > b, c - > c }; r2 = select r1 if (c > 0i32); r3 =
    semijoin r2 with r0; r4 = semijoin r0 with r3; r5 = project r3 keep { b }; r6 =
    natural_join r5 with r4; r7 = project r6 keep { a }; output r7 as answer(a); }
}
