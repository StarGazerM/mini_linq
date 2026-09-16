mod wiring {
    pub use ::mini_linq::{
        pull_set, collect_pipeline, graph_query as query, hash_materialized as choose_realization,
        result_indexes as choose_indexes,
    };
}
use wiring as mini_linq;

mini_linq::query! {
    struct Chain;
    relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(c:i32,d:i32);
    answer(a,d) :- R(a,b), S(b,c), T(c,d);
}

fn main() {
    let answer = Chain::run([(1, 2), (9, 8)], [(2, 3), (8, 7)], [(3, 4)]);
    assert_eq!(answer, vec![(1, 4)]);
    println!("{answer:?}");
}
