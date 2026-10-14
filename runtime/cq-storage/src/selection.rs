//! Immutable position snapshots; retained bytes exclude the borrowed base.
//! Masks use 8 * ceil(rows / 64) bytes, plus optional per-block bitmap words.
//! Construction/narrowing initialize the full mask and enumerate candidate
//! positions. Construction tests keys in retained blocks; narrowing tests only
//! previously selected IDs in retained blocks.
//! Rows iterate bitmap words and survivors, fetching payload only for survivors.
use crate::{BlockRead, Build, Column, PositionalRows, ReadPredicate, RowId};
#[derive(Clone, Copy)]
pub enum SelectionSummary {
    None,
    PerBlock,
}
pub struct Selection<H: PositionalRows> {
    host: H,
    bits: Vec<u64>,
    blocks: Option<Vec<u64>>,
    _allocation: crate::counters_memory::Allocation,
}
impl<H: PositionalRows> Selection<H> {
    fn empty(host: H, summary: SelectionSummary) -> Self {
        let bits = vec![0; host.len().div_ceil(64)];
        let blocks = match summary {
            SelectionSummary::None => None,
            SelectionSummary::PerBlock => Some(vec![
                0;
                host.len()
                    .div_ceil(host.block_size().get())
                    .div_ceil(64)
            ]),
        };
        let bytes = 8 * (bits.capacity() + blocks.as_ref().map_or(0, Vec::capacity));
        Self {
            host,
            bits,
            blocks,
            _allocation: crate::counters_memory::Allocation::new(bytes),
        }
    }
    fn insert(&mut self, id: RowId) {
        assert!(
            id.offset() < self.host.len(),
            "read must return live host IDs"
        );
        let i = id.offset();
        self.bits[i / 64] |= 1 << (i % 64);
        if let Some(blocks) = &mut self.blocks {
            let b = i / self.host.block_size().get();
            blocks[b / 64] |= 1 << (b % 64);
        }
    }
    pub fn select<C, P>(host: H, column: C, predicate: &P, summary: SelectionSummary) -> Self
    where
        C: Column<H>,
        H: BlockRead<C>,
        P: ReadPredicate<C::Key>,
    {
        let mut result = Self::empty(host, summary);
        for id in host.read(column, predicate) {
            result.insert(id);
        }
        result
    }
    /// Cardinality from bitmap words, used only by measurement builds.
    #[cfg(any(feature = "counters", feature = "operator-timing"))]
    pub fn profile_len(&self) -> u64 {
        self.bits
            .iter()
            .map(|word| u64::from(word.count_ones()))
            .sum()
    }
    pub fn positions(&self) -> impl Iterator<Item = RowId> + '_ {
        self.host.positions().filter(|&id| self.contains(id))
    }
    pub fn rows(&self) -> SelectionRows<'_, H> {
        SelectionRows {
            selection: self,
            cursor: 0,
        }
    }
    pub fn contains(&self, id: RowId) -> bool {
        id.offset() < self.host.len()
            && self.bits[id.offset() / 64] & (1 << (id.offset() % 64)) != 0
    }
    pub fn narrow<C, P>(&self, column: C, predicate: &P, summary: SelectionSummary) -> Self
    where
        C: Column<H>,
        H: BlockRead<C>,
        P: ReadPredicate<C::Key>,
    {
        let mut result = Self::empty(self.host, summary);
        for block in self.host.blocks() {
            crate::count!(blocks_considered);
            let empty = self.blocks.as_ref().map_or_else(
                || !self.host.block_positions(block).any(|id| self.contains(id)),
                |bits| bits[block.offset() / 64] & (1 << (block.offset() % 64)) == 0,
            );
            if empty {
                crate::count!(blocks_skipped);
                crate::count!(blocks_skipped_empty_selection);
                continue;
            }
            if self
                .host
                .bounds(column, block)
                .is_some_and(|(lo, hi)| !predicate.any_in(&lo, &hi))
            {
                crate::count!(blocks_skipped);
                crate::count!(blocks_skipped_metadata);
                continue;
            }
            crate::count!(blocks_visited);
            for id in self
                .host
                .block_positions(block)
                .filter(|&id| self.contains(id))
            {
                crate::count!(rows_tested);
                if predicate.test(&column.key(self.host, id)) {
                    result.insert(id);
                }
            }
        }
        result
    }
    pub fn build<'a, F, K, V, T>(&'a self, pair: F) -> T
    where
        F: FnMut(H::Row) -> (K, V),
        T: Build<std::iter::Map<SelectionRows<'a, H>, F>>,
    {
        T::build(self.rows().map(pair))
    }
    pub fn retained_bytes(&self) -> usize {
        8 * (self.bits.capacity() + self.blocks.as_ref().map_or(0, Vec::capacity))
    }
}
pub struct SelectionRows<'a, H: PositionalRows> {
    selection: &'a Selection<H>,
    cursor: usize,
}
impl<H: PositionalRows> Iterator for SelectionRows<'_, H> {
    type Item = H::Row;
    fn next(&mut self) -> Option<Self::Item> {
        while self.cursor < self.selection.host.len() {
            let word_index = self.cursor / 64;
            let word = self.selection.bits[word_index] & (u64::MAX << (self.cursor % 64));
            if word == 0 {
                self.cursor = (word_index + 1).saturating_mul(64);
                continue;
            }
            let offset = word_index * 64 + word.trailing_zeros() as usize;
            self.cursor = offset + 1;
            let row = self.selection.host.row_at(RowId(offset));
            if row.is_some() {
                crate::count!(rows_enumerated);
            }
            return row;
        }
        None
    }
}
impl<H: PositionalRows> std::iter::FusedIterator for SelectionRows<'_, H> {}
