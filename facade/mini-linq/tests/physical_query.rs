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

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a HashMap<(Value,), Vec<(Value,)>>,
        S: &'a BTreeMap<(Value,), Vec<(Value,)>>,
    ) -> HashSet<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(::mini_linq::compile! { pipeline {  }
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

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> Vec<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(::mini_linq::compile! { pipeline {  }
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

/// A caller-selected provider builds a user index for materialized results.
mod custom_indexes {
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

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> Vec<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(
            ::mini_linq::passes::select! { pipeline { ::core_pass_fixtures::custom_indexes, ::mini_linq::passes::print_pull };
                physical {
                    relation L(a: Value); relation R(a: Value, b: Value); relation S(b: Value, c: Value);
                    l = pipe rename L@0 {a -> a}; r = pipe rename R@1 {a -> a, b -> b};
                    s = let rename S@2 {b -> b, c -> c};
                    j = pipe probe first [a] l, r; p = pipe probe first [b] j, s;
                    out = pipe canon project p keep {a, c}; return out as answer(a, c);
                }
            },
        )
    }
}

#[test]
fn a_user_defined_index_type_is_built_and_probed() {
    let (l, r, s) = fixture();
    let actual: HashSet<_> = custom_indexes::run(&l, &r, &s).into_iter().collect();
    assert_eq!(actual, oracle(&l, &r, &s));
}

/// `join = merge`: both operands of every join are read in key order and
/// co-iterated; intermediates whose key changes are materialized into
/// ordered maps, an intermediate merged on its own key streams on.
mod merged {
    use super::*;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a BTreeMap<(Value,), Vec<()>>,
        R: &'a BTreeMap<(Value,), Vec<(Value,)>>,
        S: &'a BTreeMap<(Value,), Vec<(Value,)>>,
    ) -> std::collections::BTreeSet<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(
            ::mini_linq::compile! { pipeline { physical = [::core_pass_fixtures::merged]; structures = [::core_pass_fixtures::ordered_dedup]; }
                struct Chain;
                relation L(a: Value);
                relation R(a: Value, b: Value);
                relation S(b: Value, c: Value);
                answer(a, c) :- L(a), R(a, b), S(b, c);
            },
        )
    }
}

#[test]
fn merge_joins_over_host_and_built_ordered_maps_agree() {
    let (l, r, s) = fixture();
    let lm = l.iter().map(|(a,)| ((a.clone(),), vec![()])).collect();
    let rm = grouped(&r);
    let sm = grouped(&s);
    let actual: HashSet<_> = merged::run(&lm, &rm, &sm).into_iter().collect();
    assert_eq!(actual, oracle(&l, &r, &s));
}

/// The acyclic planner emits Yannakakis semijoin passes; they realize as
/// membership tests over built key sets before the final joins.
mod reduced {
    use super::*;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> HashSet<(&'a Value, &'a Value)> {
        ::core::iter::Iterator::collect(
            ::mini_linq::compile! { pipeline { plan = ::core_pass_fixtures::plan_hypergraph; }
                struct Chain;
                relation L(a: Value);
                relation R(a: Value, b: Value);
                relation S(b: Value, c: Value);
                answer(a, c) :- L(a), R(a, b), S(b, c);
            },
        )
    }
}

#[test]
fn yannakakis_plan_executes_through_semijoin_membership() {
    let (l, r, s) = fixture();
    assert_eq!(reduced::run(&l, &r, &s), oracle(&l, &r, &s));
}

/// Rust comparisons become relational constraints; the leading key window
/// reads the ordered host range while the remaining column predicate filters.
mod windowed {

    #[allow(non_snake_case)]
    pub(super) fn run(
        R: &std::collections::BTreeMap<(i32,), Vec<(i32,)>>,
    ) -> std::collections::HashSet<(&i32, &i32)> {
        ::core::iter::Iterator::collect(::mini_linq::compile! { pipeline {  }
            struct Windowed;
            relation R(a: i32, b: i32);
            answer(a, b) :- R(a, b), if *a >= 2 && *a < 9 && *b != 5;
        })
    }
}

#[test]
fn ordered_host_range_preserves_the_rust_comparison_answer() {
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

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        R: &'a [(i32, i32)],
        S: &'a [(i32, i32)],
    ) -> Vec<(&'a i32, String, &'a i32)> {
        ::core::iter::Iterator::collect(::mini_linq::compile! { pipeline {  }
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

/// Relational prefix canonicalization preserves the final set. The opaque
/// guard stays after the complete written join; evaluation counts are outside
/// the denotation.
mod generic_canon {
    use std::cell::Cell;

    fn observe(a: &i32, visits: &Cell<usize>) -> bool {
        // a=4 has an R prefix but no complete triangle. Evaluating this Rust
        // step on that prefix would move it before a written relational step.
        assert_ne!(*a, 4, "guard evaluated before the complete written prefix");
        visits.set(visits.get() + 1);
        true
    }

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        R: &'a [(i32, i32)],
        S: &'a [(i32, i32)],
        T: &'a [(i32, i32)],
        visits: &Cell<usize>,
    ) -> Vec<(&'a i32, &'a i32, &'a i32)> {
        ::core::iter::Iterator::collect(
            ::mini_linq::compile! { pipeline { plan = ::core_pass_fixtures::plan_generic; physical = [::core_pass_fixtures::canonical_projects]; }
                struct Triangle;
                relation R(a: i32, b: i32);
                relation S(b: i32, c: i32);
                relation T(a: i32, c: i32);
                answer(a, b, c) :- R(a, b), S(b, c), T(a, c), if observe(a, visits);
            },
        )
    }
}

#[test]
fn generic_prefix_canonicalization_preserves_the_written_guard_domain() {
    let r = [(1, 2), (1, 3), (1, 3), (2, 3), (4, 5)];
    let s = [(2, 3), (3, 4), (3, 1), (5, 6)];
    let t = [(1, 3), (1, 4), (2, 1), (4, 7)];
    let visits = std::cell::Cell::new(0);
    let actual = generic_canon::run(&r, &s, &t, &visits);
    let mut expected = HashSet::new();
    for (a, b) in &r {
        for (sb, c) in &s {
            if sb == b && t.iter().any(|(ta, tc)| ta == a && tc == c) {
                expected.insert((a, b, c));
            }
        }
    }
    assert_eq!(actual.len(), expected.len());
    assert_eq!(actual.into_iter().collect::<HashSet<_>>(), expected);
    assert!(
        visits.get() > 0,
        "the surviving tuples reach the final guard"
    );
}

/// Generic join: one variable at a time, every atom holding it checked
/// before enumeration continues. `T` is read on `[a]` and then `[a, c]` —
/// several keys, dispatched independently with shared per-read fallback state.
mod generic {

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        R: &'a [(i32, i32)],
        S: &'a [(i32, i32)],
        T: &'a [(i32, i32)],
    ) -> std::collections::HashSet<(&'a i32, &'a i32, &'a i32)> {
        ::core::iter::Iterator::collect(
            ::mini_linq::compile! { pipeline { plan = ::core_pass_fixtures::plan_generic; }
                struct Triangle;
                relation R(a: i32, b: i32);
                relation S(b: i32, c: i32);
                relation T(a: i32, c: i32);
                answer(a, b, c) :- R(a, b), S(b, c), T(a, c);
            },
        )
    }
}

mod generic_smallest {

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        R: &'a [(i32, i32)],
        S: &'a [(i32, i32)],
        T: &'a [(i32, i32)],
    ) -> std::collections::HashSet<(&'a i32, &'a i32, &'a i32)> {
        ::core::iter::Iterator::collect(
            ::mini_linq::compile! { pipeline { plan = ::core_pass_fixtures::plan_generic; physical = [::core_pass_fixtures::smallest]; }
                struct Triangle;
                relation R(a: i32, b: i32);
                relation S(b: i32, c: i32);
                relation T(a: i32, c: i32);
                answer(a, b, c) :- R(a, b), S(b, c), T(a, c);
            },
        )
    }
}

#[test]
fn generic_join_reads_multiple_host_keys_through_dispatch() {
    let r = [(1, 2), (1, 3), (2, 3), (4, 5)];
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
    assert_eq!(generic_smallest::run(&r, &s, &t), expected);
}

#[test]
fn key_only_filtered_index_preserves_existential_answers_without_primary_keys() {
    let l = [(1_i64,), (1,), (2,), (3,), (4,)];
    let r = [(1_i64, 2_i64), (1, 3), (1, 3), (2, 0), (3, 2), (9, 5)];
    let rows: Vec<_> = mini_linq::passes::select! {
        pipeline { mini_linq::passes::print_pull };
        physical {
            relation l(a:i64); relation r(a:i64,b:i64);
            left = pipe rename l@0 {a -> x};
            right = pipe rename r@1 {a -> x,b -> y};
            selected = let range select right if (y > 1);
            joined = pipe probe first [x] left,selected;
            result = pipe canon project joined keep {x}; return result as answer(x);
        }
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(rows, [1, 3]);
}
