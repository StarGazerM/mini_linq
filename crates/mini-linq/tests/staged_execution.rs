//! Borrowed execution through index, realization, Access, Pipeline and Native APIs.
use std::{
    cell::Cell,
    collections::{BTreeSet, HashSet},
};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Key(String);

#[derive(Debug, PartialEq, Eq, Hash)]
struct Payload(String);

mod borrowed {
    use super::{Key, Payload};
    mod routing {
        pub use ::cq_storage as storage;
        pub use ::mini_linq::{
            access_to_distinct_iter, access_to_exists, access_to_support_iter,
            cq_to_ra_hypergraph as query, lower_borrowed_nested as choose_relation_lowering,
            no_index_demands as choose_index_policy, pipeline_to_distinct_iter, pipeline_to_exists,
            pipeline_to_support_iter, replay_resource,
        };
        pub mod inputs {
            pub use ::mini_linq::{
                tuple_hash_rows as Facts, tuple_hash_rows as Tags, tuple_sorted_rows as Allowed,
            };
        }
    }
    use routing as mini_linq;
    mini_linq::query! {
        pub struct Star;
        relation Facts(key: Key, payload: Payload, value: i32);
        relation Tags(key: Key, tag: String);
        relation Allowed(key: Key);
        answer(value, tag) :- Facts(key, payload, value), Tags(key, tag), Allowed(key);
    }
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
                let pulled: Vec<_> = borrowed::Star::iter(&facts, &tags, &allowed).collect();
                assert_eq!(pulled.len(), expected.len());
                assert_eq!(pulled.into_iter().collect::<HashSet<_>>(), expected);
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

thread_local! {
    static VISITS: Cell<usize> = const { Cell::new(0) };
    static SCANS: Cell<usize> = const { Cell::new(0) };
    static FIELDS: Cell<usize> = const { Cell::new(0) };
}
macro_rules! measured_rows {
    (scan ($value:expr)) => {{
        SCANS.with(|n| n.set(n.get() + 1));
        ($value).iter().inspect(|_| VISITS.with(|n| n.set(n.get() + 1)))
    }};
    (field ($value:expr) ($column:tt)) => { *{
        FIELDS.with(|n| n.set(n.get() + 1));
        &($value).$column
    } };
    ($($op:tt)*) => { rows! { $($op)* } };
}

fn reset_measurements() {
    SCANS.with(|n| n.set(0));
    VISITS.with(|n| n.set(0));
    FIELDS.with(|n| n.set(0));
}

fn measurements() -> (usize, usize, usize) {
    (
        SCANS.with(Cell::get),
        VISITS.with(Cell::get),
        FIELDS.with(Cell::get),
    )
}

// Each input relation is a caller-resolved native-operation macro name.
mod routing {
    pub use ::cq_storage as storage;
    pub use ::mini_linq::{
        access_to_distinct_iter, access_to_exists, access_to_support_iter, lower_borrowed_nested,
        pipeline_to_distinct_iter, pipeline_to_exists, pipeline_to_support_iter, replay_resource,
    };
    pub(super) mod inputs {
        pub(crate) use measured_rows as Measured;
        pub(crate) use measured_rows as OtherMeasured;
        pub(crate) use rows as R;
        pub(crate) use rows as S;
    }
}
use routing as mini_linq;

macro_rules! shared_plan {
    ($realizer:path, $name:ident) => {
        $realizer! {
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
            } indexes {}
        }
    };
}
shared_plan!(mini_linq::lower_borrowed_nested, SharedPull);

#[test]
fn projection_and_shared_dag_have_set_semantics() {
    let source = vec![
        (1, "first".into()),
        (1, "second".into()),
        (1, "first".into()),
        (2, "third".into()),
    ];
    let pulled: Vec<_> = SharedPull::iter(&source).collect();
    assert_eq!(pulled, vec![(&1,), (&2,)]);
    assert!(SharedPull::iter(&Vec::new()).next().is_none());
}

mini_linq::lower_borrowed_nested! {
    struct Demand;
    relation Measured(key: i32);
    answer(key) :- Measured(key);
    relational { r = rename Measured {key -> key}; output r as answer(key); } indexes {}
}

#[test]
fn pull_does_no_traversal_until_demanded_and_stops_on_drop() {
    let input = vec![(1,), (1,), (2,), (3,)];
    reset_measurements();
    let mut output = Demand::iter(&input);
    assert_eq!(measurements(), (0, 0, 0));
    assert_eq!(output.next(), Some((&1,)));
    VISITS.with(|n| assert_eq!(n.get(), 1));
    assert_eq!(output.next(), Some((&2,)));
    VISITS.with(|n| assert_eq!(n.get(), 3));
    drop(output);
    VISITS.with(|n| assert_eq!(n.get(), 3));
}

