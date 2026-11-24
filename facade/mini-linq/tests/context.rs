//! Stacked compilers supply facts through lower/select argument packets.
use std::collections::HashSet;

#[test]
fn canonical_consumer_owns_the_lowered_return_boundary() {
    let rows = [(1,), (2,), (3,)];
    let visits = std::cell::Cell::new(0);
    let answer: HashSet<_> = mini_linq::compile! {
        pipeline { lower = mini_linq::passes::lower(consumer = canonical); }
        struct Q; relation rows(x:i32); relation answer(x:i32);
        answer(*x % 2) :- rows(x);
    }
    .inspect(|_| visits.set(visits.get() + 1))
    .collect();
    assert_eq!(answer, HashSet::from([(0,), (1,)]));
    assert_eq!(visits.get(), 3);

    // The ordinary route deduplicates itself: two answers from three rows.
    // An answer is a set; its order is not a contract.
    let mut ordinary: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation rows(x:i32); relation answer(x:i32);
        answer(*x % 2) :- rows(x);
    }
    .collect();
    ordinary.sort();
    assert_eq!(ordinary, [(0,), (1,)]);
}

#[test]
fn selection_records_canonical_consumer_context() {
    let rows = [(1,), (2,), (3,)];
    let visits = std::cell::Cell::new(0);
    let answer: HashSet<_> = mini_linq::passes::select! {
        arguments { consumer = canonical };
        pipeline { mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
        physical {
            relation rows(x:i32);
            r = pipe rename rows {x -> x};
            return dedup r as answer(*x % 2);
        }
    }
    .inspect(|_| visits.set(visits.get() + 1))
    .collect();
    assert_eq!(answer, HashSet::from([(0,), (1,)]));
    assert_eq!(visits.get(), 3);
}

