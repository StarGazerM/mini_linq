//! An external theta surface enters the public pipeline at IX lowering.
//!
//! `ra-theta` constructs a complete IX program directly, without planning CQ.
//! The build script quotes `theta_passes::lower!` with an explicit queue ending
//! in selection and printing. The second route inserts a checked Physical
//! rewrite that records `merge [k]`; its hosts are ordinary ordered maps.
//! Each core dependency is declared with a version and a local development path.
//! Tests use the public cq-testkit oracle and executable refinement harness.
//!
//! The meaning is `T X = bottom + Set X`, with one abort and an opaque partial
//! Rust guard after the relational join. Other effects and evaluation counts
//! are outside this meaning.
//! From the repository root, run `python3 features/diverged-ra/check.py` to
//! validate both routes, or `cargo test --manifest-path
//! features/diverged-ra/Cargo.toml --locked` for the workspace tests.
include!(concat!(env!("OUT_DIR"), "/query.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use cq_testkit::{Database, Rows, harness, semantics};
    use std::collections::BTreeSet;
    fn oracle(left: &[(i32, i32)], right: &[(i32, i32)]) -> BTreeSet<(i32, i32)> {
        left.iter()
            .flat_map(|&(k, x)| {
                right
                    .iter()
                    .filter_map(move |&(l, y)| (k == l && x < y).then_some((x, y)))
            })
            .collect()
    }

    fn database(left: &[(i32, i32)], right: &[(i32, i32)]) -> Database {
        [("L", left), ("R", right)]
            .into_iter()
            .map(|(name, rows)| {
                (
                    name.to_owned(),
                    rows.iter().map(|&(k, v)| vec![k, v]).collect(),
                )
            })
            .collect()
    }
    #[test]
    fn theta_join_executes_through_both_unmodified_physical_routes() {
        let left = [(1, 2), (1, 2), (1, 9), (2, 3), (3, 0)];
        let right = [(1, 5), (1, 5), (1, 8), (2, 1), (2, 7), (4, 9)];
        for run in [run_probe, run_merge] {
            for (l, r) in [
                (&left[..], &right[..]),
                (&[][..], &right[..]),
                (&left[..], &[][..]),
            ] {
                let rows = run(l, r);
                let set: BTreeSet<_> = rows.iter().copied().collect();
                assert_eq!(rows.len(), set.len(), "returned rows must be distinct");
                assert_eq!(set, oracle(l, r));
            }
        }
    }
    #[test]
    fn small_inputs_match_the_nested_loop_oracle() {
        let indexes = syn::parse_str::<ra_theta::Query>(include_str!("../query.theta"))
            .unwrap()
            .indexes()
            .unwrap();
        let domain = [(0, 0), (0, 1), (1, 0), (1, 2)];
        for l in 0..16 {
            for r in 0..16 {
                let left: Vec<_> = domain
                    .iter()
                    .enumerate()
                    .filter_map(|(i, row)| (l & (1 << i) != 0).then_some(*row))
                    .collect();
                let right: Vec<_> = domain
                    .iter()
                    .enumerate()
                    .filter_map(|(i, row)| (r & (1 << i) != 0).then_some(*row))
                    .collect();
                for run in [run_probe, run_merge] {
                    assert_eq!(
                        run(&left, &right).into_iter().collect::<BTreeSet<_>>(),
                        oracle(&left, &right)
                    );
                    let actual: Rows = run(&left, &right)
                        .into_iter()
                        .map(|(x, y)| vec![x, y])
                        .collect();
                    assert_eq!(
                        actual,
                        semantics::evaluate_ra(&indexes.ra, &database(&left, &right)).output
                    );
                }
            }
        }
    }

    #[test]
    fn opaque_partial_guard_refines_written_order() {
        fn partial(x: &i32, y: &i32) -> bool {
            assert_ne!(*x, 9, "one undistinguished bottom");
            x < y
        }
        type Input<'a> = &'a [(i32, i32)];
        let cases: &[(Input<'_>, Input<'_>)] = &[
            (&[(1, 2), (1, 2)], &[(1, 5), (1, 5)]),
            (&[(1, 9)], &[(2, 5)]),
            (&[(1, 9)], &[(1, 5)]),
            (&[], &[(1, 5)]),
            (&[(1, 9)], &[]),
        ];
        for &(left, right) in cases {
            for run in [run_partial_probe, run_partial_merge] {
                harness::check_refinement(
                    || {
                        let mut answer = BTreeSet::new();
                        for &(k, x) in left {
                            for &(l, y) in right {
                                if k == l && partial(&x, &y) {
                                    answer.insert((x, y));
                                }
                            }
                        }
                        answer
                    },
                    || {
                        run(left, right, partial)
                            .into_iter()
                            .collect::<BTreeSet<_>>()
                    },
                )
                .unwrap();
            }
        }
    }
}
