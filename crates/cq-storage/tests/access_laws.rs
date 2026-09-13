//! Exercise the semantic boundary through traits rather than inherent APIs.

use cq_storage::{
    BlockDescriptor, BlockMask, BlockRead, BlockStore, BuildBlocks, CoordinateBounds,
    ExactMembership, MakeSelection, RangeSummary, ZoneSummary,
};
use std::collections::{BTreeSet, HashSet};
use std::num::NonZeroUsize;

fn check_directory<S, M>(store: &S, expected: &HashSet<[i64; 2]>)
where
    S: BlockRead<Row = [i64; 2]>,
    S::BlockId: Eq + std::hash::Hash,
    S::RowId: Eq,
    for<'a> S::Descriptor<'a>: CoordinateBounds,
    M: MakeSelection<S>,
{
    let all = M::all(store);
    let mut selected = M::empty(store);
    assert!(selected.is_empty());
    let mut seen = HashSet::new();
    let mut ids = HashSet::new();
    for descriptor in store.blocks() {
        assert!(ids.insert(descriptor.id()));
        assert!(descriptor.store().identity() == store.identity());
        let mut nonempty = false;
        for (offset, row) in descriptor.rows() {
            assert!(seen.insert(*row));
            let repeated = store
                .blocks()
                .find(|b| b.id() == descriptor.id())
                .unwrap()
                .rows()
                .find(|(id, _)| *id == offset)
                .unwrap()
                .1;
            assert!(std::ptr::eq(row, repeated));
            assert!(all.contains(descriptor.id(), offset));
            assert!(!selected.contains(descriptor.id(), offset));
            assert!(selected.insert(descriptor.id(), offset));
            assert!(!selected.insert(descriptor.id(), offset));
            for (&value, &(lo, hi)) in row.iter().zip(descriptor.bounds()) {
                assert!(lo <= value && value <= hi);
            }
            nonempty = true;
        }
        assert_eq!(all.block_nonempty(descriptor.id()), nonempty);
        assert_eq!(selected.block_nonempty(descriptor.id()), nonempty);
    }
    assert_eq!(&seen, expected);
    assert_eq!(all.len(), expected.len());
    assert_eq!(selected.len(), expected.len());
}

#[test]
fn constructed_directory_and_separate_selection_share_exact_address_domain() {
    let expected = HashSet::from([[i64::MIN, 5], [3, i64::MAX], [20, -4], [7, 9]]);
    let store = BlockStore::<2>::pack_unique(expected.clone(), NonZeroUsize::new(3).unwrap());
    check_directory::<_, BlockMask>(&store, &expected);
    let empty = BlockStore::<2>::pack_unique(HashSet::new(), NonZeroUsize::new(3).unwrap());
    check_directory::<_, BlockMask>(&empty, &HashSet::new());

    // Payload addresses cannot provide identity for zero-width records, while
    // the store's identity must continue to distinguish their address spaces.
    let left = BlockStore::<0>::pack_unique(HashSet::from([[]]), NonZeroUsize::new(1).unwrap());
    let right = BlockStore::<0>::pack_unique(HashSet::from([[]]), NonZeroUsize::new(1).unwrap());
    assert!(std::ptr::eq(
        left.blocks().next().unwrap().rows().next().unwrap().1,
        right.blocks().next().unwrap().rows().next().unwrap().1,
    ));
    assert_ne!(left.identity(), right.identity());
}

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

fn check_range_summary<S: RangeSummary<2>>() {
    let universe = [[-1, -1], [-1, 1], [1, -1], [1, 1]];
    for mask in 0..16 {
        let keys: Vec<_> = universe
            .into_iter()
            .enumerate()
            .filter_map(|(index, key)| (mask & (1 << index) != 0).then_some(key))
            .collect();
        let mut summary = S::default();
        for key in &keys {
            summary.observe(key);
        }
        for columns in [[0, 1], [1, 0]] {
            for a_lo in -2..=2 {
                for a_hi in a_lo..=2 {
                    for b_lo in -2..=2 {
                        for b_hi in b_lo..=2 {
                            let bounds = [(a_lo, a_hi), (b_lo, b_hi)];
                            let exact_possible = keys.iter().any(|key| {
                                columns.iter().zip(key).all(|(&column, &value)| {
                                    let (lo, hi) = bounds[column];
                                    lo <= value && value <= hi
                                })
                            });
                            assert!(!exact_possible || summary.may_match(&bounds, &columns));
                            if keys.is_empty() {
                                assert!(!summary.may_match(&bounds, &columns));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn range_summary_rejection_has_no_false_negatives_for_composite_keys() {
    check_range_summary::<ZoneSummary<2>>();
    let mut unit = ZoneSummary::<0>::default();
    assert!(!RangeSummary::may_match(&unit, &[], &[]));
    RangeSummary::observe(&mut unit, &[]);
    assert!(RangeSummary::may_match(&unit, &[], &[]));
}
