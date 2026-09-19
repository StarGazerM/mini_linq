//! Real macro composition through the approved IX and Physical boundaries.
//! The final capture deliberately stops before provider realization.
use cq_ir::{index_requirements as ix, physical};
use quote::ToTokens;

macro_rules! explicit_policy {
    ($($ix:tt)*) => {
        const ACTUAL_IX: &str = stringify!($($ix)*);
        ::mini_linq::given_physical_bindings! {
            $($ix)*
            physical {
                relations {
                    L = external::slice_rows;
                    R = external::hash_relation;
                    S = external::sorted_relation;
                    l = external::slice_rows;
                    r = external::hash_relation;
                    s = external::sorted_relation;
                    j0 = external::borrowed_pairs;
                    j1 = external::borrowed_triples;
                }
                indexes {
                    i0 = external::hash_relation;
                    i1 = external::sorted_relation;
                }
                lowerings {
                    l = external::rename;
                    r = external::rename;
                    s = external::rename;
                    j0 = external::indexed_join;
                    j1 = external::indexed_join;
                }
            }
        }
    };
}
macro_rules! capture_physical {
    ($($physical:tt)*) => {
        const ACTUAL_PHYSICAL: &str = stringify!($($physical)*);
    };
}
use capture_physical as realize_physical;
use explicit_policy as choose_physical;
mod wiring {
    pub(crate) use super::{choose_physical, realize_physical};
}
use wiring as mini_linq;

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

#[test]
fn actual_macro_boundaries_equal_independently_authored_programs() {
    let indexed: ix::Module = syn::parse_str(ACTUAL_IX).unwrap();
    ix::contract::check(&indexed).unwrap();
    let expected_ix: ix::Module =
        syn::parse_str(include_str!("fixtures/physical-selection/expected.ix")).unwrap();
    assert_eq!(
        indexed.to_token_stream().to_string(),
        expected_ix.to_token_stream().to_string()
    );

    let selected: physical::Module = syn::parse_str(ACTUAL_PHYSICAL).unwrap();
    physical::contract::check_structure(&selected).unwrap();
    let expected: physical::Module = syn::parse_str(include_str!(
        "fixtures/physical-selection/expected.physical"
    ))
    .unwrap();
    assert_eq!(
        selected.to_token_stream().to_string(),
        expected.to_token_stream().to_string()
    );
    assert_eq!(selected.index_requirements, indexed);
}

#[test]
fn selecting_a_plan_does_not_resolve_or_interpret_opaque_provider_names() {
    // `external` is deliberately not a Rust module. Admission is a later
    // provider obligation, not something a structural selection test proves.
    let selected: physical::Module = syn::parse_str(ACTUAL_PHYSICAL).unwrap();
    assert_eq!(selected.assignments.relations.len(), 8);
    assert_eq!(selected.assignments.indexes.len(), 2);
    assert_eq!(selected.assignments.lowerings.len(), 5);
}
