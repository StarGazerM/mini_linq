//! Each representation is admissible only because an abstraction function `α`
//! commutes with its operations. These tests state `α` through the public
//! traits and check the squares exhaustively over small universes.

use cq_storage::{
    BlockDescriptor, BlockMask, BlockRead, BlockSelection, BlockStore, BuildBlocks, BuildIndex,
    BuildReplay, CoordinateBounds, HashIndex, IndexRead, MakeSelection, Replay, ReplayRead,
};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;

type Bag<T> = BTreeMap<T, usize>;

fn bag<T: Ord>(items: impl IntoIterator<Item = T>) -> Bag<T> {
    let mut bag = Bag::new();
    for item in items {
        *bag.entry(item).or_default() += 1;
    }
    bag
}

/// Every subset of `universe`, as the bit patterns of a small universe.
fn subsets<T: Copy>(universe: &[T]) -> impl Iterator<Item = Vec<T>> + '_ {
    (0..1u32 << universe.len()).map(move |mask| {
        universe
            .iter()
            .enumerate()
            .filter_map(|(i, &x)| (mask & (1 << i) != 0).then_some(x))
            .collect()
    })
}

fn alpha_store<const N: usize>(store: &BlockStore<N>) -> BTreeSet<[i64; N]> {
    store
        .blocks()
        .flat_map(|block| block.rows().map(|(_, row)| *row))
        .collect()
}

fn alpha_selection<const N: usize>(
    store: &BlockStore<N>,
    selection: &impl BlockSelection<BlockId = usize, RowId = usize>,
) -> BTreeSet<[i64; N]> {
    store
        .blocks()
        .flat_map(|block| {
            let id = block.id();
            block
                .rows()
                .filter(move |(row, _)| selection.contains(id, *row))
                .map(|(_, row)| *row)
        })
        .collect()
}

#[test]
fn block_store_abstraction_is_the_supplied_set_partitioned_into_enclosed_blocks() {
    let universe = [[0, 0], [0, 1], [1, 0], [1, 1], [2, -1]];
    for rows in subsets(&universe) {
        for block_rows in 1..=3 {
            let size = NonZeroUsize::new(block_rows).unwrap();
            // Duplicated input still abstracts to the set.
            let duplicated = rows.iter().chain(rows.iter()).copied();
            let expected: BTreeSet<_> = rows.iter().copied().collect();
            let store = BlockStore::<2>::from_rows(duplicated, size);
            assert_eq!(alpha_store(&store), expected);
            assert_eq!(
                alpha_store(&BlockStore::pack_unique(rows.clone(), size)),
                expected
            );

            // Coordinates partition α: each row appears at exactly one coordinate.
            let coordinates: Vec<_> = store
                .blocks()
                .flat_map(|b| b.rows().map(move |(r, _)| (b.id(), r)))
                .collect();
            assert_eq!(coordinates.len(), expected.len());
            for block in store.blocks() {
                assert!(block.rows().count() <= block_rows);
                for (_, row) in block.rows() {
                    for (&value, &(lo, hi)) in row.iter().zip(block.bounds()) {
                        assert!(lo <= value && value <= hi);
                    }
                }
            }
        }
    }
}

#[test]
fn selection_abstraction_is_a_subset_grown_one_row_per_insert() {
    let universe = [[0, 0], [0, 1], [1, 0], [1, 1], [2, -1]];
    for rows in subsets(&universe) {
        let store = BlockStore::<2>::pack_unique(rows.clone(), NonZeroUsize::new(2).unwrap());
        let all = BlockMask::all(&store);
        assert_eq!(alpha_selection(&store, &all), alpha_store(&store));
        assert_eq!(all.len(), rows.len());

        let mut selection = BlockMask::empty(&store);
        assert!(alpha_selection(&store, &selection).is_empty());
        let mut expected = BTreeSet::new();
        for block in store.blocks() {
            for (offset, row) in block.rows() {
                assert!(selection.insert(block.id(), offset));
                assert!(!selection.insert(block.id(), offset));
                expected.insert(*row);
                assert_eq!(alpha_selection(&store, &selection), expected);
                assert_eq!(selection.len(), expected.len());
                assert!(selection.block_nonempty(block.id()));
            }
        }
        assert_eq!(alpha_selection(&store, &selection), alpha_store(&store));
    }
}

#[test]
fn index_abstraction_is_the_source_bag_and_lookup_is_exact_selection_in_order() {
    let universe = [(0, 'a'), (0, 'b'), (1, 'a'), (1, 'c'), (2, 'a')];
    for pairs in subsets(&universe) {
        // Multiplicity: duplicate every second pair.
        let source: Vec<_> = pairs
            .iter()
            .enumerate()
            .flat_map(|(i, &p)| std::iter::repeat_n(p, 1 + i % 2))
            .collect();
        let index = HashIndex::build(source.iter().map(|&(k, v)| (k, v)));
        let mut alpha = Bag::new();
        for key in -1..=3 {
            let items: Vec<_> = index.lookup(&key).collect();
            let expected: Vec<_> = source
                .iter()
                .filter(|(k, _)| *k == key)
                .map(|&(_, v)| v)
                .collect();
            assert_eq!(
                items, expected,
                "lookup({key}) must be σ_key in source order"
            );
            for item in items {
                *alpha.entry((key, item)).or_default() += 1;
            }
        }
        assert_eq!(alpha, bag(source.iter().copied()));
    }
}

#[test]
fn replay_abstraction_is_the_source_sequence_for_every_cursor() {
    let universe = [1, 1, 2, 3, 3];
    for prefix_len in 0..=universe.len() {
        let source = &universe[..prefix_len];
        let replay = Replay::replay(source.iter().copied());
        let first: Vec<_> = replay.cursor().take(prefix_len / 2).collect();
        assert_eq!(first, source[..prefix_len / 2]);
        assert_eq!(replay.cursor().collect::<Vec<_>>(), source);
        assert_eq!(replay.clone().cursor().collect::<Vec<_>>(), source);
    }
}

#[test]
fn traversals_enumerate_exactly_the_selected_or_matching_rows() {
    use cq_storage::{RangeSummary, ZoneSummary, pruned_rows, selected_rows};
    let universe = [[0, 0], [0, 1], [1, 0], [1, 1], [2, -1], [3, 3]];
    let store = BlockStore::<2>::pack_unique(universe, NonZeroUsize::new(2).unwrap());
    for rows in subsets(&universe) {
        let chosen: BTreeSet<_> = rows.iter().copied().collect();
        let mut selection = BlockMask::empty(&store);
        for block in store.blocks() {
            for (offset, row) in block.rows() {
                if chosen.contains(row) {
                    selection.insert(block.id(), offset);
                }
            }
        }
        let selected: BTreeSet<_> = selected_rows(&store, &selection)
            .map(|(_, _, row)| *row)
            .collect();
        assert_eq!(selected, chosen);

        // Semijoin residual: rows whose column-1 value is in a key set.
        let keys: BTreeSet<[i64; 1]> = rows.iter().map(|row| [row[1]]).collect();
        let mut summary = ZoneSummary::<1>::default();
        for key in &keys {
            summary.observe(key);
        }
        let matched: BTreeSet<_> = pruned_rows(&store, &summary, [1], &keys)
            .map(|(_, _, row)| *row)
            .collect();
        let expected: BTreeSet<_> = universe
            .into_iter()
            .filter(|row| keys.contains(&[row[1]]))
            .collect();
        assert_eq!(matched, expected);
    }
}
