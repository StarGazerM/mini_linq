use cq_storage::counters::{self, Snapshot};
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
fn membership_hashing_is_counted_without_logical_distinct_insertions() {
    counters::reset();
    // Retained hash use is a membership probe, not whole-stream canonicalization.
    let mut members = counters::HashSet::default();
    assert!(members.insert(1));
    assert!(members.insert(2));
    let built = counters::snapshot().hash_computations;
    assert!(built > 0);
    assert!(members.contains(&1));
    assert!(!members.contains(&3));
    assert!(counters::snapshot().hash_computations > built);
    assert_eq!(counters::snapshot().distinct_insertions, 0);
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

#[test]
fn singleton_cursor_needs_no_retained_allocation_and_count_does_not_walk_bags() {
    use cq_storage::Candidates;
    counters::reset();
    let singleton = HashIndex::build([(1, 10)].into_iter());
    let cursor = singleton.lookup(&1);
    assert!(counters::snapshot().bytes_retained_current > 0);
    drop(singleton);
    assert_eq!(counters::snapshot().bytes_retained_current, 0);
    assert_eq!(cursor.collect::<Vec<_>>(), [10]);

    counters::reset();
    let index = HashIndex::build([(1, 10), (1, 10), (1, 11)].into_iter());
    assert_eq!(index.count(&1), 3);
    let built = counters::snapshot();
    assert_eq!(built.index_builds, 1);
    assert_eq!(built.rows_copied, 3);
    assert_eq!(built.cursor_steps, 0);
    assert_eq!(index.count(&1), 3);
    assert_eq!(index.count(&2), 0);
    let counted = counters::snapshot();
    assert_eq!(counted.index_builds, 1);
    assert_eq!(counted.rows_copied, 3);
    assert_eq!(counted.cursor_steps, 0);
}

#[test]
fn sort_canonicalization_does_not_count_hashing_or_set_insertions() {
    use cq_storage::dedup::{ordered, ordered_ref, par_sort, sort, verify_prefix_order};

    counters::reset();
    assert_eq!(sort([2, 1, 2], Ord::cmp).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(par_sort([2, 1, 2], Ord::cmp).collect::<Vec<_>>(), [1, 2]);
    let checked = verify_prefix_order(vec![(1, 2), (1, 1), (1, 2)], |a, b| a.0.cmp(&b.0)).unwrap();
    assert_eq!(
        ordered_ref(&checked, Ord::cmp).copied().collect::<Vec<_>>(),
        [(1, 1), (1, 2)]
    );
    assert_eq!(
        ordered(checked, Ord::cmp).collect::<Vec<_>>(),
        [(1, 1), (1, 2)]
    );
    let counts = counters::snapshot();
    assert_eq!(counts.hash_computations, 0);
    assert_eq!(counts.distinct_insertions, 0);
}
