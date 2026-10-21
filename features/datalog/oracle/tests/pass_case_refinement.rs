//! Executable companions to the three rewrite PassCase fixtures in test-passes.
//! These use the same authored programs, execute the actual rewrite macros, and
//! compare native typed answers rather than treating token equality as semantics.
use cq_datalog_testkit::check_refinement;

#[test]
fn source_duplicate_fixture_executes_before_and_after_rewrite() {
    for facts in [vec![], vec![(1_i32,)], vec![(1,), (1,), (2,)]] {
        let rows = &facts;
        check_refinement(
            || {
                cq_datalog_test_passes::eval_source! {
                    struct Q; relation E(x:i32) = (rows); relation R(x:i32); output R;
                    R(x) :- E(x); R(x) :- E(x);
                }
            },
            || {
                cq_datalog_test_passes::deduplicate_source! {
                    pipeline { cq_datalog::passes::stratify,
                        cq_datalog::passes::incremental, cq_datalog::passes::storage,
                        cq_datalog::passes::fork, cq_datalog_test_passes::eval_round };
                    struct Q; relation E(x:i32) = (rows); relation R(x:i32); output R;
                    R(x) :- E(x); R(x) :- E(x);
                }
            },
        )
        .unwrap();
    }
}

#[test]
fn strata_merge_fixture_executes_recorded_components() {
    for facts in [vec![], vec![(1_i32,)], vec![(1,), (1,), (2,)]] {
        let rows = &facts;
        macro_rules! evaluate {
            ($($entry:ident)::+; $($pipeline:tt)*) => {
                $($entry)::+! {
                    pipeline { $($pipeline)* };
                    struct Q; relation E(x:i32) = (rows);
                    relation A(x:i32); relation B(x:i32); relation C(x:i32); output A, B, C;
                    A(x) :- E(x); B(x) :- E(x); C(x) :- E(x), !A(x);
                    strata { component [A]; component [B]; component [C]; }
                }
            };
        }
        check_refinement(
            || {
                evaluate!(cq_datalog::passes::incremental;
                cq_datalog::passes::storage, cq_datalog::passes::fork,
                cq_datalog_test_passes::eval_strata_from_round)
            },
            || {
                evaluate!(cq_datalog_test_passes::merge_strata;
                cq_datalog::passes::incremental, cq_datalog::passes::storage,
                cq_datalog::passes::fork, cq_datalog_test_passes::eval_round)
            },
        )
        .unwrap();
    }
}

#[test]
fn storage_alternative_fixture_executes_rebound_column_order() {
    for facts in [vec![], vec![(1_i32, 2_i32)], vec![(1, 2), (1, 2), (3, 4)]] {
        let rows = &facts;
        macro_rules! evaluate {
            ($($configuration:tt)*) => {
                cq_datalog::datalog! {
                    pipeline { $($configuration)* }
                    struct Q; relation E(x:i32,y:i32) = (rows); relation R(x:i32,y:i32); output R;
                    R(x,y) :- E(x,y);
                }
            };
        }
        check_refinement(
            || evaluate! { print = cq_datalog_test_passes::eval_incremental_from_round; },
            || {
                evaluate! {
                    storage_rewrites = [cq_datalog_test_passes::alternate_storage];
                    print = cq_datalog_test_passes::eval_round;
                }
            },
        )
        .unwrap();
    }
}