mini_linq::lower_borrowed_nested! {
    struct JoinDemand;
    relation Measured(x: i32);
    relation OtherMeasured(x: i32, y: i32);
    answer(x, y) :- Measured(x), OtherMeasured(x, y);
    relational {
        r = rename Measured { x -> x };
        s = rename OtherMeasured { x -> x, y -> y };
        joined = natural_join r with s;
        output joined as answer(x, y);
    } indexes {}
}

#[test]
fn pull_join_preserves_inner_cursor_and_replays_for_next_left() {
    let left = vec![(1,), (2,)];
    let right = vec![(1, 10), (1, 11), (2, 20), (99, 0)];
    reset_measurements();
    let mut output = JoinDemand::iter(&left, &right);
    assert_eq!(measurements(), (0, 0, 0));

    // No collection is needed to trigger query execution. A request returns
    // before the remaining right rows are visited.
    assert_eq!(output.next(), Some((&1, &10)));
    SCANS.with(|n| assert_eq!(n.get(), 2));
    VISITS.with(|n| assert_eq!(n.get(), 2));
    assert_eq!(output.next(), Some((&1, &11)));
    SCANS.with(|n| assert_eq!(n.get(), 2));
    VISITS.with(|n| assert_eq!(n.get(), 3));

    // A new left row traverses the right prefix again without revisiting
    // its provider. Unmatched right rows are fetched only at the frontier.
    assert_eq!(output.next(), Some((&2, &20)));
    SCANS.with(|n| assert_eq!(n.get(), 2));
    VISITS.with(|n| assert_eq!(n.get(), 6));
    let before_drop = measurements();
    drop(output);
    assert_eq!(measurements(), before_drop);
}

#[test]
fn pull_consumers_execute_without_materializing_and_unused_branches_stay_unopened() {
    let left = vec![(1,), (2,)];
    let right = vec![(1, 10), (1, 11), (2, 20)];
    reset_measurements();
    let output = JoinDemand::iter(&left, &right);
    drop(output);
    assert_eq!(measurements(), (0, 0, 0));

    assert!(JoinDemand::iter(&left, &right).any(|(x, y)| (*x, *y) == (1, 10)));
    SCANS.with(|n| assert_eq!(n.get(), 2));
    VISITS.with(|n| assert_eq!(n.get(), 2));

    reset_measurements();
    let count = JoinDemand::iter(&left, &right).fold(0, |count, _| count + 1);
    assert_eq!(count, 3);
    SCANS.with(|n| assert_eq!(n.get(), 2));

    reset_measurements();
    assert_eq!(JoinDemand::iter(&Vec::new(), &right).next(), None);
    assert_eq!(measurements(), (1, 0, 0));
    reset_measurements();
    assert_eq!(ExistsPull::iter(&Vec::new(), &left).next(), None);
    assert_eq!(measurements(), (1, 0, 0));
}

mini_linq::lower_borrowed_nested! {
    struct SharedDemand;
    relation Measured(x: i32);
    answer(x) :- Measured(x);
    relational {
        r = rename Measured { x -> x };
        kept = semijoin r with r;
        output kept as answer(x);
    } indexes {}
}

#[test]
fn shared_ra_uses_have_independent_cursors_over_one_lazy_source() {
    let input = vec![(1,), (2,)];
    reset_measurements();
    let mut output = SharedDemand::iter(&input);
    assert_eq!(measurements(), (0, 0, 0));
    assert_eq!(output.next(), Some((&1,)));
    SCANS.with(|n| assert_eq!(n.get(), 1));
    VISITS.with(|n| assert_eq!(n.get(), 1));
    assert_eq!(output.next(), Some((&2,)));
    SCANS.with(|n| assert_eq!(n.get(), 1));
    VISITS.with(|n| assert_eq!(n.get(), 2));
    assert_eq!(output.next(), None);
}

mini_linq::lower_borrowed_nested! {
    struct SharedJoinDemand;
    relation Measured(x: i32);
    answer(x) :- Measured(x);
    relational {
        r = rename Measured { x -> x };
        joined = natural_join r with r;
        output joined as answer(x);
    } indexes {}
}

