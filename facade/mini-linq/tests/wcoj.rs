//! Hash operations include construction and execution, but exclude fixture
//! creation and the independent ordered-set oracle. A fresh query builds its
//! structures on every run; the counter is isolated per test thread. Both
//! generic plans canonicalize prefix projections so repeated prefixes do not
//! multiply the work being measured at the candidate intersection.
use std::cell::Cell;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

thread_local! { static HASHES: Cell<usize> = const { Cell::new(0) }; }

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct V(i32);
impl Hash for V {
    fn hash<H: Hasher>(&self, state: &mut H) {
        HASHES.with(|n| n.set(n.get() + 1));
        self.0.hash(state);
    }
}

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
fn fan_hash_counts_prove_smallest_avoids_quadratic_enumeration() {
    let mut counts = Vec::new();
    println!("n\tbinary\tfirst\tsmallest");
    for n in [50, 100, 200] {
        let fan: Vec<_> = (1..=n).flat_map(|i| [(V(0), V(i)), (V(i), V(0))]).collect();
        let expected = oracle(&fan, &fan, &fan);
        let mut row = Vec::new();
        for run in [binary::run, first::run, smallest::run] {
            HASHES.with(|n| n.set(0));
            let actual = run(&fan, &fan, &fan);
            let hashes = HASHES.with(Cell::get);
            assert_eq!(actual, expected);
            row.push(hashes);
        }
        println!("{n}\t{}\t{}\t{}", row[0], row[1], row[2]);
        counts.push(row);
    }
    for pair in counts.windows(2) {
        for strategy in [0, 1] {
            assert!(
                pair[1][strategy] * 2 >= pair[0][strategy] * 7,
                "binary and first must grow at least 3.5x: {counts:?}"
            );
        }
        assert!(
            pair[1][2] * 2 <= pair[0][2] * 5,
            "smallest must grow at most 2.5x: {counts:?}"
        );
    }
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
