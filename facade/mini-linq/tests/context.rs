//! Stacked compilers supply facts through lower/select argument packets.
use std::collections::HashSet;

#[test]
fn canonical_consumer_owns_the_lowered_return_boundary() {
    let rows = [(1,), (1,), (2,)];
    let visits = std::cell::Cell::new(0);
    let answer: HashSet<_> = mini_linq::compile! {
        pipeline { lower = mini_linq::passes::lower(consumer = canonical); }
        struct Q; relation rows(x:i32); answer(x) :- rows(x);
    }
    .inspect(|_| visits.set(visits.get() + 1))
    .collect();
    assert_eq!(answer, HashSet::from([(&1,), (&2,)]));
    assert_eq!(visits.get(), 3);

    let ordinary: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation rows(x:i32); answer(x) :- rows(x);
    }
    .collect();
    assert_eq!(ordinary, [(&1,), (&2,)]);
}

#[test]
fn selection_records_consumer_and_discharged_host_facts() {
    let rows = [(1,), (2,), (3,)];
    let visits = std::cell::Cell::new(0);
    let answer: HashSet<_> = mini_linq::passes::select! {
        arguments { consumer = canonical, host_set rows discharged };
        pipeline { mini_linq::passes::print_pull };
        physical {
            relation rows(x:i32);
            r = pipe rename rows@0 {x -> x};
            return r as answer(*x % 2);
        }
    }
    .inspect(|_| visits.set(visits.get() + 1))
    .collect();
    assert_eq!(answer, HashSet::from([(0,), (1,)]));
    assert_eq!(visits.get(), 3);
}

#[test]
fn discharged_host_fact_survives_the_default_pipeline() {
    let rows = [(1,), (2,)];
    let answer: Vec<_> = mini_linq::compile! {
        pipeline { lower = mini_linq::passes::lower(host_set rows discharged); }
        struct Q; relation rows(x:i32); answer(x) :- rows(x);
    }
    .collect();
    assert_eq!(answer, [(&1,), (&2,)]);
}

#[test]
#[should_panic(expected = "host_set")]
fn certified_host_fact_still_checks_duplicate_rows() {
    let rows = [(1,), (1,)];
    let _: Vec<_> = mini_linq::passes::select! {
        pipeline { mini_linq::passes::print_pull };
        physical {
            relation rows(x:i32);
            host_set rows;
            r = pipe rename rows@0 {x -> x};
            return r as answer(x);
        }
    }
    .collect();
}
