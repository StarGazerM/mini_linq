use cq_testkit::harness::{Outcome, check_refinement};
use std::collections::BTreeSet;

#[test]
fn explicit_physical_and_trace_queues_execute_like_the_default() {
    let rows = [(1,), (1,), (2,)];
    let default: BTreeSet<_> = mini_linq::compile! {
        pipeline {}
        struct Q; relation R(x: i32) = (rows);
        answer(x) :- R(x);
    }
    .collect();
    let from_physical: BTreeSet<_> = stacked_ir::enter! {
        pipeline { stacked_ir::leave, mini_linq::passes::select, mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
        physical {
            relation R(x: i32) = (rows);
            r = pipe rename R@0 {x -> x};
            out = pipe canon project r keep {x};
            return out as answer(x);
        }
    }
    .collect();
    let from_trace: BTreeSet<_> = stacked_ir::leave! {
        pipeline { mini_linq::passes::select, mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
        physical {
            relation R(x: i32) = (rows);
            r = pipe rename R@0 {x -> x};
            out = pipe canon project r keep {x};
            return out as answer(x);
        }
        trace {}
    }
    .collect();
    assert_eq!(from_physical, default);
    assert_eq!(from_trace, default);
}

#[test]
fn composite_rewrites_can_repeat_without_choosing_the_continuation() {
    let rows = [(1,), (1,), (2,)];
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline { physical = [stacked_ir::round_trip, stacked_ir::round_trip]; }
        struct Q; relation R(x: i32) = (rows);
        answer(x) :- R(x);
    }
    .collect();
    assert_eq!(result, BTreeSet::from([(&1,), (&2,)]));
}

#[test]
fn rust_steps_refine_the_written_order_on_returning_and_bottom_inputs() {
    fn first(value: &i32) -> i32 {
        assert_ne!(*value, 0);
        12 / value
    }
    fn second(value: &i32) -> bool {
        assert_ne!(*value, -1);
        *value > 0
    }
    for rows in [
        vec![(1,), (2,), (2,), (-2,)],
        vec![(0,)],
        vec![(-1,)],
        vec![],
    ] {
        let observations = check_refinement(
            || {
                rows.iter()
                    .filter_map(|(x,)| {
                        let y = first(x);
                        second(x).then_some((*x, y))
                    })
                    .collect::<BTreeSet<_>>()
            },
            || {
                mini_linq::compile! {
                    pipeline { physical = [stacked_ir::round_trip]; }
                    struct Q; relation R(x: i32) = (&rows);
                    answer(x, y) :- R(x), let y: i32 = first(x), if second(x);
                }
                .map(|(x, y)| (*x, y))
                .collect::<BTreeSet<_>>()
            },
        )
        .unwrap();
        // This identity round trip also preserves the default chain's panics.
        if matches!(observations.reference, Outcome::Panicked(_)) {
            assert!(matches!(observations.target, Outcome::Panicked(_)));
        }
    }
}
