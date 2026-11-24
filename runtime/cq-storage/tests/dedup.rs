use cq_storage::dedup::{
    ordered, ordered_ref, par_ordered_ref, par_sort, sort, verify_prefix_order,
};
use cq_storage::rows::Rows;
use std::cell::Cell;
use std::cmp::Ordering;

#[test]
fn whole_stream_sort_removes_nonadjacent_duplicates() {
    let rows = [(3, 1), (1, 2), (3, 1), (1, 1), (1, 2), (2, 0)];
    let expected = [(1, 1), (1, 2), (2, 0), (3, 1)];
    assert_eq!(sort(rows, Ord::cmp).collect::<Vec<_>>(), expected);
    assert_eq!(par_sort(rows, Ord::cmp).collect::<Vec<_>>(), expected);
}

#[test]
fn verified_prefix_runs_still_sort_and_remove_duplicates() {
    let rows = vec![(1, 3), (1, 1), (1, 3), (1, 2), (2, 9), (2, 8), (2, 9)];
    let checked = verify_prefix_order(rows, |a, b| a.0.cmp(&b.0)).unwrap();
    let expected = [(1, 1), (1, 2), (1, 3), (2, 8), (2, 9)];
    assert_eq!(
        ordered_ref(&checked, Ord::cmp).copied().collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        par_ordered_ref(&checked, Ord::cmp)
            .copied()
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        (&checked).rows().map(|(a, b)| (*a, *b)).collect::<Vec<_>>(),
        [(1, 3), (1, 1), (1, 3), (1, 2), (2, 9), (2, 8), (2, 9)]
    );
    // Borrowed canonicalization neither mutates nor consumes the load witness.
    assert_eq!(checked.as_slice()[0], (1, 3));
    assert_eq!(checked.len(), 7);
    assert_eq!(ordered(checked, Ord::cmp).collect::<Vec<_>>(), expected);
}

#[test]
fn run_boundaries_are_not_duplicate_identity() {
    let checked =
        verify_prefix_order(vec![(1, 7), (1, 7), (2, 7), (2, 7)], |a, b| a.0.cmp(&b.0)).unwrap();
    assert_eq!(
        ordered(checked, Ord::cmp).collect::<Vec<_>>(),
        [(1, 7), (2, 7)]
    );
}

#[test]
fn empty_and_singleton_inputs_need_no_comparisons() {
    fn unexpected(_: &i32, _: &i32) -> Ordering {
        panic!("empty or singleton input must not compare rows")
    }
    for rows in [vec![], vec![4]] {
        let expected = rows.clone();
        assert_eq!(sort(rows.clone(), unexpected).collect::<Vec<_>>(), expected);
        assert_eq!(
            par_sort(rows.clone(), unexpected).collect::<Vec<_>>(),
            expected
        );
        let checked = verify_prefix_order(rows, unexpected).unwrap();
        assert_eq!(checked.is_empty(), expected.is_empty());
        assert_eq!(
            ordered_ref(&checked, unexpected)
                .copied()
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            par_ordered_ref(&checked, unexpected)
                .copied()
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(ordered(checked, unexpected).collect::<Vec<_>>(), expected);
    }
}

// No Clone or Hash implementation: canonicalization only moves rows and borrows
// fields. Thirteen columns also exceed Rust's flat tuple Ord implementations.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Opaque(i32);

type Wide = (
    Opaque,
    Opaque,
    Opaque,
    Opaque,
    Opaque,
    Opaque,
    Opaque,
    Opaque,
    Opaque,
    Opaque,
    Opaque,
    Opaque,
    Opaque,
);

fn wide(prefix: i32, suffix: i32) -> Wide {
    (
        Opaque(prefix),
        Opaque(0),
        Opaque(0),
        Opaque(0),
        Opaque(0),
        Opaque(0),
        Opaque(0),
        Opaque(0),
        Opaque(0),
        Opaque(0),
        Opaque(0),
        Opaque(0),
        Opaque(suffix),
    )
}

fn compare_wide(a: &Wide, b: &Wide) -> Ordering {
    (
        (&a.0, &a.1, &a.2, &a.3, &a.4, &a.5, &a.6),
        (&a.7, &a.8, &a.9, &a.10, &a.11, &a.12),
    )
        .cmp(&(
            (&b.0, &b.1, &b.2, &b.3, &b.4, &b.5, &b.6),
            (&b.7, &b.8, &b.9, &b.10, &b.11, &b.12),
        ))
}

#[test]
fn wide_rows_and_opaque_ord_fields_need_no_row_traits() {
    let rows = || vec![wide(2, 1), wide(1, 9), wide(1, 2), wide(1, 9)];
    let endpoints = |row: Wide| (row.0.0, row.12.0);
    let expected = [(1, 2), (1, 9), (2, 1)];
    assert_eq!(
        sort(rows(), compare_wide)
            .map(endpoints)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        par_sort(rows(), compare_wide)
            .map(endpoints)
            .collect::<Vec<_>>(),
        expected
    );
    let checked = verify_prefix_order(
        vec![wide(1, 9), wide(1, 2), wide(1, 9), wide(2, 1)],
        |a, b| a.0.cmp(&b.0),
    )
    .unwrap();
    assert_eq!(
        ordered_ref(&checked, compare_wide)
            .map(|row| (row.0.0, row.12.0))
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        ordered(checked, compare_wide)
            .map(endpoints)
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn parallel_sort_requires_send_but_not_sync_rows() {
    let rows = [Cell::new(3), Cell::new(1), Cell::new(3), Cell::new(2)];
    assert_eq!(
        par_sort(rows, |a, b| a.get().cmp(&b.get()))
            .map(Cell::into_inner)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
}
