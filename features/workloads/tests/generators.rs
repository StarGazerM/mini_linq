//! DuckDB-free comparisons against ordinary Rust loops in written clause order.
//! These references execute Rust, rather than extending the integer interpreter.
#![allow(non_snake_case)]

use cq_testkit::harness::{Outcome, check_refinement};
use std::collections::BTreeSet;

#[test]
fn exclusive_and_inclusive_ranges_match_written_rust() {
    let start = -2_i32;
    let end = 3_i32;
    let Seed = [(0_i32,)];
    let observation = check_refinement(
        || {
            let mut rows = BTreeSet::new();
            for _ in &Seed {
                for x in start..end {
                    for y in x..=end {
                        rows.insert((x, y));
                    }
                }
            }
            rows
        },
        || {
            mini_linq::compile! {
                pipeline { plan = cq_planners::order(written, coarse); }
                struct Ranges;
                relation Seed(unit: i32);
                relation answer(x: i32, y: i32);
                answer(x, y) :- Seed(_), for x in start..end, for y in x.to_owned()..=end;
            }
            .map(|(x, y)| (x.to_owned(), y.to_owned()))
            .collect::<BTreeSet<_>>()
        },
    )
    .unwrap();
    assert!(matches!(&observation.reference, Outcome::Returned(rows) if rows.len() == 20));
}

#[test]
fn vec_patterns_duplicates_and_empty_iterables_have_set_semantics() {
    let Seed = [(0_i32,)];
    for values in [vec![(2_i32, 7_i32), (2, 7), (3, 8)], Vec::new()] {
        let observation = check_refinement(
            || {
                let mut rows = BTreeSet::new();
                for _ in &Seed {
                    for (x, _) in values.clone() {
                        rows.insert(x);
                    }
                }
                rows
            },
            || {
                mini_linq::compile! {
                    pipeline { plan = cq_planners::order(written, coarse); }
                    struct Values;
                    relation Seed(unit: i32);
                    relation answer(x: i32);
                    answer(x) :- Seed(_), for (x, _) in values.clone();
                }
                .map(|(x,)| x.to_owned())
                .collect::<BTreeSet<_>>()
            },
        )
        .unwrap();
        assert_eq!(
            observation.reference,
            Outcome::Returned(values.iter().map(|&(x, _)| x).collect())
        );
    }
}

#[test]
fn captures_scalar_bindings_joins_and_nested_generators_match_written_rust() {
    let R = [(1_i32,), (1,), (2,)];
    let S = [(2_i32, 20_i32), (3, 30), (3, 30), (9, 90)];
    let offset = 1_i32;
    let limit = 4_i32;
    let reference = || {
        let mut rows = BTreeSet::new();
        for &(x,) in &R {
            let start = x + offset;
            for y in start..limit {
                for &(key, z) in &S {
                    if key == y {
                        for (w, _) in vec![(z, false), (z, true)] {
                            rows.insert((x, y, w));
                        }
                    }
                }
            }
        }
        rows
    };
    macro_rules! compare {
        ($($plan:tt)*) => {
            check_refinement(reference, || {
                mini_linq::compile! {
                    pipeline { plan = cq_planners::order($($plan)*); }
                    struct Joined;
                    relation R(x: i32);
                    relation S(y: i32, z: i32);
                    relation answer(x: i32, y: i32, w: i32);
                    answer(x, y, w) :- R(x), let start: i32 = x.to_owned() + offset,
                        for y in start.to_owned()..limit, S(y, z),
                        for (w, _) in vec![(z.to_owned(), false), (z.to_owned(), true)];
                }
                .map(|(x, y, w)| (x.to_owned(), y.to_owned(), w.to_owned()))
                .collect::<BTreeSet<_>>()
            }).unwrap()
        };
    }
    assert_eq!(
        reference(),
        BTreeSet::from([(1, 2, 20), (1, 3, 30), (2, 3, 30)])
    );
    compare!(written, coarse);
    compare!(hypergraph);
    compare!(elimination, fine);
}

fn partial_range(denominator: i32) -> std::ops::Range<i32> {
    0..(6 / denominator)
}

fn partial_iterator() -> impl Iterator<Item = i32> {
    (0..2).map(|index| {
        assert_eq!(index, 0, "panic after yielding an answer");
        7
    })
}

