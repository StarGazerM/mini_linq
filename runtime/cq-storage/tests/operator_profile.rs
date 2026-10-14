#![cfg(feature = "counters")]
use cq_storage::counters::{self, OperatorLabel, OperatorProgram};
const LABELS: &[OperatorLabel] = &[
    OperatorLabel {
        result: "scan",
        kind: "SCAN",
        key: "R",
        inputs: &[],
    },
    OperatorLabel {
        result: "index",
        kind: "INDEX_BUILD",
        key: "k",
        inputs: &["scan"],
    },
];
#[test]
fn deferred_build_counts_only_actual_consumption() {
    use cq_storage::{Build, IndexRead};
    counters::reset();
    let p = OperatorProgram::new(LABELS);
    let index: cq_storage::HashIndex<'_, _, _> = p.build(1, || {
        cq_storage::HashIndex::build(
            p.inserting(1, p.stream(0, [(1, 10), (1, 11), (2, 20)].into_iter())),
        )
    });
    assert_eq!(counters::operators()[1].calls, 0);
    assert_eq!(index.lookup(&1).collect::<Vec<_>>(), vec![10, 11]);
    let records = counters::operators();
    assert_eq!(records[0].rows_out, 3);
    assert_eq!(
        (
            records[1].rows_in,
            records[1].rows_out,
            records[1].rows_inserted,
            records[1].calls
        ),
        (3, 3, 3, 1)
    );
    if counters::STORAGE_COUNTERS_ENABLED {
        assert_eq!(records[1].rows_inserted, counters::snapshot().rows_copied);
    }
    assert!(records[1].elapsed > std::time::Duration::ZERO);
}
#[test]
fn short_circuit_drop_and_reset_do_not_force_rows_or_pollute_a_new_epoch() {
    counters::reset();
    let p = OperatorProgram::new(LABELS);
    let mut rows = p.stream(0, (0..10).inspect(|&v| assert!(v < 2)));
    assert_eq!(rows.next(), Some(0));
    drop(rows);
    assert_eq!(counters::operators()[0].rows_out, 1);
    let old = p.stream(0, 0..3);
    counters::reset();
    drop(old);
    assert!(counters::operators().is_empty());
}
#[test]
fn nested_and_panicking_build_scopes_restore_the_active_build() {
    counters::reset();
    let p = OperatorProgram::new(LABELS);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        p.build(1, || panic!("stop"))
    }));
    use cq_storage::{Build, IndexRead};
    let index: cq_storage::HashIndex<'_, _, _> = cq_storage::HashIndex::build([(1, 2)].into_iter());
    let before = counters::operators();
    assert_eq!(index.lookup(&1).next(), Some(2));
    assert_eq!(counters::operators(), before);
}

#[test]
fn folded_pipeline_counts_match_scalar_demand() {
    counters::reset();
    let p = OperatorProgram::new(LABELS);
    let sum = p
        .stream(0, p.input(0, 0..10).filter(|x| x % 2 == 0))
        .fold((0, 0), |(sum, count), value| (sum + value, count + 1));
    assert_eq!(sum, (20, 5));
    let r = &counters::operators()[0];
    assert_eq!((r.rows_in, r.rows_out), (10, 5));
    fn send<T: Send>() {}
    fn unwind<T: std::panic::UnwindSafe>() {}
    send::<cq_storage::dispatch::KeySet<std::vec::IntoIter<i32>>>();
    unwind::<cq_storage::dispatch::KeySet<std::vec::IntoIter<i32>>>();
}
