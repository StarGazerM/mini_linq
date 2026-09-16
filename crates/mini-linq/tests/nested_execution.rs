//! Whole-RA execution: the planner hands the same RA to independent realizations.
use std::{
    cell::Cell,
    collections::{BTreeSet, HashSet},
};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Key(String);

#[derive(Debug, PartialEq, Eq, Hash)]
struct Payload(String);

macro_rules! star {
    () => {
        mini_linq::graph_query! {
            pub struct Star;
            relation Facts(key: Key, payload: Payload, value: i32);
            relation Tags(key: Key, tag: String);
            relation Allowed(key: Key);
            answer(value, tag) :- Facts(key, payload, value), Tags(key, tag), Allowed(key);
        }
    };
}

mod pull {
    use super::{Key, Payload};
    macro_rules! realize {
        ($($ra:tt)*) => { ::mini_linq::nested_pull! {
            inputs {
                Facts => ::mini_linq::tuple_hash_rows;
                Tags => ::mini_linq::tuple_hash_rows;
                Allowed => ::mini_linq::tuple_sorted_rows;
            }
            $($ra)*
        } };
    }
    use realize as choose_indexes;
    mod routing {
        pub(super) use super::choose_indexes;
        pub use ::mini_linq::graph_query;
    }
    use routing as mini_linq;
    star!();
}

mod push {
    use super::{Key, Payload};
    macro_rules! realize {
        ($($ra:tt)*) => { ::mini_linq::nested_push! {
            inputs {
                Facts => ::mini_linq::tuple_hash_rows;
                Tags => ::mini_linq::tuple_hash_rows;
                Allowed => ::mini_linq::tuple_sorted_rows;
            }
            $($ra)*
        } };
    }
    use realize as choose_indexes;
    mod routing {
        pub(super) use super::choose_indexes;
        pub use ::mini_linq::graph_query;
    }
    use routing as mini_linq;
    star!();
}

#[test]
fn ya_star_matches_oracle_with_borrowed_nonclone_inputs() {
    // All subsets cover empty inputs, absent keys, multiple witnesses, and
    // duplicate answer values from different fact rows.
    for facts_mask in 0..16 {
        for tags_mask in 0..8 {
            for allowed_mask in 0..4 {
                let facts: HashSet<_> = [
                    ("a", "one", 7),
                    ("a", "two", 7),
                    ("b", "three", 9),
                    ("c", "four", 11),
                ]
                .into_iter()
                .enumerate()
                .filter(|(i, _)| facts_mask & (1 << i) != 0)
                .map(|(_, (key, payload, value))| (Key(key.into()), Payload(payload.into()), value))
                .collect();
                let tags: HashSet<_> = [("a", "red"), ("a", "blue"), ("b", "green")]
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| tags_mask & (1 << i) != 0)
                    .map(|(_, (key, tag))| (Key(key.into()), tag.to_owned()))
                    .collect();
                let allowed: BTreeSet<_> = ["a", "b"]
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| allowed_mask & (1 << i) != 0)
                    .map(|(_, key)| (Key(key.into()),))
                    .collect();
                let expected: HashSet<_> = facts
                    .iter()
                    .flat_map(|fact| {
                        tags.iter().filter_map(|tag| {
                            (fact.0 == tag.0 && allowed.iter().any(|key| key.0 == fact.0))
                                .then_some((&fact.2, &tag.1))
                        })
                    })
                    .collect();
                let pulled: Vec<_> = pull::Star::iter(&facts, &tags, &allowed).collect();
                let mut pushed = Vec::new();
                push::Star::for_each(&facts, &tags, &allowed, |row| pushed.push(row));
                assert_eq!(pulled.len(), expected.len());
                assert_eq!(pushed.len(), expected.len());
                assert_eq!(pulled.into_iter().collect::<HashSet<_>>(), expected);
                assert_eq!(pushed.into_iter().collect::<HashSet<_>>(), expected);
            }
        }
    }
}

// A leaf adapter can expose only the demanded native operations. Vec is used
// intentionally here to test duplicate enumeration, including after projection.
macro_rules! rows {
    (type ($($ty:ty,)*)) => { Vec<($($ty,)*)> };
    (scan ($value:expr)) => { ($value).iter() };
    (field ($value:expr) ($column:tt)) => { ($value).$column };
}

macro_rules! shared_plan {
    ($realizer:path, $name:ident) => {
        $realizer! {
            inputs { R => rows; }
            struct $name;
            relation R(key: i32, value: String);
            answer(key) :- R(key, value);
            relational {
                source = rename R {key -> key, value -> value};
                keys = project source keep {key};
                witnesses = natural_join keys with source;
                shared = semijoin witnesses with keys;
                identity = unit;
                present = project keys keep {};
                seed = natural_join present with identity;
                result = natural_join shared with seed;
                projected = project result keep {key};
                output projected as answer(key);
            }
        }
    };
}
shared_plan!(mini_linq::nested_pull, SharedPull);
shared_plan!(mini_linq::nested_push, SharedPush);

