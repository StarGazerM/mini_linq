//! Immutable integer chunks with a separate metadata directory.

use crate::{BlockDescriptor, BlockRead, BuildBlocks, CoordinateBounds, Range};
use std::collections::HashSet;
use std::num::NonZeroUsize;

#[derive(Debug)]
struct Metadata<const N: usize> {
    len: usize,
    bounds: [Range; N],
}

/// Deduplicated, immutable chunks of fixed-width integer records.
///
/// The metadata directory is separate from the payload chunks. Enumerating
/// descriptors, checking lengths, and reading bounds do not enumerate payload
/// rows. Bounds enclose every row of their chunk; there is no operation that can
/// mutate the payload independently of those bounds.
#[derive(Debug)]
pub struct BlockStore<const N: usize> {
    metadata: Vec<Metadata<N>>,
    payload: Vec<Box<[[i64; N]]>>,
}

impl<const N: usize> BlockStore<N> {
    /// Pack distinct rows in first-occurrence order.
    pub fn from_rows(rows: impl IntoIterator<Item = [i64; N]>, block_rows: NonZeroUsize) -> Self {
        let mut seen = HashSet::new();
        Self::pack_rows(rows.into_iter().filter(|row| seen.insert(*row)), block_rows)
    }

    fn pack_rows(rows: impl IntoIterator<Item = [i64; N]>, block_rows: NonZeroUsize) -> Self {
        let mut metadata: Vec<Metadata<N>> = Vec::new();
        let mut chunks: Vec<Vec<[i64; N]>> = Vec::new();
        for row in rows {
            if metadata
                .last()
                .is_none_or(|block| block.len == block_rows.get())
            {
                metadata.push(Metadata {
                    len: 0,
                    bounds: row.map(|value| (value, value)),
                });
                chunks.push(Vec::new());
            }
            let block = metadata.last_mut().unwrap();
            for (range, value) in block.bounds.iter_mut().zip(row) {
                range.0 = range.0.min(value);
                range.1 = range.1.max(value);
            }
            block.len += 1;
            chunks.last_mut().unwrap().push(row);
        }
        Self {
            metadata,
            payload: chunks.into_iter().map(Vec::into_boxed_slice).collect(),
        }
    }

    // Construction bridge for the bitmap implementation; not a public access API.
    pub(crate) fn block_lengths(
        &self,
    ) -> impl ExactSizeIterator<Item = usize> + DoubleEndedIterator {
        self.metadata.iter().map(|block| block.len)
    }
}

/// A metadata descriptor that can explicitly open its associated payload.
#[derive(Clone, Copy, Debug)]
pub struct Block<'a, const N: usize> {
    store: &'a BlockStore<N>,
    id: usize,
}

impl<const N: usize> BlockRead for BlockStore<N> {
    type Row = [i64; N];
    type BlockId = usize;
    type RowId = usize;
    type Identity = *const Self;
    type Descriptor<'a> = Block<'a, N>;

    fn identity(&self) -> Self::Identity {
        // BlockStore itself is not zero sized, including for zero-column rows.
        // Its address is stable for the duration of the shared borrow.
        std::ptr::from_ref(self)
    }

    fn blocks(&self) -> impl Iterator<Item = Self::Descriptor<'_>> {
        (0..self.metadata.len()).map(|id| Block { store: self, id })
    }
}

impl<'a, const N: usize> BlockDescriptor<'a> for Block<'a, N> {
    type Store = BlockStore<N>;

    fn store(&self) -> &'a Self::Store {
        self.store
    }

    fn id(&self) -> usize {
        self.id
    }

    fn rows(self) -> impl Iterator<Item = (usize, &'a [i64; N])> {
        self.store.payload[self.id].iter().enumerate()
    }
}

impl<const N: usize> CoordinateBounds for Block<'_, N> {
    fn bounds(&self) -> &[(i64, i64)] {
        &self.store.metadata[self.id].bounds
    }
}

impl<const N: usize> BuildBlocks for BlockStore<N> {
    fn pack_unique(rows: HashSet<Self::Row>, block_rows: NonZeroUsize) -> Self {
        Self::pack_rows(rows, block_rows)
    }
}
