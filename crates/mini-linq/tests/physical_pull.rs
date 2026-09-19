//! The real CQ chain reaches the existing lazy Pipeline endpoint.
#![recursion_limit = "1024"]

#[path = "support/trace.rs"]
mod trace;

use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap, HashSet},
};

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Value(i32);

thread_local! {
    // Each provider records source evaluations and successful row polls.
    static OBSERVATIONS: Cell<[[usize; 2]; 3]> = const { Cell::new([[0; 2]; 3]) };
}

fn observe<I: Iterator>(provider: usize, source: I) -> impl Iterator<Item = I::Item> {
    OBSERVATIONS.with(|counts| {
        let mut value = counts.get();
        value[provider][0] += 1;
        counts.set(value);
    });
    source.inspect(move |_| {
        OBSERVATIONS.with(|counts| {
            let mut value = counts.get();
            value[provider][1] += 1;
            counts.set(value);
        });
    })
}

// The caller adds observation to the provider's iterable leaf. Neither this
// wrapper nor the provider chooses loops or dependent iterator composition.
macro_rules! observe_provider {
    (provider ($provider:path) kind ($kind:literal)
     reply_to ($callback:path) {$($state:tt)*}
     plan {$($plan:tt)*} environment {$($environment:tt)*} request {$($request:tt)*}) => {
        $provider! {
            reply_to (observe_reply) {
                callback ($callback) kind ($kind) state {$($state)*}
            }
            plan {$($plan)*} environment {$($environment)*} request {$($request)*}
        }
    };
}

macro_rules! observe_reply {
    (resume {callback ($callback:path) kind ($kind:literal) state {$($state:tt)*}}
     source ($source:expr) item ($pattern:pat) witness {$($witness:tt)*}) => {
        $callback! {
            resume {$($state)*}
            source (observe($kind, $source)) item ($pattern) witness {$($witness)*}
        }
    };
    (resume {callback ($callback:path) kind ($kind:literal) state {$($state:tt)*}}
     emitted {$($emitted:tt)*} result {$($result:tt)*}) => {
        $callback! {
            resume {$($state)*} emitted {$($emitted)*} result {$($result)*}
        }
    };
}

macro_rules! observed_slice {
    ($($request:tt)*) => {
        observe_provider! {provider (::cq_example_providers::slice_rows) kind (0) $($request)*}
    };
}
macro_rules! observed_hash {
    ($($request:tt)*) => {
        observe_provider! {provider (::cq_example_providers::hash_relation) kind (1) $($request)*}
    };
}
macro_rules! observed_sorted {
    ($($request:tt)*) => {
        observe_provider! {provider (::cq_example_providers::sorted_relation) kind (2) $($request)*}
    };
}

macro_rules! explicit_policy {
    ($($ix:tt)*) => {
        ::mini_linq::given_physical_bindings! {
            $($ix)*
            physical {
                relations {
                    L = observed_slice;
                    R = observed_hash;
                    S = observed_sorted;
                    r0 = observed_slice;
                    r1 = observed_hash;
                    r2 = ::cq_example_providers::borrowed_fields;
                    r3 = observed_sorted;
                    r4 = ::cq_example_providers::borrowed_fields;
                    r5 = ::cq_example_providers::borrowed_fields;
                }
                indexes { i0 = observed_hash; i1 = observed_sorted; }
                lowerings {
                    r0 = ::cq_example_providers::rename;
                    r1 = ::cq_example_providers::rename;
                    r2 = ::cq_example_providers::indexed_join;
                    r3 = ::cq_example_providers::rename;
                    r4 = ::cq_example_providers::indexed_join;
                    r5 = ::cq_example_providers::project;
                }
            }
        }
    };
}
use explicit_policy as choose_physical;
mod wiring {
    pub(crate) use super::choose_physical;
    pub(crate) use ::mini_linq::{
        access_to_distinct_iter as access, index_demands as choose_index_policy,
        pipeline_to_distinct_iter, realize_physical,
    };
}
use wiring as mini_linq;

