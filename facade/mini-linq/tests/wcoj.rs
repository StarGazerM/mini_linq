//! Candidate intersection semantics; seeded fan growth lives in cq-workloads'
//! feature-gated costs framework, shared with cargo xtask bench.
use std::collections::BTreeSet;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
struct V(i32);

type Answer<'a> = BTreeSet<(&'a V, &'a V, &'a V)>;

mod binary {
    use super::{Answer, V};
    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! { $($ix)* policy {    } }
        };
    }
    pub(crate) use policy as physical_policy;
    mod wiring {
        pub(crate) use super::physical_policy;
        pub(crate) use ::mini_linq::{
            index_demands as choose_index_policy, select_structures, structures_to_rust,
        };
        pub(crate) use cq_state_providers::default_indexes as index_provider;
        pub(crate) use cq_state_providers::hash_distinct as distinct_state;
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(R: &'a [(V, V)], S: &'a [(V, V)], T: &'a [(V, V)]) -> Answer<'a> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra! {
            struct Triangle;
            relation R(a: V, b: V);
            relation S(b: V, c: V);
            relation T(a: V, c: V);
            answer(a, b, c) :- R(a, b), S(b, c), T(a, c);
        })
    }
}

mod first {
    use super::{Answer, V};
    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! { $($ix)* policy {    project = canon; } }
        };
    }
    pub(crate) use policy as physical_policy;
    mod wiring {
        pub(crate) use super::physical_policy;
        pub(crate) use ::mini_linq::{
            index_demands as choose_index_policy, select_structures, structures_to_rust,
        };
        pub(crate) use cq_state_providers::default_indexes as index_provider;
        pub(crate) use cq_state_providers::hash_distinct as distinct_state;
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(R: &'a [(V, V)], S: &'a [(V, V)], T: &'a [(V, V)]) -> Answer<'a> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra_generic! {
            struct Triangle;
            relation R(a: V, b: V);
            relation S(b: V, c: V);
            relation T(a: V, c: V);
            answer(a, b, c) :- R(a, b), S(b, c), T(a, c);
        })
    }
}

mod smallest {
    use super::{Answer, V};
    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! { $($ix)* policy {    project = canon; intersect = smallest; } }
        };
    }
    pub(crate) use policy as physical_policy;
    mod wiring {
        pub(crate) use super::physical_policy;
        pub(crate) use ::mini_linq::{
            index_demands as choose_index_policy, select_structures, structures_to_rust,
        };
        pub(crate) use cq_state_providers::default_indexes as index_provider;
        pub(crate) use cq_state_providers::hash_distinct as distinct_state;
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(R: &'a [(V, V)], S: &'a [(V, V)], T: &'a [(V, V)]) -> Answer<'a> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra_generic! {
            struct Triangle;
            relation R(a: V, b: V);
            relation S(b: V, c: V);
            relation T(a: V, c: V);
            answer(a, b, c) :- R(a, b), S(b, c), T(a, c);
        })
    }
}

fn oracle<'a>(r: &'a [(V, V)], s: &'a [(V, V)], t: &'a [(V, V)]) -> Answer<'a> {
    let t: BTreeSet<_> = t.iter().map(|(a, c)| (a, c)).collect();
    r.iter()
        .flat_map(|(a, b)| {
            let t = &t;
            s.iter()
                .filter_map(move |(sb, c)| (sb == b && t.contains(&(a, c))).then_some((a, b, c)))
        })
        .collect()
}

#[test]
fn candidate_choice_preserves_nonempty_answers_with_duplicate_rows() {
    let r = [(V(1), V(2)), (V(1), V(2)), (V(1), V(3)), (V(4), V(5))];
    let s = [(V(2), V(6)), (V(2), V(7)), (V(3), V(6)), (V(5), V(8))];
    let t = [(V(1), V(6)), (V(1), V(6)), (V(4), V(8)), (V(4), V(9))];
    let expected = oracle(&r, &s, &t);
    assert!(!expected.is_empty());
    for run in [binary::run, first::run, smallest::run] {
        assert_eq!(run(&r, &s, &t), expected);
    }
}
