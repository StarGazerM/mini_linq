//! A caller-written host composes RA → IX → Physical → Rust[Access].
#![recursion_limit = "512"]

use std::collections::{BTreeMap, HashMap, HashSet};

// This policy supplies independent choices for each exact relation version,
// index, and RA definition. Provider paths are ordinary external macro paths.
macro_rules! explicit_policy {
    ($($ix:tt)*) => {
        ::mini_linq::given_physical_bindings! {
            $($ix)*
            physical {
                relations {
                    L = ::cq_example_providers::slice_rows;
                    R = ::cq_example_providers::hash_relation;
                    S = ::cq_example_providers::sorted_relation;
                    l = ::cq_example_providers::slice_rows;
                    r = ::cq_example_providers::hash_relation;
                    s = ::cq_example_providers::sorted_relation;
                    j0 = ::cq_example_providers::borrowed_fields;
                    j1 = ::cq_example_providers::borrowed_fields;
                }
                indexes {
                    i0 = ::cq_example_providers::hash_relation;
                    i1 = ::cq_example_providers::sorted_relation;
                }
                lowerings {
                    l = ::cq_example_providers::rename;
                    r = ::cq_example_providers::rename;
                    s = ::cq_example_providers::rename;
                    j0 = ::cq_example_providers::indexed_join;
                    j1 = ::cq_example_providers::indexed_join;
                }
            }
        }
    };
}
use explicit_policy as choose_physical;

mod wiring {
    pub(crate) use super::choose_physical;
    pub(crate) use ::mini_linq::{access_to_set_loops as access, realize_physical};
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
