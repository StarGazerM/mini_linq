//! Bounded lookahead for total positional stages. Partial operations consume
//! survivors only after all total selections have completed.
use crate::{PositionalRows, RowId};
use std::ops::Range;

/// A contiguous base range and its ordered selection of live occurrences.
/// Selecting again intersects the selection without reintroducing rejected IDs.
pub struct Chunk {
    range: Range<RowId>,
    selection: Vec<RowId>,
}

impl Chunk {
    /// The original half-open base range, unchanged by selections.
    pub fn range(&self) -> Range<RowId> {
        self.range.clone()
    }

    /// Test each current survivor once, retaining its original base order.
    pub fn select_with(&mut self, mut f: impl FnMut(RowId) -> bool) {
        self.selection.retain(|&id| {
            crate::count!(rows_tested);
            f(id)
        });
    }

    pub fn survivors(&self) -> impl Iterator<Item = RowId> {
        self.selection.iter().copied()
    }
}

/// Partition an immutable positional host into nonempty chunks of at most `N`
/// occurrences. Construction is lazy; each demanded chunk counts as one
/// considered and visited staging block. Row tests are counted by `select_with`.
///
/// # Panics
/// Panics if `N` is zero.
pub fn for_chunks<const N: usize, H: PositionalRows>(host: H) -> impl Iterator<Item = Chunk> {
    assert!(N > 0, "stage chunk size must be positive");
    let len = host.len();
    (0..len).step_by(N).map(move |start| {
        crate::count!(blocks_considered);
        crate::count!(blocks_visited);
        let end = start.saturating_add(N).min(len);
        Chunk {
            range: RowId(start)..RowId(end),
            selection: (start..end).map(RowId).collect(),
        }
    })
}
