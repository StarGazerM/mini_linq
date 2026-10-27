//! Exercise the semantic boundary through traits rather than inherent APIs.

use cq_storage::ExactMembership;
use std::collections::{BTreeSet, HashSet};

fn has<K: ?Sized>(keys: &impl ExactMembership<K>, key: &K) -> bool {
    keys.contains(key)
}

#[test]
fn exact_collections_and_borrowed_resources_preserve_membership() {
    for mask in 0..8 {
        let values: Vec<_> = (-1..=1)
            .filter(|value| mask & (1 << (value + 1)) != 0)
            .collect();
        let hash: HashSet<_> = values.iter().copied().collect();
        let sorted: BTreeSet<_> = values.iter().copied().collect();
        for key in -2..=2 {
            let expected = values.contains(&key);
            assert_eq!(has(&hash, &key), expected);
            assert_eq!(has(&sorted, &key), expected);
            assert_eq!(has(&&hash, &key), expected);
            assert_eq!(has(&&&sorted, &key), expected);
        }
    }
    let hash = HashSet::from([String::from("present")]);
    let sorted = BTreeSet::from([String::from("present")]);
    assert!(has(&hash, "present"));
    assert!(!has(&sorted, "absent"));
}

#[test]
fn sorted_run_membership_is_exact_bucket_nonemptiness() {
    use cq_storage::{Build, I64Codec, IndexRead, SortedRun};
    for mask in 0..16 {
        let source: Vec<_> = [-65i64, -1, 0, 65]
            .into_iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .flat_map(|(_, key)| [(key, 'a'), (key, 'a'), (key, 'b')])
            .collect();
        let run = SortedRun::<_, _, I64Codec>::build(source.iter().copied());
        for key in -66..=66 {
            let expected = source.iter().any(|(k, _)| *k == key);
            assert_eq!(has(&run, &key), expected);
            assert_eq!(has(&&run, &key), expected);
            assert_eq!((&run).lookup(&key).next().is_some(), expected);
        }
    }
}

#[test]
fn adaptive_membership_and_intervals_match_independent_ordered_sets() {
    use cq_storage::{AdaptiveKeySet, ExactIntervalMembership, I64Codec, NoDense};
    let universe = [-65i64, -64, -1, 0, 1, 63, 64, 65, 127];
    for mask in 0..1usize << universe.len() {
        let input: Vec<_> = universe
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .flat_map(|(_, &k)| [k, k])
            .collect();
        let oracle: std::collections::BTreeSet<_> = input.iter().copied().collect();
        let dense = AdaptiveKeySet::<_, _, 193>::from_keys(input.iter().copied(), I64Codec);
        let hash = AdaptiveKeySet::<_, _, 193>::from_keys(input.iter().copied(), NoDense);
        for lo in -67..=129 {
            assert_eq!(dense.contains(&lo), oracle.contains(&lo));
            assert_eq!(hash.contains(&lo), oracle.contains(&lo));
            for hi in [lo - 1, lo, lo + 1, 64, 130] {
                let expected = lo <= hi && oracle.iter().any(|&k| lo <= k && k <= hi);
                assert_eq!(dense.any_in(&lo, &hi), expected, "{input:?} [{lo},{hi}]");
                assert_eq!(hash.any_in(&lo, &hi), expected);
                assert_eq!(hash.any_in(&lo, &hi), expected);
            }
        }
    }
}

#[test]
fn adaptive_extremes_budget_edges_and_borrowed_tpc_keys() {
    use cq_storage::*;
    for input in [
        vec![],
        vec![i64::MIN],
        vec![i64::MAX],
        vec![i64::MIN, i64::MAX],
        vec![-1, -1, 0],
    ] {
        let set = AdaptiveKeySet::<_, _, 64>::from_keys(input.clone(), I64Codec);
        for lo in [i64::MIN, -1, 0, 1, i64::MAX] {
            for hi in [i64::MIN, -1, 0, 1, i64::MAX] {
                assert_eq!(
                    set.any_in(&lo, &hi),
                    input.iter().any(|&k| lo <= k && k <= hi)
                );
            }
        }
    }
    let at = AdaptiveKeySet::<_, _, 65>::from_keys([0i64, 64], I64Codec);
    assert_eq!(at.retained_bytes(), 16);
    let below = AdaptiveKeySet::<_, _, 64>::from_keys([0i64, 64], I64Codec);
    assert!(below.retained_bytes() > 16);
    let zero = AdaptiveKeySet::<_, _, 0>::from_keys([0i64], I64Codec);
    assert!(zero.contains(&0));
    let keys = [i64::MIN, -1, 0, i64::MAX];
    let borrowed =
        AdaptiveKeySet::<_, _, 128>::from_keys(keys.iter().map(|k| (k,)), BorrowedI64TupleCodec);
    assert!(borrowed.contains(&(&keys[1],)));
    assert!(!borrowed.any_in(&(&1,), &(&2,)));
    let i32s = AdaptiveKeySet::<i32, _, 65>::from_keys([-32, 32], I64Codec);
    assert_eq!(i32s.retained_bytes(), 16);
    let borrowed32 =
        AdaptiveKeySet::<_, _, 65>::from_keys([(&-32i32,), (&32i32,)], BorrowedI64TupleCodec);
    assert!(borrowed32.any_in(&(&32,), &(&32,)));
    #[derive(Clone, Copy)]
    struct Partial;
    impl KeyCodec<i64> for Partial {
        fn ordinal(self, key: &i64) -> Option<u128> {
            (*key >= 0).then_some(*key as u128)
        }
    }
    let partial = AdaptiveKeySet::<_, _, 128>::from_keys([1, 3], Partial);
    assert!(partial.any_in(&-1, &2));
}

