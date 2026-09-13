use cq_storage::{
    BlockDescriptor, BlockRead, BlockStore, BuildBlocks, CoordinateBounds, RangeSummary,
    ZoneSummary,
};
use std::collections::HashSet;
use std::num::NonZeroUsize;

fn size(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

#[test]
fn chunk_directory_encloses_every_payload_and_preserves_coordinates() {
    let rows = [
        [i64::MIN, 5],
        [3, i64::MAX],
        [3, i64::MAX],
        [20, -4],
        [7, 9],
    ];
    let store = BlockStore::from_rows(rows, size(3));
    let descriptors = store.blocks().collect::<Vec<_>>();
    assert_eq!(descriptors.len(), 2);
    assert_eq!(
        descriptors
            .iter()
            .map(|block| block.rows().count())
            .collect::<Vec<_>>(),
        [3, 1]
    );
    assert_eq!(descriptors[0].bounds(), &[(i64::MIN, 20), (-4, i64::MAX)]);
    assert_eq!(descriptors[1].bounds(), &[(7, 7), (9, 9)]);
    for (id, block) in descriptors.into_iter().enumerate() {
        assert_eq!(id, block.id());
        assert_ne!(block.rows().count(), 0);
        let repeated = store.blocks().find(|block| block.id() == id).unwrap();
        for (offset, row) in block.rows() {
            let (_, repeated_row) = repeated
                .rows()
                .find(|&(candidate, _)| candidate == offset)
                .unwrap();
            assert!(std::ptr::eq(row, repeated_row));
            for (&value, &(lo, hi)) in row.iter().zip(block.bounds()) {
                assert!(lo <= value && value <= hi);
            }
        }
    }
}

#[test]
fn constructors_deduplicate_and_handle_empty_and_zero_width_records() {
    let store = BlockStore::from_rows([[3], [1], [3], [2], [1]], size(2));
    assert_eq!(
        store
            .blocks()
            .flat_map(|block| block.rows().map(|(_, row)| *row))
            .collect::<Vec<_>>(),
        [[3], [1], [2]]
    );
    let expected = HashSet::from([[3], [1], [2]]);
    let packed_set = BlockStore::pack_unique(expected.clone(), size(2));
    assert_eq!(
        packed_set
            .blocks()
            .flat_map(|block| block.rows().map(|(_, row)| *row))
            .collect::<HashSet<_>>(),
        expected
    );

    let empty = BlockStore::<0>::from_rows([], size(1));
    assert_eq!(empty.blocks().count(), 0);
    let unit = BlockStore::<0>::from_rows([[], []], size(2));
    assert_eq!(unit.blocks().flat_map(|block| block.rows()).count(), 1);
    assert_eq!(unit.blocks().next().unwrap().bounds(), &[]);
    assert_eq!(unit.blocks().next().unwrap().rows().next(), Some((0, &[])));
    assert_eq!(
        BlockStore::pack_unique(HashSet::from([[], []]), size(1))
            .blocks()
            .flat_map(|block| block.rows())
            .count(),
        1
    );
}

#[test]
fn metadata_can_reject_a_chunk_before_the_caller_opens_it() {
    let store = BlockStore::from_rows([[1], [2], [100], [101]], size(2));
    let mut summary = ZoneSummary::default();
    summary.observe(&[2]);
    let mut opened = Vec::new();
    let mut candidates = Vec::new();
    for descriptor in store.blocks() {
        if summary.may_match(descriptor.bounds(), &[0]) {
            opened.push(descriptor.id());
            candidates.extend(descriptor.rows().map(|(_, row)| *row));
        }
    }
    assert_eq!(opened, [0]);
    // Bounds are conservative; exact filtering is the caller's operation.
    assert_eq!(candidates, [[1], [2]]);
}

#[test]
fn summaries_preserve_soundness_without_claiming_tuple_membership() {
    let mut summary = ZoneSummary::<2>::default();
    let keys = HashSet::from([[i64::MIN, 2], [3, i64::MAX]]);
    for key in &keys {
        summary.observe(key);
    }
    assert!(summary.may_match(&[(2, i64::MAX), (i64::MIN, 3)], &[1, 0]));
    assert!(!keys.contains(&[i64::MIN, i64::MAX]));
    assert!(!summary.may_match(&[(0, 1), (i64::MIN, 3)], &[1, 0]));
    for key in &keys {
        assert!(summary.may_match(&[(key[1], key[1]), (key[0], key[0])], &[1, 0]));
    }
}

#[test]
fn summaries_distinguish_empty_and_nonempty_zero_width_keys() {
    let mut summary = ZoneSummary::<0>::default();
    assert!(!summary.may_match(&[(1, 2)], &[]));
    assert!(!summary.may_match(&[], &[]));
    summary.observe(&[]);
    assert!(summary.may_match(&[(1, 2)], &[]));
    assert!(summary.may_match(&[], &[]));
}

#[test]
#[should_panic(expected = "zone summary key arity mismatch")]
fn empty_summary_still_validates_coordinate_mapping() {
    ZoneSummary::<2>::default().may_match(&[(1, 2)], &[0]);
}
