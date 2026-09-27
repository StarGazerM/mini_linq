//! The real chain, CQ through Physical and Structures to executed Rust,
//! under several policies: host maps read in place, compiler-built hash
//! indexes, a user-defined index type, merge joins over compiler-built
//! ordered maps, the acyclic planner's Yannakakis plan, and a bounded scan
//! of an ordered host. Every query evaluates to an iterator of answers.
#![recursion_limit = "1024"]

use std::collections::{BTreeMap, HashMap, HashSet};

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

fn grouped<M: Default + Extend<((Value,), Vec<(Value,)>)>>(items: &[(Value, Value)]) -> M {
    let mut groups: BTreeMap<(Value,), Vec<(Value,)>> = BTreeMap::new();
    for (k, v) in items {
        groups.entry((k.clone(),)).or_default().push((v.clone(),));
    }
    let mut map = M::default();
    map.extend(groups);
    map
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

/// Host maps keyed on the join columns are read in place; nothing is built.
mod hosted {
    use super::*;

    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! {
                $($ix)*
                policy { L; R keyed [a]; S keyed [b]; }
            }
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
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a HashMap<(Value,), Vec<(Value,)>>,
        S: &'a BTreeMap<(Value,), Vec<(Value,)>>,
    ) -> HashSet<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra_written_order! {
            struct Chain;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            relation S(b: Value, c: Value);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        })
    }
}

#[test]
fn host_maps_are_probed_in_place() {
    let (l, r, s) = fixture();
    let r_map: HashMap<_, _> = grouped(&r);
    let s_map: BTreeMap<_, _> = grouped(&s);
    assert_eq!(hosted::run(&l, &r_map, &s_map), oracle(&l, &r, &s));
}

/// Hosts only iterate: the compiler builds hash indexes over borrowed rows
/// inside the query block; the answers stream out of it.
mod built {
    use super::*;

    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! {
                $($ix)*
                policy { L; R; S; }
            }
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
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> Vec<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra_written_order! {
            struct Chain;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            relation S(b: Value, c: Value);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        })
    }
}

#[test]
fn compiler_built_indexes_feed_the_pull_program() {
    let (l, r, s) = fixture();
    let actual: HashSet<_> = built::run(&l, &r, &s).into_iter().collect();
    assert_eq!(actual, oracle(&l, &r, &s));
    // Canonical return: duplicated inputs do not duplicate answers.
    let doubled: Vec<_> = l.iter().chain(&l).cloned().collect();
    assert_eq!(built::run(&doubled, &r, &s).len(), oracle(&l, &r, &s).len());
}

/// `#[index(Type)]` on a declaration makes the compiler build that Rust type
/// — here one defined in this test, unknown to `cq-storage` — over the
/// relation, reading it through the runtime traits the plan uses; rustc
/// checks the type implements them. `R`, unadvised, keeps the default hash
/// index.
mod advised {
    use super::*;

    /// A user-defined index: rows sorted by key, probed by binary search.
    pub struct SortedIndex<K, V>(Vec<(K, V)>);

    impl<K: Ord, V, I: Iterator<Item = (K, V)>> cq_storage::Build<I> for SortedIndex<K, V> {
        fn build(source: I) -> Self {
            let mut rows: Vec<(K, V)> = source.collect();
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            SortedIndex(rows)
        }
    }

    impl<K: Ord, V> SortedIndex<K, V> {
        fn run(&self, key: &K) -> &[(K, V)] {
            let low = self.0.partition_point(|(k, _)| k < key);
            let high = self.0.partition_point(|(k, _)| k <= key);
            &self.0[low..high]
        }
    }

    impl<'a, K: Ord, V: Copy> cq_storage::IndexRead for &'a SortedIndex<K, V> {
        type Key = K;
        type Item = V;
        type Cursor = std::iter::Map<std::slice::Iter<'a, (K, V)>, fn(&'a (K, V)) -> V>;
        fn lookup(&self, key: &K) -> Self::Cursor {
            self.run(key).iter().map(|(_, v)| *v)
        }
    }

    impl<K: Ord, V> cq_storage::ExactMembership<K> for SortedIndex<K, V> {
        fn contains(&self, key: &K) -> bool {
            !self.run(key).is_empty()
        }
    }

    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! {
                $($ix)*
                policy { L; R; S; }
            }
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
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> Vec<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra_written_order! {
            struct Chain;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            #[index(SortedIndex)]
            relation S(b: Value, c: Value);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        })
    }
}