#[test]
fn predicates_obey_inverted_and_exclusive_interval_laws() {
    use cq_storage::{Point, ReadPredicate, RowPredicate, Window};
    use std::ops::Bound::*;
    let bounds = [
        Unbounded,
        Included(-1),
        Excluded(-1),
        Included(0),
        Excluded(0),
        Included(1),
        Excluded(1),
    ];
    for low in bounds {
        for high in bounds {
            let window = Window { low, high };
            for lo in -2..=2 {
                for hi in -2..=2 {
                    // Doubled integer universe independently models open order intervals.
                    let expected = (2 * lo..=2 * hi).any(|x| {
                        let l = match low {
                            Unbounded => true,
                            Included(a) => x >= 2 * a,
                            Excluded(a) => x > 2 * a,
                        };
                        let h = match high {
                            Unbounded => true,
                            Included(a) => x <= 2 * a,
                            Excluded(a) => x < 2 * a,
                        };
                        l && h
                    });
                    assert_eq!(window.any_in(&lo, &hi), expected);
                    assert_eq!(Point(0).any_in(&lo, &hi), lo <= 0 && 0 <= hi);
                }
                assert_eq!(
                    window.test(&lo),
                    match low {
                        Unbounded => true,
                        Included(a) => lo >= a,
                        Excluded(a) => lo > a,
                    } && match high {
                        Unbounded => true,
                        Included(a) => lo <= a,
                        Excluded(a) => lo < a,
                    }
                );
            }
        }
    }
    assert!(RowPredicate(|_: &i32| false).any_in(&0, &1));
}

#[test]
fn codec_full_width_overflow_and_nonclone_keys_are_exact() {
    use cq_storage::*;
    #[derive(Eq, PartialEq, Ord, PartialOrd, Hash)]
    struct NonClone(String);
    let set = AdaptiveKeySet::<_, _, 64>::from_keys(
        [NonClone("a".into()), NonClone("c".into())],
        NoDense,
    );
    assert!(!set.any_in(&NonClone("b".into()), &NonClone("b".into())));
    #[derive(Clone, Copy)]
    struct Full;
    impl KeyCodec<u128> for Full {
        fn ordinal(self, key: &u128) -> Option<u128> {
            Some(*key)
        }
        fn total(self) -> bool {
            true
        }
    }
    let full = AdaptiveKeySet::<_, _, 64>::from_keys([0, u128::MAX], Full);
    assert!(!full.any_in(&1, &(u128::MAX - 1)));
    assert!(full.any_in(&u128::MAX, &u128::MAX));
    assert_eq!(I64Codec.ordinal(&i64::MIN), Some(0));
    assert_eq!(I64Codec.ordinal(&i64::MAX), Some(u64::MAX as u128));
    let empty = AdaptiveKeySet::<i64, _, 64>::from_keys([], I64Codec);
    assert_eq!(empty.retained_bytes(), 0);
}

#[test]
fn immutable_adaptive_forms_support_shared_task_reads() {
    use cq_storage::{AdaptiveKeySet, ExactMembership, I64Codec, NoDense};
    let dense = AdaptiveKeySet::<_, _, 64>::from_keys([1i64, 3], I64Codec);
    let hashed = AdaptiveKeySet::<_, _, 64>::from_keys([1i64, 3], NoDense);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|key| {
                let dense = &dense;
                let hashed = &hashed;
                scope.spawn(move || (dense.contains(&key), hashed.contains(&key)))
            })
            .collect();
        for (key, worker) in workers.into_iter().enumerate() {
            let expected = key == 1 || key == 3;
            assert_eq!(worker.join().unwrap(), (expected, expected));
        }
    });
}