#[test]
fn empty_prefix_and_empty_generator_suppress_later_partial_rust() {
    let R: [(i32,); 0] = [];
    let empty_prefix = check_refinement(
        || {
            let mut rows = BTreeSet::new();
            for &(x,) in &R {
                for y in partial_range(x) {
                    rows.insert(y);
                }
            }
            rows
        },
        || {
            mini_linq::compile! {
                pipeline { plan = cq_planners::order(written, coarse); }
                struct EmptyPrefix;
                relation R(x: i32);
                relation answer(y: i32);
                answer(y) :- R(x), for y in partial_range(x.to_owned());
            }
            .map(|(y,)| y.to_owned())
            .collect::<BTreeSet<_>>()
        },
    )
    .unwrap();
    assert_eq!(empty_prefix.reference, Outcome::Returned(BTreeSet::new()));

    let Seed = [(0_i32,)];
    let empty_generator = check_refinement(
        || {
            let mut rows = BTreeSet::new();
            for _ in &Seed {
                for _ in Vec::<i32>::new() {
                    for y in partial_range(0) {
                        rows.insert(y);
                    }
                }
            }
            rows
        },
        || {
            mini_linq::compile! {
                pipeline { plan = cq_planners::order(written, coarse); }
                struct EmptyGenerator;
                relation Seed(unit: i32);
                relation answer(y: i32);
                answer(y) :- Seed(_), for _ in Vec::<i32>::new(), for y in partial_range(0);
            }
            .map(|(y,)| y.to_owned())
            .collect::<BTreeSet<_>>()
        },
    )
    .unwrap();
    assert_eq!(
        empty_generator.reference,
        Outcome::Returned(BTreeSet::new())
    );
}

#[test]
fn expression_and_iterator_panics_abort_the_whole_answer() {
    let Seed = [(0_i32,)];
    let expression = check_refinement(
        || {
            Seed.iter()
                .flat_map(|_| partial_range(0))
                .collect::<BTreeSet<_>>()
        },
        || {
            mini_linq::compile! {
                pipeline { plan = cq_planners::order(written, coarse); }
                struct PartialExpression;
                relation Seed(unit: i32);
                relation answer(x: i32);
                answer(x) :- Seed(_), for x in partial_range(0);
            }
            .map(|(x,)| x.to_owned())
            .collect::<BTreeSet<_>>()
        },
    )
    .unwrap();
    assert!(matches!(expression.reference, Outcome::Panicked(_)));
    assert!(matches!(expression.target, Outcome::Panicked(_)));

    let iterator = check_refinement(
        || {
            Seed.iter()
                .flat_map(|_| partial_iterator())
                .collect::<BTreeSet<_>>()
        },
        || {
            mini_linq::compile! {
                pipeline { plan = cq_planners::order(written, coarse); }
                struct PartialIterator;
                relation Seed(unit: i32);
                relation answer(x: i32);
                answer(x) :- Seed(_), for x in partial_iterator();
            }
            .map(|(x,)| x.to_owned())
            .collect::<BTreeSet<_>>()
        },
    )
    .unwrap();
    assert!(matches!(iterator.reference, Outcome::Panicked(_)));
    assert!(matches!(iterator.target, Outcome::Panicked(_)));
}

#[test]
fn later_relational_emptiness_may_refine_a_panicking_generator() {
    let R: [(i32,); 0] = [];
    let observation = check_refinement(
        || {
            let mut rows = BTreeSet::new();
            for x in partial_range(0) {
                for &(y,) in &R {
                    rows.insert((x, y));
                }
            }
            rows
        },
        || {
            mini_linq::compile! {
                pipeline { plan = cq_planners::order(hypergraph); }
                struct LaterEmpty;
                relation R(y: i32);
                relation answer(x: i32, y: i32);
                answer(x, y) :- for x in partial_range(0), R(y);
            }
            .map(|(x, y)| (x.to_owned(), y.to_owned()))
            .collect::<BTreeSet<_>>()
        },
    )
    .unwrap();
    // Bottom permits either outcome, not a required panic or evaluation count.
    assert!(matches!(observation.reference, Outcome::Panicked(_)));
}
