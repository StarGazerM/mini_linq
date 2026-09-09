//! Executable counterexamples to unrestricted cross-backend equivalence.
//! Passing these tests confirms a documented semantic gap; it is not evidence
//! that the two routes preserve arbitrary Rust effects. See doc/formal.
use std::cell::Cell;

thread_local! {
    static SEQUENCE: Cell<i32> = const { Cell::new(0) };
}

fn tick(_: &i32) -> i32 {
    SEQUENCE.with(|value| {
        value.set(value.get() + 1);
        value.get()
    })
}

fn latest(value: i32) -> bool {
    SEQUENCE.with(|current| current.get() == value)
}

mini_linq::mini_linq! {
    struct Lazy;
    relation R(key:i32);
    answer(x) :- R(x), let generation:i32 = tick(x), if latest(generation);
}

mini_linq::planned_query! {
    struct Materialized;
    relation R(key:i32);
    answer(x) :- R(x), let generation:i32 = tick(x), if latest(generation);
}

#[test]
fn materialization_changes_stateful_query_results() {
    SEQUENCE.with(|value| value.set(0));
    let lazy = Lazy::run([(10,), (20,)]);
    assert_eq!(lazy, [(10,), (20,)]);
    SEQUENCE.with(|value| value.set(0));
    let materialized = Materialized::run([(10,), (20,)]);
    assert_eq!(materialized.len(), 1);
    assert!(lazy.contains(&materialized[0]));
}

#[test]
fn build_and_partial_consumption_have_different_effect_traces() {
    SEQUENCE.with(|value| value.set(0));
    let storage = Lazy::build([(10,), (20,)]);
    let mut cursor = storage.query();
    assert_eq!(SEQUENCE.with(Cell::get), 0);
    assert!(cursor.next().is_some());
    assert_eq!(SEQUENCE.with(Cell::get), 1);
    drop(cursor);
    assert_eq!(SEQUENCE.with(Cell::get), 1);

    SEQUENCE.with(|value| value.set(0));
    let storage = Materialized::build([(10,), (20,)]);
    assert_eq!(SEQUENCE.with(Cell::get), 2);
    assert_eq!(storage.query().count(), 1);
    assert_eq!(SEQUENCE.with(Cell::get), 2);
}