#[test]
fn projection_and_shared_dag_have_set_semantics() {
    let source = vec![
        (1, "first".into()),
        (1, "second".into()),
        (1, "first".into()),
        (2, "third".into()),
    ];
    let pulled: Vec<_> = SharedPull::iter(&source).collect();
    let mut pushed = Vec::new();
    SharedPush::for_each(&source, |row| pushed.push(row));
    assert_eq!(pulled, vec![(&1,), (&2,)]);
    assert_eq!(pushed, pulled);
    assert!(SharedPull::iter(&Vec::new()).next().is_none());
}

thread_local! {
    static VISITS: Cell<usize> = const { Cell::new(0) };
    static SCANS: Cell<usize> = const { Cell::new(0) };
}
macro_rules! measured_rows {
    (scan ($value:expr)) => {{
        SCANS.with(|n| n.set(n.get() + 1));
        ($value).iter().inspect(|_| VISITS.with(|n| n.set(n.get() + 1)))
    }};
    ($($op:tt)*) => { rows! { $($op)* } };
}
mini_linq::nested_pull! {
    inputs { R => measured_rows; }
    struct Demand;
    relation R(key: i32);
    answer(key) :- R(key);
    relational { r = rename R {key -> key}; output r as answer(key); }
}

#[test]
fn pull_does_no_traversal_until_demanded_and_stops_on_drop() {
    let input = vec![(1,), (1,), (2,), (3,)];
    VISITS.with(|n| n.set(0));
    SCANS.with(|n| n.set(0));
    let mut output = Demand::iter(&input);
    VISITS.with(|n| assert_eq!(n.get(), 0));
    SCANS.with(|n| assert_eq!(n.get(), 0));
    assert_eq!(output.next(), Some((&1,)));
    VISITS.with(|n| assert_eq!(n.get(), 1));
    assert_eq!(output.next(), Some((&2,)));
    VISITS.with(|n| assert_eq!(n.get(), 3));
    drop(output);
    VISITS.with(|n| assert_eq!(n.get(), 3));
}

macro_rules! borrowed_output {
    ($realizer:path, $name:ident) => {
        $realizer! {
            inputs { R => rows; }
            struct $name;
            relation R(payload: Payload);
            answer(payload) :- R(payload);
            relational { r = rename R {payload -> payload}; output r as answer(payload); }
        }
    };
}
borrowed_output!(mini_linq::nested_pull, BorrowPull);
borrowed_output!(mini_linq::nested_push, BorrowPush);

#[test]
fn output_itself_can_be_nonclone() {
    let source = vec![(Payload("borrowed".into()),)];
    let pulled = BorrowPull::iter(&source).next().unwrap();
    assert!(std::ptr::eq(pulled.0, &source[0].0));
    BorrowPush::for_each(&source, |row| assert!(std::ptr::eq(row.0, &source[0].0)));
}

macro_rules! existential_plan {
    ($realizer:path, $name:ident) => {
        $realizer! {
            inputs { R => measured_rows; S => measured_rows; }
            struct $name;
            relation R(x: i32);
            relation S(x: i32);
            answer(x) :- R(x), S(x);
            relational {
                r = rename R {x -> x};
                s = rename S {x -> x};
                kept = semijoin r with s;
                output kept as answer(x);
            }
        }
    };
}
existential_plan!(mini_linq::nested_pull, ExistsPull);
existential_plan!(mini_linq::nested_push, ExistsPush);

#[test]
fn semijoin_short_circuits_witnesses() {
    let r = vec![(1,), (2,)];
    let s = vec![(1,), (2,), (3,)];
    VISITS.with(|n| n.set(0));
    let mut output = ExistsPull::iter(&r, &s);
    assert_eq!(output.next(), Some((&1,)));
    VISITS.with(|n| assert_eq!(n.get(), 2));
    assert_eq!(output.next(), Some((&2,)));
    assert_eq!(output.next(), None);
    VISITS.with(|n| assert_eq!(n.get(), 5));
    VISITS.with(|n| n.set(0));
    let mut pushed = Vec::new();
    ExistsPush::for_each(&r, &s, |row| pushed.push(row));
    assert_eq!(pushed, vec![(&1,), (&2,)]);
    VISITS.with(|n| assert_eq!(n.get(), 5));
    assert!(ExistsPull::iter(&r, &vec![]).next().is_none());
}

macro_rules! disjoint_plan {
    ($realizer:path, $name:ident) => {
        $realizer! {
            inputs { R => rows; S => rows; }
            struct $name;
            relation R(x: i32);
            relation S(y: i32);
            answer(x, x) :- R(x), S(y);
            relational {
                r = rename R {x -> x};
                s = rename S {y -> y};
                kept = semijoin r with s;
                output kept as answer(x, x);
            }
        }
    };
}
disjoint_plan!(mini_linq::nested_pull, DisjointPull);
disjoint_plan!(mini_linq::nested_push, DisjointPush);

#[test]
fn disjoint_semijoin_is_existence_and_repeated_output_borrows() {
    let r = vec![(1,), (1,), (2,)];
    for s in [vec![], vec![(8,), (9,)]] {
        let expected = if s.is_empty() {
            vec![]
        } else {
            vec![(&1, &1), (&2, &2)]
        };
        assert_eq!(DisjointPull::iter(&r, &s).collect::<Vec<_>>(), expected);
        let mut pushed = Vec::new();
        DisjointPush::for_each(&r, &s, |row| pushed.push(row));
        assert_eq!(pushed, expected);
    }
}
