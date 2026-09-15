//! This compiler includes only the graph/result-index/materialized/pull route.
//! The alias selects each next stage; Cargo excludes the other implementations.
mod wiring {
    pub use ::mini_linq::{
        collect_access, collect_pipeline, graph_query as mini_linq,
        materialized_query as index_requirements, result_indexes as relational_plan,
    };
}
use wiring as mini_linq;

fn main() {
    let query = mini_linq::mini_linq! {
        struct Neighbors;
        relation Edge(src: i32, dst: i32);
        relation Allowed(key: i32);
        answer(dst) :- Edge(src, dst), Allowed(src);
    };
    let rows = query([(1, 10), (2, 20), (1, 10)], [(1,)]);
    assert_eq!(rows, std::collections::HashSet::from([(10,)]));
    println!("Selected compiler returned {rows:?}");
}
