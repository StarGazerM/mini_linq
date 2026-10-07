use cq_storage::counters::{self, DistinctSet, Snapshot};
use cq_storage::rows::Rows;
use cq_storage::{Build, HashIndex, IndexRead};

fn run() -> Snapshot {
    let rows = [(1, 10), (1, 11), (2, 20)];
    counters::reset();
    let index: HashIndex<_, _> = Build::build((&rows).rows());
    assert_eq!(counters::snapshot(), Snapshot::default());
    let mut cursor = index.lookup(&&1);
    assert_eq!(cursor.next(), Some(&10));
    let first = counters::snapshot();
    assert_eq!(
        (first.index_builds, first.lookups, first.rows_enumerated),
        (1, 1, 4)
    );
    assert!(first.hash_computations > 0);
    let shared = index.clone();
    assert_eq!(shared.lookup(&&2).next(), Some(&20));
    assert_eq!(counters::snapshot().index_builds, 1);
    drop(shared);
    drop(index);
    assert_eq!(cursor.next(), Some(&11));
    assert_eq!(cursor.next(), None);
    counters::snapshot()
}

#[test]
fn fresh_runs_have_identical_costs_and_builds_are_shared() {
    assert_eq!(run(), run());
}

#[test]
fn counters_are_isolated_per_thread() {
    counters::reset();
    assert!(std::thread::spawn(run).join().unwrap().rows_enumerated > 0);
    assert_eq!(counters::snapshot(), Snapshot::default());
}

#[test]
fn distinct_insertions_include_rejected_duplicates() {
    counters::reset();
    let mut seen = DistinctSet::new();
    assert!(seen.insert(1));
    assert!(!seen.insert(1));
    assert!(seen.insert(2));
    assert_eq!(counters::snapshot().distinct_insertions, 3);
}

#[test]
fn retained_payload_cursors_and_reset_epochs_have_correct_ownership() {
    use cq_storage::{AdaptiveKeySet, I64Codec};
    counters::reset();
    let old = AdaptiveKeySet::<_, _, 64>::from_keys([1i64], I64Codec);
    counters::reset();
    let fresh = AdaptiveKeySet::<_, _, 64>::from_keys([2i64], I64Codec);
    drop(old);
    assert_eq!(counters::snapshot().bytes_retained_current, 8);
    drop(fresh);
    assert_eq!(counters::snapshot().bytes_retained_current, 0);
    let index = HashIndex::build([(1i64, 10i64), (1, 11), (2, 20)].into_iter());
    assert_eq!(counters::snapshot().rows_copied, 0);
    let cursor = index.lookup(&1);
    assert_eq!(counters::snapshot().rows_copied, 3);
    let allocated = counters::snapshot().bytes_retained_current;
    assert!(allocated > 0);
    drop(index);
    let remaining = counters::snapshot().bytes_retained_current;
    assert!(remaining > 0 && remaining < allocated);
    drop(cursor);
    assert_eq!(counters::snapshot().bytes_retained_current, 0);
}
