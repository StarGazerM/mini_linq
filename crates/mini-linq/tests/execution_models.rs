#![cfg(any(feature = "pull", feature = "push-set"))]

#[cfg(all(feature = "pull", feature = "push-set", feature = "collect-pipeline"))]
use std::cell::RefCell;
use std::collections::HashSet;

#[cfg(all(feature = "pull", feature = "push-set", feature = "collect-pipeline"))]
#[test]
fn push_and_fully_consumed_pull_agree_on_correlated_join_results_and_effect_order() {
    let events = RefCell::new(Vec::new());
    let observed = &events;
    macro_rules! region {
        ($execution:ident) => {
            mini_linq::$execution! {
                out(x, y) => ({ observed.borrow_mut().push((2, x, y)); (x % 2, y) }) :-
                R(x) => for (x,) in ([(1,), (1,), (2,), (3,)]),
                S(x, y) => for (y,) in ({
                    observed.borrow_mut().push((0, x, 0));
                    (0..x).map(|y| (y,))
                }),
                Keep(x, y) => if ({ observed.borrow_mut().push((1, x, y)); y % 2 == 0 });
            }
        };
    }
    let pushed = region!(push_set);
    let push_events = events.replace(Vec::new());
    let collected = region!(pull_set);
    let pull_events = events.replace(Vec::new());
    assert_eq!(pushed, HashSet::from([(1, 0), (0, 0), (1, 2)]));
    assert_eq!(pushed, collected);
    assert_eq!(push_events, pull_events);
}

#[cfg(all(feature = "pull", feature = "push-set", feature = "iterator-pipeline"))]
#[test]
fn eager_push_and_partial_pull_have_different_observation_contracts() {
    let calls = std::cell::Cell::new(0);
    let observed = &calls;
    let mut pulled = mini_linq::pull! {
        out(x) => (x) :-
        R(x) => for (x,) in ([(1,), (2,), (3,)]),
        Keep(x) => if ({ observed.set(observed.get() + 1); true });
    };
    assert_eq!(calls.get(), 0);
    assert_eq!(pulled.next(), Some(1));
    assert_eq!(calls.get(), 1);
    drop(pulled);
    calls.set(0);
    let pushed = mini_linq::push_set! {
        out(x) => (x) :-
        R(x) => for (x,) in ([(1,), (2,), (3,)]),
        Keep(x) => if ({ observed.set(observed.get() + 1); true });
    };
    assert_eq!(calls.get(), 3);
    assert_eq!(pushed, HashSet::from([1, 2, 3]));
}

#[cfg(feature = "push-set")]
#[test]
fn push_moves_owned_outputs_and_preserves_outer_values_for_multiple_witnesses() {
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct Output(String, i32);
    let pushed = mini_linq::push_set! {
        out(x, y) => (Output(x, y)) :-
        R(x) => for (x,) in ([(String::from("a"),)]),
        S(x, y) => for (y,) in ([(1,), (1,), (2,)]);
    };
    assert_eq!(
        pushed,
        HashSet::from([Output("a".into(), 1), Output("a".into(), 2)])
    );
    assert_eq!(
        mini_linq::push_set! { out() => (()) :- Unit() => if (true); },
        HashSet::from([()]),
    );
    assert!(mini_linq::push_set! { out() => (()) :- Unit() => if (false); }.is_empty());
}

#[cfg(all(feature = "iterator-pipeline", feature = "collect-pipeline"))]
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
    let collected = pipeline!(collect_pipeline);
    let pulled = pipeline!(iterator_pipeline).collect::<HashSet<_>>();
    assert_eq!(collected, HashSet::from([5, 6]));
    assert_eq!(collected, pulled);
}

#[cfg(feature = "collect-pipeline")]
#[test]
fn dropping_an_enclosing_binding_does_not_require_cloning_it() {
    struct NonClone(i32);
    let rows = mini_linq::collect_pipeline! {
        source = scan (value,) in ([(NonClone(4),)]) yield (value,);
        witnesses = join source as (value,) with (x,) in ([(value.0 + 1,), (value.0 + 2,)]) yield (x,);
        unique = distinct witnesses;
        return unique;
    };
    assert_eq!(rows, HashSet::from([(5,), (6,)]));
}

#[cfg(feature = "push-set")]
#[test]
fn push_does_not_clone_an_outer_value_used_only_to_select_inner_rows() {
    struct NonClone(i32);
    let rows = mini_linq::push_set! {
        out(y) => (y) :-
        R(x) => for (x,) in ([(NonClone(4),)]),
        S(x, y) => for (y,) in ([(x.0 + 1,), (x.0 + 2,)]);
    };
    assert_eq!(rows, HashSet::from([5, 6]));
}
