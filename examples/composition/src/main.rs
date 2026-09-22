//! A caller-written host composes RA → IX → Physical → Rust[Access].
//! Requires general physical selection. Do not supply a table of query-local
//! relation, index, or operator names to make this acceptance test pass.
#![recursion_limit = "512"]

use std::collections::{BTreeMap, HashMap, HashSet};

mod wiring {
    pub(crate) use ::mini_linq::{access_to_set_loops as access, realize_physical};
    #[allow(unused_imports)]
    pub(crate) use cq_example_providers::{
        hash_distinct as distinct_state, hash_set_sink as set_sink,
    };
}
use wiring as mini_linq;

// The host owns its signature and chooses the Access consumer explicitly.
// The physical realizer borrows these existing representations and indexes.
#[allow(non_snake_case)]
fn query<'a>(
    L: &'a [i32],
    R: &'a HashMap<i32, Vec<i32>>,
    S: &'a BTreeMap<i32, Vec<i32>>,
) -> HashSet<(&'a i32, &'a i32, &'a i32)> {
    ::mini_linq::index_demands! {
        struct Heterogeneous;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, b, c) :- L(a), R(a, b), S(b, c);
        relational {
            l = rename L {a -> a};
            r = rename R {a -> a, b -> b};
            s = rename S {b -> b, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            output j1 as answer(a, b, c);
        }
    }
}

fn main() {
    let left = [1, 1, 2, 9];
    let middle = HashMap::from([(1, vec![10, 11]), (2, vec![20])]);
    let right = BTreeMap::from([(10, vec![100, 101]), (11, vec![]), (20, vec![200])]);
    let borrowed = query(&left, &middle, &right);
    // Copy only for deterministic display; the query itself returns references.
    let mut rows: Vec<_> = borrowed.into_iter().map(|(a, b, c)| (*a, *b, *c)).collect();
    rows.sort_unstable();
    assert_eq!(rows, [(1, 10, 100), (1, 10, 101), (2, 20, 200)]);
    println!("Borrowed heterogeneous join: {rows:?}");
}
