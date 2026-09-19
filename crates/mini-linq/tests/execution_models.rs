#![cfg(any(feature = "access-to-pipeline", feature = "access-to-set-loops"))]

#[cfg(all(
    feature = "access-to-pipeline",
    feature = "access-to-set-loops",
    feature = "pipeline-set"
))]
use std::cell::RefCell;
#[cfg(any(feature = "access-to-set-loops", feature = "pipeline-set"))]
use std::collections::HashSet;

#[cfg(all(
    feature = "access-to-pipeline",
    feature = "access-to-set-loops",
    feature = "pipeline-set"
))]
#[test]
fn push_and_fully_consumed_pull_agree_on_correlated_join_results_and_effect_order() {
    let events = RefCell::new(Vec::new());
    let observed = &events;
    macro_rules! region {
        ($execution:ident) => {
            mini_linq::$execution! {
                out(x, y) => ({ observed.borrow_mut().push((2, x, y)); (x % 2, y) }) :-
                R(x) => bind (x,) <- ([(1,), (1,), (2,), (3,)]),
                S(x, y) => bind (y,) <- ({
                    observed.borrow_mut().push((0, x, 0));
                    (0..x).map(|y| (y,))
                }),
                Keep(x, y) => if ({ observed.borrow_mut().push((1, x, y)); y % 2 == 0 });
            }
        };
    }
    let pushed = region!(access_to_set_loops);
    let push_events = events.replace(Vec::new());
    let collected = region!(access_to_set);
    let pull_events = events.replace(Vec::new());
    assert_eq!(pushed, HashSet::from([(1, 0), (0, 0), (1, 2)]));
    assert_eq!(pushed, collected);
    assert_eq!(push_events, pull_events);
}

#[cfg(all(
    feature = "access-to-pipeline",
    feature = "access-to-set-loops",
    feature = "pipeline-distinct-iter"
))]
#[test]
fn eager_push_and_partial_pull_have_different_observation_contracts() {
    let calls = std::cell::Cell::new(0);
    let observed = &calls;
    let mut pulled = mini_linq::access_to_distinct_iter! {
        out(x) => (x) :-
        R(x) => bind (x,) <- ([(1,), (2,), (3,)]),
        Keep(x) => if ({ observed.set(observed.get() + 1); true });
    };
    assert_eq!(calls.get(), 0);
    assert_eq!(pulled.next(), Some(1));
    assert_eq!(calls.get(), 1);
    drop(pulled);
    calls.set(0);
    let pushed = mini_linq::access_to_set_loops! {
        out(x) => (x) :-
        R(x) => bind (x,) <- ([(1,), (2,), (3,)]),
        Keep(x) => if ({ observed.set(observed.get() + 1); true });
    };
    assert_eq!(calls.get(), 3);
    assert_eq!(pushed, HashSet::from([1, 2, 3]));
}

#[cfg(feature = "access-to-set-loops")]
#[test]
fn push_moves_owned_outputs_and_preserves_outer_values_for_multiple_witnesses() {
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct Output(String, i32);
    let pushed = mini_linq::access_to_set_loops! {
        out(x, y) => (Output(x, y)) :-
        R(x) => bind (x,) <- ([(String::from("a"),)]),
        S(x, y) => bind (y,) <- ([(1,), (1,), (2,)]);
    };
    assert_eq!(
        pushed,
        HashSet::from([Output("a".into(), 1), Output("a".into(), 2)])
    );
    assert_eq!(
        mini_linq::access_to_set_loops! { out() => (()) :- Unit() => if (true); },
        HashSet::from([()]),
    );
    assert!(mini_linq::access_to_set_loops! { out() => (()) :- Unit() => if (false); }.is_empty());
}

#[cfg(all(feature = "pipeline-distinct-iter", feature = "pipeline-set"))]
#[test]
fn execution_endpoints_accept_binding_retention_and_early_projection() {
    macro_rules! pipeline {
        ($execution:ident) => {
            mini_linq::$execution! {
                source = scan (x, _ignored,) in ([(1, 10), (2, 20), (2, 30)]) yield (x,);
                doubled = project source as (value,) yield ((value * 2,));
                accepted = filter doubled as (key,) if (key > 2);
                witnesses = join accepted as (key,) with (y, _unused,) in ([(key + 1, 0), (key + 2, 1)]) yield (y,);
                output = project witnesses as (result,) yield (result);
                unique = distinct output;
                return unique;
            }
        };
    }
    let collected = pipeline!(pipeline_to_set);
    let pulled = pipeline!(pipeline_to_distinct_iter).collect::<HashSet<_>>();
    assert_eq!(collected, HashSet::from([5, 6]));
    assert_eq!(collected, pulled);
}

#[cfg(feature = "pipeline-set")]
#[test]
fn dropping_an_enclosing_binding_does_not_require_cloning_it() {
    struct NonClone(i32);
    let rows = mini_linq::pipeline_to_set! {
        source = scan (value,) in ([(NonClone(4),)]) yield (value,);
        witnesses = join source as (value,) with (x,) in ([(value.0 + 1,), (value.0 + 2,)]) yield (x,);
        unique = distinct witnesses;
        return unique;
    };
    assert_eq!(rows, HashSet::from([(5,), (6,)]));
}

#[cfg(feature = "access-to-set-loops")]
#[test]
fn push_does_not_clone_an_outer_value_used_only_to_select_inner_rows() {
    struct NonClone(i32);
    let rows = mini_linq::access_to_set_loops! {
        out(y) => (y) :-
        R(x) => bind (x,) <- ([(NonClone(4),)]),
        S(x, y) => bind (y,) <- ([(x.0 + 1,), (x.0 + 2,)]);
    };
    assert_eq!(rows, HashSet::from([5, 6]));
}
