//! A caller-written host composes RA → IX → Physical → Structures → Rust.
//!
//! The policy is query-independent: it states facts about each host value
//! and nothing about this query's results. `R` and `S` are host maps keyed
//! on their join columns, so the compiler probes them in place and builds no
//! structure of its own; `L` is only scanned.
#![recursion_limit = "512"]

use std::collections::{BTreeMap, HashMap, HashSet};

macro_rules! policy {
    ($($ix:tt)*) => {
        ::mini_linq::lower_physical! {
            $($ix)*
            policy {
                L;
                R keyed [a];
                S keyed [b];
            }
        }
    };
}
use policy as physical_policy;
mod wiring {
    pub(crate) use super::physical_policy;
    pub(crate) use ::mini_linq::{select_structures, structures_to_rust};
    pub(crate) use cq_state_providers::hash_distinct as distinct_state;
}
use wiring as mini_linq;

// The host owns its signature; the query is an iterator of borrowed answers.
// Host maps hold tuples: keys over the key columns, payloads over the rest.
#[allow(non_snake_case)]
fn query<'a>(
    L: &'a [(i32,)],
    R: &'a HashMap<(i32,), Vec<(i32,)>>,
    S: &'a BTreeMap<(i32,), Vec<(i32,)>>,
) -> HashSet<(&'a i32, &'a i32, &'a i32)> {
    ::core::iter::Iterator::collect(::mini_linq::index_demands! {
        relational {
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
            l = rename L {a -> a};
            r = rename R {a -> a, b -> b};
            s = rename S {b -> b, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            output j1 as answer(a, b, c);
        }
    })
}

fn main() {
    let left = [(1,), (1,), (2,), (9,)];
    let middle = HashMap::from([((1,), vec![(10,), (11,)]), ((2,), vec![(20,)])]);
    let right = BTreeMap::from([
        ((10,), vec![(100,), (101,)]),
        ((11,), vec![]),
        ((20,), vec![(200,)]),
    ]);
    let borrowed = query(&left, &middle, &right);
    // Copy only for deterministic display; the query itself returns references.
    let mut rows: Vec<_> = borrowed.into_iter().map(|(a, b, c)| (*a, *b, *c)).collect();
    rows.sort_unstable();
    assert_eq!(rows, [(1, 10, 100), (1, 10, 101), (2, 20, 200)]);
    println!("Borrowed heterogeneous join: {rows:?}");
}
