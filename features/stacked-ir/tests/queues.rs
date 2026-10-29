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
fn total_rust_steps_preserve_answers_after_their_free_variable_binders() {
    fn first(value: &i32) -> i32 {
        value.saturating_add(12)
    }
    fn second(value: &i32) -> bool {
        *value > 0
    }
    for rows in [
        vec![(1,), (2,), (2,), (-2,)],
        vec![(0,)],
        vec![(-1,)],
        vec![],
    ] {
        let expected = rows
            .iter()
            .filter_map(|(x,)| {
                let y = first(x);
                second(x).then_some((*x, y))
            })
            .collect::<BTreeSet<_>>();
        // Both Rust steps need x from R, but neither depends on the other.
        let actual = mini_linq::compile! {
            pipeline { physical = [stacked_ir::round_trip]; }
            struct Q; relation R(x: i32) = (&rows);
            answer(x, y) :- R(x), let y: i32 = first(x), if second(x);
        }
        .map(|(x, y)| (*x, y))
        .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
        let reordered = mini_linq::compile! {
            pipeline { physical = [stacked_ir::round_trip]; }
            struct Reordered; relation R(x: i32) = (&rows);
            answer(x, y) :- R(x), if second(x), let y: i32 = first(x);
        }
        .map(|(x, y)| (*x, y))
        .collect::<BTreeSet<_>>();
        assert_eq!(reordered, expected);
    }
}
