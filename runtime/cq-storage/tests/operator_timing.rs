#![cfg(all(feature = "operator-timing", not(feature = "counters")))]
use cq_storage::counters::{self, OperatorLabel, OperatorProgram};

#[test]
fn timing_retains_native_inputs_and_records_deferred_execution() {
    use cq_storage::{Build, IndexRead};
    counters::reset();
    let p = OperatorProgram::new(&[
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
    ]);
    let source: std::ops::Range<i32> = p.input(0, 0..10);
    let index: cq_storage::HashIndex<'_, _, _> = p.build(1, || {
        cq_storage::HashIndex::build(p.stream(0, source.map(|k| (k, k))))
    });
    assert_eq!(counters::operators()[1].calls, 0);
    assert_eq!(index.lookup(&5).next(), Some(5));
    let records = counters::operators();
    assert_eq!(records[1].calls, 1);
    assert!(records[1].elapsed > std::time::Duration::ZERO);
    assert_eq!(
        records[0].rows_out, 0,
        "rows come from the separate counted execution"
    );
    assert_eq!(counters::snapshot(), counters::Snapshot::default());
}
