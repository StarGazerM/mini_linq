//! The real chain, CQ through Stream to executed Rust, under three policies:
//! host maps read in place, compiler-built hash indexes, and merge joins over
//! compiler-built ordered maps; plus the acyclic planner's Yannakakis plan.
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
            ::mini_linq::ix_to_stream! {
                $($ix)*
                policy {
                    L provides { iter };
                    R provides { seek @R[0] };
                    S provides { seek @S[0] };
                }
            }
        };
    }
    pub(crate) use policy as stream_policy;
    mod wiring {
        pub(crate) use super::stream_policy;
        pub(crate) use ::mini_linq::{
            access_to_set_loops as access, index_demands as choose_index_policy, stream_to_access,
        };
        #[allow(unused_imports)]
        pub(crate) use cq_state_providers::{
            hash_distinct as distinct_state, hash_set_sink as set_sink,
        };
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a HashMap<(Value,), Vec<(Value,)>>,
        S: &'a BTreeMap<(Value,), Vec<(Value,)>>,
    ) -> HashSet<(&'a Value, &'a Value)> {
        ::mini_linq::cq_to_ra_written_order! {
            struct Chain;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            relation S(b: Value, c: Value);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        }
    }
}

#[test]
fn host_maps_are_probed_in_place() {
    let (l, r, s) = fixture();
    let r_map: HashMap<_, _> = grouped(&r);
    let s_map: BTreeMap<_, _> = grouped(&s);
    assert_eq!(hosted::run(&l, &r_map, &s_map), oracle(&l, &r, &s));
}

/// Hosts only iterate: the compiler builds hash indexes over borrowed rows.
/// Built structures live in the query block, so the lazy pull endpoint is
/// consumed there by the caller's `access` binding.
mod built {
    use super::*;

    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::ix_to_stream! {
                $($ix)*
                policy { L provides { iter }; R provides { iter }; S provides { iter }; }
            }
        };
    }
    pub(crate) use policy as stream_policy;
    macro_rules! collect_pull {
        ($($access:tt)*) => {
            ::core::iter::Iterator::collect::<::std::vec::Vec<_>>(
                ::mini_linq::access_to_distinct_iter! { $($access)* }
            )
        };
    }
    pub(crate) use collect_pull as access;
    mod wiring {
        pub(crate) use super::{access, stream_policy};
        pub(crate) use ::mini_linq::{
            index_demands as choose_index_policy, pipeline_to_distinct_iter,
            pipeline_to_support_iter, stream_to_access,
        };
        #[allow(unused_imports)]
        pub(crate) use cq_state_providers::{
            hash_distinct as distinct_state, hash_set_sink as set_sink,
        };
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> Vec<(&'a Value, &'a Value)> {
        ::mini_linq::cq_to_ra_written_order! {
            struct Chain;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            relation S(b: Value, c: Value);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        }
    }
}

#[test]
fn compiler_built_indexes_feed_the_lazy_pull_endpoint() {
    let (l, r, s) = fixture();
    let actual: HashSet<_> = built::run(&l, &r, &s).into_iter().collect();
    assert_eq!(actual, oracle(&l, &r, &s));
    // Distinct: duplicated inputs do not duplicate answers.
    let doubled: Vec<_> = l.iter().chain(&l).cloned().collect();
    assert_eq!(built::run(&doubled, &r, &s).len(), oracle(&l, &r, &s).len());
}

/// Merge joins: every operand is materialized ordered and co-iterated.
mod merged {
    use super::*;

    macro_rules! policy {
        ($($ix:tt)*) => {
            ::mini_linq::ix_to_stream! {
                $($ix)*
                policy {
                    L provides { iter }; R provides { iter }; S provides { iter };
                    join = merge;
                }
            }
        };
    }
    pub(crate) use policy as stream_policy;
    mod wiring {
        pub(crate) use super::stream_policy;
        pub(crate) use ::mini_linq::{
            access_to_set as access, index_demands as choose_index_policy, pipeline_to_set,
            pipeline_to_support_iter, stream_to_access,
        };
        #[allow(unused_imports)]
        pub(crate) use cq_state_providers::{
            hash_distinct as distinct_state, ordered_set_sink as set_sink,
        };
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> std::collections::BTreeSet<(&'a Value, &'a Value)> {
        ::mini_linq::cq_to_ra_written_order! {
            struct Chain;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            relation S(b: Value, c: Value);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        }
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
            ::mini_linq::ix_to_stream! {
                $($ix)*
                policy { L provides { iter }; R provides { iter }; S provides { iter }; }
            }
        };
    }
    pub(crate) use policy as stream_policy;
    mod wiring {
        pub(crate) use super::stream_policy;
        pub(crate) use ::mini_linq::{
            access_to_set_loops as access, index_demands as choose_index_policy,
            pipeline_to_support_iter, stream_to_access,
        };
        #[allow(unused_imports)]
        pub(crate) use cq_state_providers::{
            hash_distinct as distinct_state, hash_set_sink as set_sink,
        };
    }
    use wiring as mini_linq;

    #[allow(non_snake_case)]
    pub(super) fn run<'a>(
        L: &'a [(Value,)],
        R: &'a [(Value, Value)],
        S: &'a [(Value, Value)],
    ) -> HashSet<(&'a Value, &'a Value)> {
        ::mini_linq::cq_to_ra_hypergraph! {
            struct Chain;
            relation L(a: Value);
            relation R(a: Value, b: Value);
            relation S(b: Value, c: Value);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        }
    }
}

#[test]
fn yannakakis_plan_executes_through_semijoin_membership() {
    let (l, r, s) = fixture();
    assert_eq!(reduced::run(&l, &r, &s), oracle(&l, &r, &s));
}
