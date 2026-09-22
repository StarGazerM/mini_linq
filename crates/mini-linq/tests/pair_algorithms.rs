//! Algorithm replacement owns traversal, while the selected provider owns output.
use std::collections::{BTreeSet, HashSet};

macro_rules! result {
    ($algorithm:ident, $provider:ident, $left:expr, $right:expr) => {{
        let left = $left;
        let right = $right;
        mini_linq::$provider! {
            produce owner view via emit {
                mini_linq::$algorithm! {
                    pairs l in (left.iter()) by (l.0),
                          r in (right.iter()) by (r.0) {
                        emit([l.1, r.1]);
                    }
                };
            }
        }
        view.iter().copied().collect::<BTreeSet<_>>()
    }};
}

#[test]
fn different_traversal_algorithms_and_result_stores_match_an_independent_oracle() {
    let universe = [(0_i64, 10_i64), (0, 20), (1, 10), (1, 20)];
    for left_mask in 0..16 {
        for right_mask in 0..16 {
            let rows = |mask: usize| {
                universe
                    .iter()
                    .enumerate()
                    .filter_map(|(i, row)| (mask & (1 << i) != 0).then_some(*row))
                    .collect::<Vec<_>>()
            };
            let left = rows(left_mask);
            let right = rows(right_mask);
            let expected: BTreeSet<_> = left
                .iter()
                .flat_map(|l| {
                    right
                        .iter()
                        .filter(move |r| l.0 == r.0)
                        .map(move |r| [l.1, r.1])
                })
                .collect();
            assert_eq!(
                result!(hash_pairs, array_hash_rows, &left, &right),
                expected
            );
            assert_eq!(
                result!(nested_pairs, array_sorted_rows, &left, &right),
                expected
            );
            assert_eq!(
                result!(sort_merge_pairs, array_hash_rows, &left, &right),
                expected
            );
        }
    }
}

#[test]
fn pair_algorithms_preserve_duplicate_witnesses_before_the_sink() {
    let left = [(1, 10), (1, 10), (2, 20)];
    let right = [(1, 30), (1, 30), (2, 40), (3, 50)];
    macro_rules! pairs {
        ($algorithm:ident) => {{
            let mut rows = Vec::new();
            mini_linq::$algorithm! {
                pairs l in (left.iter()) by (l.0), r in (right.iter()) by (r.0) {
                    rows.push((l.1, r.1));
                }
            };
            rows.sort();
            rows
        }};
    }
    let expected = vec![(10, 30), (10, 30), (10, 30), (10, 30), (20, 40)];
    assert_eq!(pairs!(hash_pairs), expected);
    assert_eq!(pairs!(nested_pairs), expected);
    assert_eq!(pairs!(sort_merge_pairs), expected);
}

#[test]
fn empty_keys_produce_cartesian_pairs_and_empty_inputs_produce_none() {
    let mut rows = HashSet::new();
    mini_linq::sort_merge_pairs! {
        pairs l in (0..3) by (()), r in (0..2) by (()) { rows.insert((*l, *r)); }
    };
    assert_eq!(rows.len(), 6);
    fn no_scan() -> std::iter::Empty<i32> {
        panic!("empty left must not rescan right")
    }
    mini_linq::nested_pairs! {
        pairs _l in (std::iter::empty::<i32>()) by (()), _r in (no_scan()) by (()) {
            panic!("no pairs");
        }
    };
}

#[test]
fn private_algorithm_locals_do_not_capture_caller_expressions() {
    let __cq_pair_key = 2;
    let __cq_pair_index = [2];
    let mut count = 0;
    mini_linq::hash_pairs! {
        pairs l in (__cq_pair_index) by (*l), r in ([2]) by (__cq_pair_key) {
            count += usize::from(*l == *r);
        }
    };
    assert_eq!(count, 1);
}

#[test]
fn hash_and_merge_accept_owned_noncopy_items_without_a_clone_requirement() {
    struct Row(i32);
    let mut count = 0;
    mini_linq::hash_pairs! {
        pairs l in ([Row(1), Row(2)]) by (l.0), r in ([Row(2), Row(1)]) by (r.0) {
            count += usize::from(l.0 == r.0);
        }
    };
    mini_linq::sort_merge_pairs! {
        pairs l in ([Row(1), Row(2)]) by (l.0), r in ([Row(2), Row(1)]) by (r.0) {
            count += usize::from(l.0 == r.0);
        }
    };
    assert_eq!(count, 4);
}

#[test]
fn observed_pair_algorithms_preserve_caller_work_names() {
    let __cq_work = 2;
    let counters = cq_storage::Counters::default();
    let mut count = 0;
    macro_rules! observed {
        ($algorithm:ident) => {
            mini_linq::$algorithm! {
                work (&counters);
                pairs l in ([2]) by (__cq_work), r in ([2]) by (*r) {
                    count += usize::from(*l == __cq_work && *r == __cq_work);
                }
            };
        };
    }
    observed!(hash_pairs);
    observed!(nested_pairs);
    observed!(sort_merge_pairs);
    assert_eq!(count, 3);
    assert!(counters.take().exact_probes >= 3);
}
