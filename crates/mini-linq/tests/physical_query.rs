//! Start at the real CQ macro and cross every boundary through projection.
//! Requires general physical selection. Do not supply a table of query-local
//! relation, index, or operator names to make this acceptance test pass.
#![recursion_limit = "768"]

#[path = "support/trace.rs"]
mod trace;

use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Value(i32);

mod wiring {
    pub(crate) use ::mini_linq::{
        access_to_set_loops as access, index_demands as choose_index_policy, realize_physical,
    };
    #[allow(unused_imports)]
    pub(crate) use cq_example_providers::{
        hash_distinct as distinct_state, hash_set_sink as set_sink,
    };
}
use wiring as mini_linq;

#[allow(non_snake_case)]
fn run<'a>(
    L: &'a [Value],
    R: &'a HashMap<Value, Vec<Value>>,
    S: &'a BTreeMap<Value, Vec<Value>>,
) -> HashSet<(&'a Value, &'a Value)> {
    ::mini_linq::cq_to_ra_written_order! {
        struct ProjectedChain;
        relation L(a: Value);
        relation R(a: Value, b: Value);
        relation S(b: Value, c: Value);
        answer(a, c) :- L(a), R(a, b), S(b, c);
    }
}

fn oracle<'a>(
    left: &'a [Value],
    middle: &'a HashMap<Value, Vec<Value>>,
    right: &'a BTreeMap<Value, Vec<Value>>,
) -> HashSet<(&'a Value, &'a Value)> {
    let mut output = HashSet::new();
    for a in left {
        for (ra, values) in middle {
            for b in values {
                for (sb, values) in right {
                    for c in values {
                        if a == ra && b == sb {
                            output.insert((a, c));
                        }
                    }
                }
            }
        }
    }
    output
}

#[test]
fn cq_to_projected_access_preserves_set_results_and_borrows_non_clone_data() {
    let left = [Value(1), Value(1), Value(2), Value(9)];
    let middle = HashMap::from([
        (Value(1), vec![Value(10), Value(10), Value(11)]),
        (Value(2), vec![Value(20)]),
        (Value(3), vec![Value(30)]),
    ]);
    let right = BTreeMap::from([
        (Value(10), vec![Value(100), Value(101), Value(101)]),
        // Distinct b values give the same projected (a,c) pair.
        (Value(11), vec![Value(100)]),
        (Value(20), vec![Value(200)]),
        (Value(99), vec![Value(999)]),
    ]);
    let result = run(&left, &middle, &right);
    assert_eq!(result, oracle(&left, &middle, &right));
    assert_eq!(result.len(), 3);
    for (a, c) in result {
        assert!(left.iter().any(|v| std::ptr::eq(v, a)));
        assert!(right.values().flatten().any(|v| std::ptr::eq(v, c)));
    }
}

#[test]
fn projected_cq_matches_relational_enumeration_over_small_input_subsets() {
    for mask in 0u16..256 {
        let left: Vec<_> = [1, 2]
            .into_iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1 << bit) != 0)
            .map(|(_, value)| Value(value))
            .collect();
        let mut middle = HashMap::new();
        for (bit, (a, b)) in [(1, 10), (1, 11), (2, 20)].into_iter().enumerate() {
            if mask & (1 << (bit + 2)) != 0 {
                middle
                    .entry(Value(a))
                    .or_insert_with(Vec::new)
                    .push(Value(b));
            }
        }
        let mut right = BTreeMap::new();
        for (bit, (b, c)) in [(10, 100), (11, 100), (20, 200)].into_iter().enumerate() {
            if mask & (1 << (bit + 5)) != 0 {
                right
                    .entry(Value(b))
                    .or_insert_with(Vec::new)
                    .push(Value(c));
            }
        }
        assert_eq!(run(&left, &middle, &right), oracle(&left, &middle, &right));
    }
}

#[test]
#[ignore = "requires a fresh real proc-macro trace in CQ_REALIZATION_TRACE"]
fn captured_boundaries_equal_authored_ix_physical_and_access() {
    trace::verify(
        Some(include_str!("fixtures/physical-query/expected.ra")),
        &format!(
            "{}\n{}",
            include_str!("fixtures/physical-query/expected.ra"),
            include_str!("fixtures/physical-query/expected.indexes"),
        ),
        include_str!("fixtures/physical-query/expected.selection"),
        include_str!("fixtures/physical-query/expected.rs"),
    );
}
