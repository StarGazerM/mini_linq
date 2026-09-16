//! This compiler includes only the graph/result-index/materialized/pull route.
//! The alias selects each next stage; Cargo excludes the other implementations.
mod wiring {
    pub use ::mini_linq::{
        pull_set, collect_pipeline, graph_query as mini_linq,
        hash_materialized_query as choose_realization, result_indexes as choose_indexes,
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
