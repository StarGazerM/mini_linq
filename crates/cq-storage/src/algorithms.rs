//! Generic traversals over the access concepts, in the style of algorithms
//! over iterator categories. They add no meaning: each enumerates a subset of
//! `α(store)` fixed by its arguments, and any conservative skip is paired
//! with its exact residual here, so consumers never see approximate answers.

use crate::{
    BlockDescriptor, BlockRead, BlockSelection, CoordinateBounds, ExactMembership, RangeSummary,
};

/// Coordinates and rows of `store` that `selection` contains.
///
/// Blocks with no selected coordinate are not opened.
pub fn selected_rows<'a, S, M>(
    store: &'a S,
    selection: &'a M,
) -> impl Iterator<Item = (S::BlockId, S::RowId, &'a S::Row)> + 'a
where
    S: BlockRead,
    M: BlockSelection<BlockId = S::BlockId, RowId = S::RowId>,
{
    store
        .blocks()
        .filter(|block| selection.block_nonempty(block.id()))
        .flat_map(move |block| {
            let id = block.id();
            block
                .rows()
                .filter(move |(row, _)| selection.contains(id, *row))
                .map(move |(row, payload)| (id, row, payload))
        })
}

/// Rows of `store` whose projection onto `columns` is a member of `keys`.
///
/// `summary` must have observed every key in `keys`; blocks it rejects are
/// never opened. Every opened row is still tested exactly, so the result is
/// independent of how precise the summary is.
pub fn pruned_rows<'a, S, Sum, Keys, const K: usize>(
    store: &'a S,
    summary: &'a Sum,
    columns: [usize; K],
    keys: &'a Keys,
) -> impl Iterator<Item = (S::BlockId, S::RowId, &'a S::Row)> + 'a
where
    S: BlockRead,
    S::Row: AsRef<[i64]>,
    for<'b> S::Descriptor<'b>: CoordinateBounds,
    Sum: RangeSummary<K>,
    Keys: ExactMembership<[i64; K]>,
{
    store
        .blocks()
        .filter(move |block| summary.may_match(block.bounds(), &columns))
        .flat_map(move |block| {
            let id = block.id();
            block
                .rows()
                .filter(move |(_, row)| {
                    let row = row.as_ref();
                    keys.contains(&columns.map(|column| row[column]))
                })
                .map(move |(row, payload)| (id, row, payload))
        })
}