#[test]
fn shared_join_cursors_can_alternate_between_cached_rows_and_the_frontier() {
    let input = vec![(1,), (2,), (3,)];
    reset_measurements();
    let mut output = SharedJoinDemand::iter(&input);
    assert_eq!(measurements(), (0, 0, 0));
    assert_eq!(output.next(), Some((&1,)));
    assert_eq!(measurements(), (1, 1, 1));

    // The right cursor finishes the first left row's search, advancing the
    // source to exhaustion. The left cursor then resumes from its own position
    // inside the retained prefix, rather than skipping to the frontier.
    assert_eq!(output.next(), Some((&2,)));
    assert_eq!(measurements(), (1, 3, 3));
    assert_eq!(output.next(), Some((&3,)));
    assert_eq!(output.next(), None);
    assert_eq!(measurements(), (1, 3, 3));
}

// Only the common key is counted; final answer distinctness hashes Payload and
// i32, so its comparisons cannot hide repeated YA reduction work. The key
// deliberately implements neither Clone nor Hash.
#[derive(Debug)]
struct ComparedKey {
    value: i32,
    origin: usize,
}

thread_local! {
    static STAR_SCANS: Cell<[usize; 3]> = const { Cell::new([0; 3]) };
    static STAR_VISITS: Cell<[usize; 3]> = const { Cell::new([0; 3]) };
    static KEY_COMPARISONS: Cell<[[usize; 3]; 3]> = const { Cell::new([[0; 3]; 3]) };
}

impl PartialEq for ComparedKey {
    fn eq(&self, other: &Self) -> bool {
        KEY_COMPARISONS.with(|counts| {
            let mut next = counts.get();
            next[self.origin][other.origin] += 1;
            counts.set(next);
        });
        self.value == other.value
    }
}
impl Eq for ComparedKey {}

macro_rules! counted_star_rows {
    ($origin:expr, scan ($value:expr)) => {{
        STAR_SCANS.with(|counts| {
            let mut next = counts.get();
            next[$origin] += 1;
            counts.set(next);
        });
        ($value).iter().inspect(|_| STAR_VISITS.with(|counts| {
            let mut next = counts.get();
            next[$origin] += 1;
            counts.set(next);
        }))
    }};
    ($origin:expr, $($op:tt)*) => { rows! { $($op)* } };
}

// These adapters instrument actual provider traversal, independently of the
// number of generated iterator adapters or the replay storage representation.
macro_rules! counted_facts {
    ($($op:tt)*) => { counted_star_rows! { 0, $($op)* } };
}
macro_rules! counted_tags {
    ($($op:tt)*) => { counted_star_rows! { 1, $($op)* } };
}
macro_rules! counted_allowed {
    ($($op:tt)*) => { counted_star_rows! { 2, $($op)* } };
}

mod counted_star {
    use super::{ComparedKey, Payload, STAR_SCANS, STAR_VISITS};
    mod wiring {
        pub use ::cq_storage as storage;
        pub use ::mini_linq::{
            access_to_distinct_iter, access_to_exists, access_to_support_iter,
            cq_to_ra_hypergraph as query, lower_borrowed_nested as choose_relation_lowering,
            no_index_demands as choose_index_policy, pipeline_to_distinct_iter, pipeline_to_exists,
            pipeline_to_support_iter, replay_resource,
        };
        pub(super) mod inputs {
            pub(crate) use counted_allowed as Allowed;
            pub(crate) use counted_facts as Facts;
            pub(crate) use counted_tags as Tags;
        }
    }
    use wiring as mini_linq;
    mini_linq::query! {
        pub struct Star;
        relation Facts(key: ComparedKey, value: Payload);
        relation Tags(key: ComparedKey, tag: i32);
        relation Allowed(key: ComparedKey);
        answer(value, tag) :- Facts(key, value), Tags(key, tag), Allowed(key);
    }
}

fn reset_star_measurements() {
    STAR_SCANS.with(|counts| counts.set([0; 3]));
    STAR_VISITS.with(|counts| counts.set([0; 3]));
    KEY_COMPARISONS.with(|counts| counts.set([[0; 3]; 3]));
}