#[allow(non_snake_case)]
fn run<'a>(
    L: &'a [Value],
    R: &'a HashMap<Value, Vec<Value>>,
    S: &'a BTreeMap<Value, Vec<Value>>,
) -> impl Iterator<Item = (&'a Value, &'a Value)> + 'a {
    ::mini_linq::cq_to_ra_written_order! {
        struct PullChain;
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
    let mut result = HashSet::new();
    for a in left {
        for (ra, values) in middle {
            for b in values {
                for (sb, values) in right {
                    for c in values {
                        if a == ra && b == sb {
                            result.insert((a, c));
                        }
                    }
                }
            }
        }
    }
    result
}

#[test]
fn full_cq_pull_defers_every_source_and_short_circuits_each_provider() {
    let left = [Value(1), Value(2)];
    let middle = HashMap::from([
        (Value(1), vec![Value(10), Value(11)]),
        (Value(2), vec![Value(20)]),
    ]);
    let right = BTreeMap::from([
        (Value(10), vec![Value(100), Value(101)]),
        (Value(11), vec![Value(110)]),
        (Value(20), vec![Value(200)]),
    ]);
    OBSERVATIONS.set([[0; 2]; 3]);
    drop(run(&left, &middle, &right));
    assert_eq!(OBSERVATIONS.get(), [[0; 2]; 3]);
    let mut cursor = run(&left, &middle, &right);
    assert_eq!(OBSERVATIONS.get(), [[0; 2]; 3]);
    assert!(cursor.any(|(a, c)| a.0 == 1 && c.0 == 100));
    assert_eq!(OBSERVATIONS.get(), [[1, 1]; 3]);
    // Resumption preserves both the outer binding and the unconsumed bucket.
    assert_eq!(cursor.next(), Some((&left[0], &right[&Value(10)][1])));
    assert_eq!(OBSERVATIONS.get(), [[1, 1], [1, 1], [1, 2]]);
}

#[test]
fn full_cq_pull_preserves_set_results_and_borrows_non_clone_payloads() {
    let left = [Value(1), Value(1), Value(2), Value(9)];
    let middle = HashMap::from([
        (Value(1), vec![Value(10), Value(10), Value(11)]),
        (Value(2), vec![Value(20)]),
    ]);
    let right = BTreeMap::from([
        (Value(10), vec![Value(100), Value(101), Value(101)]),
        (Value(11), vec![Value(100)]),
        (Value(20), vec![Value(200)]),
    ]);
    let rows: Vec<_> = run(&left, &middle, &right).collect();
    let actual: HashSet<_> = rows.iter().copied().collect();
    assert_eq!(actual, oracle(&left, &middle, &right));
    assert_eq!(rows.len(), actual.len());
    for (a, c) in rows {
        assert!(left.iter().any(|value| std::ptr::eq(value, a)));
        assert!(right.values().flatten().any(|value| std::ptr::eq(value, c)));
    }
}

#[test]
fn full_cq_pull_matches_relational_enumeration_over_small_input_subsets() {
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
        let rows: Vec<_> = run(&left, &middle, &right).collect();
        let actual: HashSet<_> = rows.iter().copied().collect();
        assert_eq!(actual, oracle(&left, &middle, &right));
        assert_eq!(rows.len(), actual.len());
    }
}

#[test]
#[ignore = "requires a fresh real proc-macro trace in CQ_REALIZATION_TRACE"]
fn captured_boundaries_equal_authored_ix_physical_access_and_pipeline() {
    trace::verify(
        Some(include_str!("fixtures/physical-pull/expected.ra")),
        &format!(
            "{}\n{}",
            include_str!("fixtures/physical-pull/expected.ra"),
            include_str!("fixtures/physical-pull/expected.indexes"),
        ),
        include_str!("fixtures/physical-pull/expected.selection"),
        include_str!("fixtures/physical-pull/expected.rs"),
    );
    trace::verify_pipeline(include_str!("fixtures/physical-pull/expected.pipeline"));
}
