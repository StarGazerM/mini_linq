mod wiring {
    pub use ::mini_linq::{
        access_to_set, cq_to_ra_hypergraph as query,
        materialized_hash_program as choose_relation_lowering, pipeline_to_set,
        result_index_demands as choose_index_policy,
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
