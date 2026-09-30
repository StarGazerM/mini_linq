// These fixtures use host reads; retain the provider slot for built let reads.
#[allow(unused_macros)]
macro_rules! indexes {
    (probe) => { ::cq_storage::HashIndex<'_, _, _> };
    (semi) => { ::std::collections::HashSet<_> };
    (ordered) => { ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> };
    (trie $inner:ty) => { ::cq_storage::Trie<'_, _, $inner> };
}
pub(crate) use indexes as index_provider;
use std::collections::HashSet;
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Value(i32);

fn values(items: impl IntoIterator<Item = i32>) -> Vec<(Value,)> {
    items.into_iter().map(|v| (Value(v),)).collect()
}

fn pairs(items: impl IntoIterator<Item = (i32, i32)>) -> Vec<(Value, Value)> {
    items
        .into_iter()
        .map(|(a, b)| (Value(a), Value(b)))
        .collect()
}

fn oracle<'a>(
    l: &'a [(Value,)],
    r: &'a [(Value, Value)],
    s: &'a [(Value, Value)],
) -> HashSet<(&'a Value, &'a Value)> {
    let mut out = HashSet::new();
    for (a,) in l {
        for (ra, b) in r {
            if ra != a {
                continue;
            }
            for (sb, c) in s {
                if sb == b {
                    out.insert((a, c));
                }
            }
        }
    }
    out
}

type Fixture = (Vec<(Value,)>, Vec<(Value, Value)>, Vec<(Value, Value)>);

fn fixture() -> Fixture {
    (
        values([1, 2, 3, 9]),
        pairs([(1, 10), (1, 11), (2, 12), (3, 13), (4, 14)]),
        pairs([(10, 100), (11, 101), (11, 102), (12, 100), (14, 140)]),
    )
}

mod built {
    use super::*;

    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! {
                $($ix)*
                policy {    }
            }
        };
    }
    pub(crate) use policy as physical_policy;
    mod wiring {
        pub(crate) use super::physical_policy;
        pub(crate) use crate::index_provider;
        pub(crate) use ::mini_linq::{index_demands as choose_index_policy, select_structures};
        pub(crate) use own_push::{push as structures_to_rust, set_sink as distinct_state};
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> Vec<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra! {
            struct Chain;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            relation S(b: Value, c: Value);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        })
    }
}

mod generic {
    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! {
                $($ix)*
                policy {    }
            }
        };
    }
    pub(crate) use policy as physical_policy;
    mod wiring {
        pub(crate) use super::physical_policy;
        pub(crate) use crate::index_provider;
        pub(crate) use ::mini_linq::{index_demands as choose_index_policy, select_structures};
        pub(crate) use own_push::{push as structures_to_rust, set_sink as distinct_state};
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        R: &'a [(i32, i32)],
        S: &'a [(i32, i32)],
        T: &'a [(i32, i32)],
    ) -> std::collections::HashSet<(&'a i32, &'a i32, &'a i32)> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra_generic! {
            struct Triangle;
            relation R(a: i32, b: i32);
            relation S(b: i32, c: i32);
            relation T(a: i32, c: i32);
            answer(a, b, c) :- R(a, b), S(b, c), T(a, c);
        })
    }
}
#[test]
fn built_matches_the_original_oracle_with_duplicate_inputs() {
    let (l, r, s) = fixture();
    let expected = oracle(&l, &r, &s);
    assert_eq!(
        built::run(&l, &r, &s).into_iter().collect::<HashSet<_>>(),
        expected
    );
    let doubled: Vec<_> = l.iter().chain(&l).cloned().collect();
    assert_eq!(built::run(&doubled, &r, &s).len(), expected.len());
    assert!(built::run(&[], &r, &s).is_empty());
}
#[test]
fn generic_matches_a_nested_loop_oracle() {
    for r in [vec![], vec![(1, 2), (1, 2), (2, 3), (3, 1)]] {
        let s = [(2, 3), (3, 4), (3, 1), (5, 6)];
        let t = [(1, 3), (1, 4), (2, 1), (4, 7)];
        let mut expected = HashSet::new();
        for (a, b) in &r {
            for (sb, c) in &s {
                if sb == b && t.iter().any(|(ta, tc)| ta == a && tc == c) {
                    expected.insert((a, b, c));
                }
            }
        }
        assert_eq!(generic::run(&r, &s, &t), expected);
    }
}