#[test]
fn pull_ya_shares_reductions_and_reads_each_source_occurrence_once() {
    let facts: Vec<_> = (1..=3)
        .map(|value| (ComparedKey { value, origin: 0 }, Payload(value.to_string())))
        .collect();
    let tags: Vec<_> = (1..=3)
        .map(|value| (ComparedKey { value, origin: 1 }, value * 10))
        .collect();
    let allowed: Vec<_> = (1..=3)
        .map(|value| (ComparedKey { value, origin: 2 },))
        .collect();
    let expected: HashSet<_> = facts
        .iter()
        .flat_map(|fact| {
            tags.iter().filter_map(|tag| {
                (fact.0.value == tag.0.value
                    && allowed.iter().any(|key| key.0.value == fact.0.value))
                .then_some((&fact.1, &tag.1))
            })
        })
        .collect();

    reset_star_measurements();
    let mut output = counted_star::Star::iter(&facts, &tags, &allowed);
    STAR_SCANS.with(|counts| assert_eq!(counts.get(), [0; 3]));
    STAR_VISITS.with(|counts| assert_eq!(counts.get(), [0; 3]));
    KEY_COMPARISONS.with(|counts| assert_eq!(counts.get(), [[0; 3]; 3]));

    let first = output.next().unwrap();
    assert!(std::ptr::eq(first.0, &facts[0].1));
    assert!(std::ptr::eq(first.1, &tags[0].1));
    STAR_SCANS.with(|counts| assert_eq!(counts.get(), [1; 3]));
    STAR_VISITS.with(|counts| assert_eq!(counts.get(), [1; 3]));
    let before_drop = KEY_COMPARISONS.with(Cell::get);
    drop(output);
    STAR_VISITS.with(|counts| assert_eq!(counts.get(), [1; 3]));
    KEY_COMPARISONS.with(|counts| assert_eq!(counts.get(), before_drop));
    // Output references belong to the input, even after all replay state drops.
    assert_eq!(first.0.0, "1");

    reset_star_measurements();
    let actual: Vec<_> = counted_star::Star::iter(&facts, &tags, &allowed).collect();
    assert_eq!(actual.len(), expected.len());
    assert_eq!(actual.into_iter().collect::<HashSet<_>>(), expected);
    STAR_SCANS.with(|counts| assert_eq!(counts.get(), [1; 3]));
    STAR_VISITS.with(|counts| assert_eq!(counts.get(), [3; 3]));
    // In this YA plan only Tags ⋉ Facts compares a Tags key to a Facts key.
    // Its three witnesses cost 1 + 2 + 3 comparisons, regardless of how many
    // consumers use the reduced Tags. Replaying raw inputs alone is insufficient.
    KEY_COMPARISONS.with(|counts| assert_eq!(counts.get()[1][0], 6));
}

macro_rules! borrowed_output {
    ($realizer:path, $name:ident) => {
        $realizer! {
            struct $name;
            relation R(payload: Payload);
            answer(payload) :- R(payload);
            relational { r = rename R {payload -> payload}; output r as answer(payload); } indexes {}
        }
    };
}
borrowed_output!(mini_linq::lower_borrowed_nested, BorrowPull);

mini_linq::lower_borrowed_nested! {
    struct BorrowReplay;
    relation R(payload: Payload);
    answer(payload) :- R(payload);
    relational {
        r = rename R { payload -> payload };
        kept = semijoin r with r;
        output kept as answer(payload);
    } indexes {}
}

#[test]
fn output_itself_can_be_nonclone() {
    let source = vec![(Payload("borrowed".into()),)];
    let pulled = BorrowPull::iter(&source).next().unwrap();
    assert!(std::ptr::eq(pulled.0, &source[0].0));

    let mut replayed = BorrowReplay::iter(&source);
    let retained = replayed.next().unwrap();
    drop(replayed);
    assert!(std::ptr::eq(retained.0, &source[0].0));
    assert_eq!(retained.0.0, "borrowed");
}

macro_rules! existential_plan {
    ($realizer:path, $name:ident) => {
        $realizer! {
            struct $name;
            relation Measured(x: i32);
            relation OtherMeasured(x: i32);
            answer(x) :- Measured(x), OtherMeasured(x);
            relational {
                r = rename Measured {x -> x};
                s = rename OtherMeasured {x -> x};
                kept = semijoin r with s;
                output kept as answer(x);
            } indexes {}
        }
    };
}
existential_plan!(mini_linq::lower_borrowed_nested, ExistsPull);

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
    VISITS.with(|n| assert_eq!(n.get(), 4));
    VISITS.with(|n| n.set(0));
    assert!(ExistsPull::iter(&r, &vec![]).next().is_none());
}

macro_rules! disjoint_plan {
    ($realizer:path, $name:ident) => {
        $realizer! {
            struct $name;
            relation R(x: i32);
            relation S(y: i32);
            answer(x, x) :- R(x), S(y);
            relational {
                r = rename R {x -> x};
                s = rename S {y -> y};
                kept = semijoin r with s;
                output kept as answer(x, x);
            } indexes {}
        }
    };
}
disjoint_plan!(mini_linq::lower_borrowed_nested, DisjointPull);

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
    }
}
