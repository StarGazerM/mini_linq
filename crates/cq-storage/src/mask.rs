//! Bit storage indexed by a block number and an offset inside that block.

use crate::{BlockSelection, BlockStore, MakeSelection};

#[derive(Clone, Debug)]
struct Bits {
    len: usize,
    words: Vec<u64>,
    count: usize,
}

/// An independent bitmap with a fixed, possibly irregular block shape.
///
/// Coordinates carry no storage identity or relational meaning. A caller owns
/// the association between a mask and another data structure. Zero-length
/// blocks are supported. Coordinate access panics outside the declared shape.
#[derive(Clone, Debug)]
pub struct BlockMask {
    blocks: Vec<Bits>,
    count: usize,
}

impl BlockMask {
    fn with_fill(lengths: impl IntoIterator<Item = usize>, fill: bool) -> Self {
        let blocks: Vec<_> = lengths
            .into_iter()
            .map(|len| {
                let word_count = len / 64 + usize::from(len % 64 != 0);
                let mut words = vec![if fill { u64::MAX } else { 0 }; word_count];
                if fill && len % 64 != 0 {
                    *words.last_mut().unwrap() = (1 << (len % 64)) - 1;
                }
                Bits {
                    len,
                    words,
                    count: if fill { len } else { 0 },
                }
            })
            .collect();
        let count = blocks.iter().map(|block| block.count).sum();
        Self { blocks, count }
    }
}

impl BlockSelection for BlockMask {
    type BlockId = usize;
    type RowId = usize;

    /// Set a coordinate. Returns whether it was previously unset.
    fn insert(&mut self, block: usize, row: usize) -> bool {
        let bits = &mut self.blocks[block];
        assert!(row < bits.len, "row offset exceeds block mask shape");
        let word = &mut bits.words[row / 64];
        let bit = 1 << (row % 64);
        if *word & bit != 0 {
            return false;
        }
        *word |= bit;
        bits.count += 1;
        self.count += 1;
        true
    }

    fn contains(&self, block: usize, row: usize) -> bool {
        let bits = &self.blocks[block];
        assert!(row < bits.len, "row offset exceeds block mask shape");
        bits.words[row / 64] & (1 << (row % 64)) != 0
    }

    fn block_nonempty(&self, block: usize) -> bool {
        self.blocks[block].count != 0
    }

    /// Number of set coordinates, excluding unused bits in partial words.
    fn len(&self) -> usize {
        self.count
    }
}

impl<const N: usize> MakeSelection<BlockStore<N>> for BlockMask {
    fn all(store: &BlockStore<N>) -> Self {
        Self::with_fill(store.block_lengths(), true)
    }

    fn empty(store: &BlockStore<N>) -> Self {
        Self::with_fill(store.block_lengths(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn masks_preserve_irregular_shapes_across_word_boundaries() {
        let lengths = [0, 1, 63, 64, 65, 129];
        let all = BlockMask::with_fill(lengths, true);
        let mut selected = BlockMask::with_fill(lengths, false);
        assert_eq!(all.len(), lengths.iter().sum());
        assert!(selected.is_empty());
        assert!(!all.block_nonempty(0));

        let mut expected = HashSet::new();
        for (block, len) in lengths.into_iter().enumerate() {
            for row in 0..len {
                assert!(all.contains(block, row));
                assert!(!selected.contains(block, row));
                if row % 3 == 0 {
                    assert!(selected.insert(block, row));
                    assert!(!selected.insert(block, row));
                    expected.insert((block, row));
                }
            }
        }
        assert_eq!(selected.len(), expected.len());
        for (block, len) in lengths.into_iter().enumerate() {
            assert_eq!(selected.block_nonempty(block), len != 0);
            for row in 0..len {
                assert_eq!(
                    selected.contains(block, row),
                    expected.contains(&(block, row))
                );
            }
        }
    }

    #[test]
    fn masks_are_independent_storage_without_payload_or_version_ownership() {
        let original = BlockMask::with_fill([2, 0, 3], false);
        let mut first = original.clone();
        let mut second = original.clone();
        first.insert(0, 1);
        second.insert(2, 2);
        assert!(original.is_empty());
        assert!(first.contains(0, 1));
        assert!(!second.contains(0, 1));
        assert!(!first.block_nonempty(2));
        assert!(second.block_nonempty(2));
        assert!(BlockMask::with_fill([], true).is_empty());
        assert!(BlockMask::with_fill([0, 0], true).is_empty());
    }

    #[test]
    #[should_panic(expected = "row offset exceeds block mask shape")]
    fn unused_trailing_bits_are_not_valid_coordinates() {
        BlockMask::with_fill([65], true).contains(0, 65);
    }

    #[test]
    #[should_panic(expected = "row offset exceeds block mask shape")]
    fn inserting_into_an_empty_block_is_rejected() {
        BlockMask::with_fill([0], false).insert(0, 0);
    }
}
