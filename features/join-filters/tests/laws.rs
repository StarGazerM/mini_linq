use cq_join_filters::{AlwaysTrue, BloomFilter};
use cq_storage::{Build, ConservativeMembership, ReadPredicate};

#[test]
fn no_false_negatives_including_collisions_duplicates_and_extremes() {
    let keys: Vec<_> = (0..10000)
        .map(|x| x * 7919 - 40000000)
        .chain([i64::MIN, i64::MAX, 0, 0])
        .collect();
    // Tiny budget deliberately saturates and exercises collisions.
    let filter = BloomFilter::<2>::build(keys.iter().map(|&k| (k, ())));
    for key in keys {
        assert!(ConservativeMembership::test(&filter, &key));
        assert!(ReadPredicate::test(&filter, &key));
        assert!(ReadPredicate::any_in(&filter, &key, &key));
    }
    let empty = BloomFilter::<2>::build(std::iter::empty::<(i64, ())>());
    assert!(!ConservativeMembership::test(&empty, &0));
    assert!(!ReadPredicate::any_in(&empty, &i64::MIN, &i64::MAX));
    let filter = BloomFilter::<16>::build([-7, 4, 99].map(|k| (k, ())).into_iter());
    assert!(!ReadPredicate::any_in(&filter, &100, &200));
    assert!(!ReadPredicate::any_in(&filter, &-20, &-8));
    assert!(!ReadPredicate::any_in(&filter, &9, &8));
}

#[test]
fn exact_types_automatically_satisfy_the_one_sided_law() {
    let keys = std::collections::BTreeSet::from([i64::MIN, -1, 0, i64::MAX]);
    cq_storage::require::conservative::<i64, _>(&keys);
    for k in &keys {
        assert!(ConservativeMembership::test(&keys, k));
    }
    assert!(!ConservativeMembership::test(&keys, &1));
    let all = AlwaysTrue::build([(1, ()), (2, ())].into_iter());
    for k in [i64::MIN, 1, 2, i64::MAX] {
        assert!(ConservativeMembership::test(&all, &k));
    }
}
