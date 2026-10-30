use cq_load::{PrefixOrderError, verify_prefix_order};
use cq_storage::dedup::{ordered, ordered_ref};
use std::cell::Cell;

#[test]
fn load_check_visits_each_adjacent_pair_once_without_reordering() {
    let comparisons = Cell::new(0);
    let rows = vec![(1, 9), (1, 2), (1, 9), (2, 0), (2, 0)];
    let allocation = rows.as_ptr();
    let checked = verify_prefix_order(rows, |a, b| {
        comparisons.set(comparisons.get() + 1);
        a.0.cmp(&b.0)
    })
    .unwrap();
    assert_eq!(comparisons.get(), 4);
    assert_eq!(checked.as_slice().as_ptr(), allocation);
    assert_eq!(
        checked.iter().copied().collect::<Vec<_>>(),
        [(1, 9), (1, 2), (1, 9), (2, 0), (2, 0)]
    );
    assert_eq!(
        ordered_ref(&checked, Ord::cmp).copied().collect::<Vec<_>>(),
        [(1, 2), (1, 9), (2, 0)]
    );
    assert_eq!(
        ordered(checked, Ord::cmp).collect::<Vec<_>>(),
        [(1, 2), (1, 9), (2, 0)]
    );
}

#[test]
fn unsorted_prefixes_reject_at_the_first_descending_pair() {
    let comparisons = Cell::new(0);
    let result = verify_prefix_order(vec![(1, 0), (3, 0), (2, 0), (1, 0)], |a, b| {
        comparisons.set(comparisons.get() + 1);
        a.0.cmp(&b.0)
    });
    assert_eq!(result.err(), Some(PrefixOrderError { row: 2 }));
    assert_eq!(comparisons.get(), 2);
}

#[test]
fn equal_prefixes_never_imply_unique_rows() {
    let checked =
        verify_prefix_order(vec![(7, 3), (7, 1), (7, 3), (7, 1)], |a, b| a.0.cmp(&b.0)).unwrap();
    assert_eq!(checked.len(), 4);
    assert_eq!(
        ordered(checked, Ord::cmp).collect::<Vec<_>>(),
        [(7, 1), (7, 3)]
    );
}

#[test]
fn empty_load_has_a_valid_witness() {
    let checked = verify_prefix_order(Vec::<i32>::new(), |_, _| {
        panic!("an empty load has no adjacent pair")
    })
    .unwrap();
    assert!(checked.is_empty());
    assert_eq!(checked.iter().count(), 0);
    assert_eq!(ordered(checked, Ord::cmp).count(), 0);
}

#[test]
fn recovering_rows_discards_the_order_witness() {
    let checked = verify_prefix_order(vec![1, 2, 3], Ord::cmp).unwrap();
    let mut rows = checked.into_rows();
    rows.reverse();
    assert_eq!(
        verify_prefix_order(rows, Ord::cmp).err(),
        Some(PrefixOrderError { row: 1 })
    );
}
