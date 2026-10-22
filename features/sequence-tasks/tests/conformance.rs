#![allow(non_snake_case)]

use cq_sequence_tasks_test_macros::both;
use std::collections::BTreeSet;

fn conform<Row: Ord + std::fmt::Debug>(pair: (Vec<Row>, Vec<Row>), expected: BTreeSet<Row>) {
    let (pull, tasks) = pair;
    assert_eq!(
        pull.iter().collect::<Vec<_>>(),
        tasks.iter().collect::<Vec<_>>()
    );
    assert_eq!(pull.into_iter().collect::<BTreeSet<_>>(), expected);
    assert_eq!(tasks.into_iter().collect::<BTreeSet<_>>(), expected);
}

#[test]
fn join() {
    let L = vec![(1,), (2,), (1,), (9,)];
    let R = vec![(1, 10), (1, 11), (2, 20), (4, 40)];
    conform(
        both! {
            physical {
                relation L(a:i32); relation R(a:i32,b:i32);
                l = pipe rename L@0 {a -> a};
                r = let rename R@1 {a -> a,b -> b};
                j = pipe canon probe first [a] l,r;
                return j as answer(a,b);
            } structures { r = ::cq_storage::SortedRun<_, _> [a]; }
            sequence {
                r = store rows r;
                input = store rows l;
                result = store tile 2 canon flatten lookup r [a] input;
            }
            graded tasks { r at host; input at host; result at pool; }
        },
        BTreeSet::from([(&1, &10), (&1, &11), (&2, &20)]),
    );
}

#[test]
fn chain() {
    let L = vec![(1,), (2,), (3,)];
    let R = vec![(1, 10), (1, 11), (2, 20)];
    let S = vec![(10, 100), (11, 110), (20, 200), (10, 101)];
    conform(
        both! {
            physical {
                relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32,c:i32);
                l = pipe rename L@0 {a -> a};
                r = let rename R@1 {a -> a,b -> b};
                s = let rename S@2 {b -> b,c -> c};
                j = pipe probe first [a] l,r;
                k = pipe canon probe first [b] j,s;
                return k as answer(a,b,c);
            } structures {
                r = ::cq_storage::SortedRun<_, _> [a];
                s = ::cq_storage::SortedRun<_, _> [b];
            }
            sequence {
                r = store rows r;
                s = store rows s;
                input = store rows l;
                result = store tile 2 canon flatten lookup s [b] flatten lookup r [a] input;
            }
            graded tasks { r at host; s at host; input at host; result at pool; }
        },
        BTreeSet::from([
            (&1, &10, &100),
            (&1, &10, &101),
            (&1, &11, &110),
            (&2, &20, &200),
        ]),
    );
}

#[test]
fn semi() {
    let L = vec![(1,), (2,), (1,), (3,)];
    let R = vec![(1,), (1,), (3,)];
    conform(
        both! {
            physical {
                relation L(a:i32); relation R(a:i32);
                l = pipe rename L@0 {a -> a};
                r = let rename R@1 {a -> a};
                j = pipe canon semi eager [a] l,r;
                return j as answer(a);
            } structures {}
            sequence {
                r = store rows r;
                input = store rows l;
                result = store tile 2 canon compact j lookup r [a] input;
            }
            graded tasks { r at host; input at host; result at pool; }
        },
        BTreeSet::from([(&1,), (&3,)]),
    );
}

#[test]
fn canon_prefilters_parts_then_deduplicates_globally() {
    // Repetitions straddle the task partition boundaries.
    let R: Vec<_> = (0..5000).map(|i| (i % 7,)).collect();
    conform(
        both! {
            physical {
                relation R(a:i32);
                r = pipe canon rename R@0 {a -> a};
                return r as answer(a);
            } structures {}
            sequence {
                input = store rows r;
                result = store canon input;
            }
            graded tasks { input at host; result at pool; }
        },
        (0..7).map(|i| (&[0, 1, 2, 3, 4, 5, 6][i],)).collect(),
    );
}

#[test]
fn empty_and_partial_tiles_preserve_order() {
    for R in [vec![], vec![(4,)], vec![(3,), (1,), (4,), (2,), (5,)]] {
        let (pull, tasks) = both! {
            physical {
                relation R(a:i32);
                r = pipe canon rename R@0 {a -> a};
                return r as answer(a);
            } structures {}
            sequence { input = store rows r; result = store tile 2 canon input; }
            graded tasks { input at host; result at pool; }
        };
        assert_eq!(pull, tasks);
        assert_eq!(tasks, R.iter().map(|(a,)| (a,)).collect::<Vec<_>>());
    }
}
