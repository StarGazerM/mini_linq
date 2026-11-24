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