#[test]
fn a_user_defined_index_type_is_built_and_probed() {
    let (l, r, s) = fixture();
    let actual: HashSet<_> = advised::run(&l, &r, &s).into_iter().collect();
    assert_eq!(actual, oracle(&l, &r, &s));
}

/// `join = merge`: both operands of every join are read in key order and
/// co-iterated; intermediates whose key changes are materialized into
/// ordered maps, an intermediate merged on its own key streams on.
mod merged {
    use super::*;

    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! {
                $($ix)*
                policy { L; R; S; join = merge; }
            }
        };
    }
    pub(crate) use policy as physical_policy;
    mod wiring {
        pub(crate) use super::physical_policy;
        pub(crate) use ::mini_linq::{
            index_demands as choose_index_policy, select_structures, structures_to_rust,
        };
        pub(crate) use cq_state_providers::ordered_distinct as distinct_state;
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> std::collections::BTreeSet<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra_written_order! {
            struct Chain;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            relation S(b: Value, c: Value);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        })
    }
}

#[test]
fn merge_joins_over_built_ordered_maps_agree() {
    let (l, r, s) = fixture();
    let actual: HashSet<_> = merged::run(&l, &r, &s).into_iter().collect();
    assert_eq!(actual, oracle(&l, &r, &s));
}

/// The acyclic planner emits Yannakakis semijoin passes; they realize as
/// membership tests over built key sets before the final joins.
mod reduced {
    use super::*;

    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! {
                $($ix)*
                policy { L; R; S; }
            }
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
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> HashSet<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra_hypergraph! {
            struct Chain;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            relation S(b: Value, c: Value);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        })
    }
}

#[test]
fn yannakakis_plan_executes_through_semijoin_membership() {
    let (l, r, s) = fixture();
    assert_eq!(reduced::run(&l, &r, &s), oracle(&l, &r, &s));
}

/// A comparison on an ordered host column is pushed into the cursor: the
/// host map is entered at the lower bound and left at the upper bound; the
/// remaining conjunct stays a guard.
mod windowed {
    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! {
                $($ix)*
                policy { R keyed [a] ordered; }
            }
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
    pub(super) fn run(
        R: &std::collections::BTreeMap<(i32,), Vec<(i32,)>>,
    ) -> std::collections::HashSet<(&i32, &i32)> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra_clause_prefix! {
            struct Windowed;
            relation R(a: i32, b: i32);
            answer(a, b) :- R(a, b), if a >= 2 && a < 9 && b != 5;
        })
    }
}

#[test]
fn ordered_host_scans_are_bounded_by_pushed_comparisons() {
    let host: BTreeMap<(i32,), Vec<(i32,)>> = [
        ((1,), vec![(0,)]),
        ((2,), vec![(5,), (6,)]),
        ((3,), vec![(5,)]),
        ((8,), vec![(1,)]),
        ((9,), vec![(1,)]),
    ]
    .into_iter()
    .collect();
    let actual: HashSet<(i32, i32)> = windowed::run(&host)
        .into_iter()
        .map(|(a, b)| (*a, *b))
        .collect();
    assert_eq!(actual, HashSet::from([(2, 6), (8, 1)]));
}

/// A computed column and the head's own argument order: the answer tuple
/// follows `answer(a, s, c)`, not the sorted heading, and the column
/// reference inside a nested macro resolves.
mod computed {
    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::lower_physical! {
                $($ix)*
                policy { R; S; }
            }
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
    pub(super) fn run<'a>(
        R: &'a [(i32, i32)],
        S: &'a [(i32, i32)],
    ) -> Vec<(&'a i32, String, &'a i32)> {
        ::core::iter::Iterator::collect(::mini_linq::cq_to_ra_clause_prefix! {
            struct Computed;
            relation R(a: i32, b: i32);
            relation S(b: i32, c: i32);
            answer(a, s, c) :- R(a, b), S(b, c), let s: String = format!("{}", a);
        })
    }
}

#[test]
fn computed_columns_follow_the_head_order() {
    assert_eq!(
        computed::run(&[(1, 2), (1, 2)], &[(2, 3)]),
        vec![(&1, "1".to_string(), &3)]
    );
}
