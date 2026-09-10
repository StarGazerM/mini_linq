//! Empty binding patterns preserve the extension's evaluation population.
use std::cell::Cell;

thread_local! {
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn tick() {
    CALLS.with(|calls| calls.set(calls.get() + 1));
}

mini_linq::mini_linq! {
    struct Lazy;
    relation R(key: i32);
    answer(x) :- R(x), let () = tick(), let _ = tick();
}

mini_linq::planned_query! {
    struct Materialized;
    relation R(key: i32);
    answer(x) :- R(x), let () = tick(), let _ = tick();
}

mini_linq::mini_linq! {
    struct LazyRoot;
    relation R(key: i32);
    answer(x) :- let () = tick(), R(x);
}

mini_linq::planned_query! {
    struct MaterializedRoot;
    relation R(key: i32);
    answer(x) :- let () = tick(), R(x);
}

#[test]
fn empty_patterns_evaluate_once_per_distinct_prefix_on_both_routes() {
    for rows in [vec![], vec![(1,)], vec![(1,), (2,), (1,)]] {
        let mut expected = rows.clone();
        expected.sort_unstable();
        expected.dedup();
        CALLS.with(|calls| calls.set(0));
        assert_eq!(Lazy::run(rows.clone()), expected);
        assert_eq!(CALLS.with(Cell::get), 2 * expected.len());
        CALLS.with(|calls| calls.set(0));
        assert_eq!(Materialized::run(rows), expected);
        assert_eq!(CALLS.with(Cell::get), 2 * expected.len());
    }
}

#[test]
fn zero_binding_extension_retains_lazy_demand_and_materialized_build_timing() {
    CALLS.with(|calls| calls.set(0));
    let storage = Lazy::build([(1,), (2,)]);
    assert_eq!(CALLS.with(Cell::get), 0);
    let mut cursor = storage.query();
    assert!(cursor.next().is_some());
    assert_eq!(CALLS.with(Cell::get), 2);
    drop(cursor);
    assert_eq!(CALLS.with(Cell::get), 2);

    CALLS.with(|calls| calls.set(0));
    let storage = Materialized::build([(1,), (2,)]);
    assert_eq!(CALLS.with(Cell::get), 4);
    assert_eq!(storage.query().count(), 2);
    assert_eq!(CALLS.with(Cell::get), 4);
}

#[test]
fn a_root_extension_runs_once_even_when_the_later_relation_is_empty() {
    CALLS.with(|calls| calls.set(0));
    assert!(LazyRoot::run([]).is_empty());
    assert_eq!(CALLS.with(Cell::get), 1);
    CALLS.with(|calls| calls.set(0));
    assert!(MaterializedRoot::run([]).is_empty());
    assert_eq!(CALLS.with(Cell::get), 1);
}
