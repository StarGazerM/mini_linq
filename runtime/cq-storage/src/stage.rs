//! Bounded lookahead for total positional stages. Partial operations consume
//! survivors only after all total selections have completed.
use crate::{PositionalRows, RowId};
use std::ops::Range;
use std::sync::{Arc, Mutex};

/// A contiguous base range and its ordered selection of live occurrences.
/// Selecting again intersects the selection without reintroducing rejected IDs.
pub struct Chunk {
    range: Range<RowId>,
    selection: Vec<RowId>,
    recycle: Arc<Mutex<Option<Vec<RowId>>>>,
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

impl Drop for Chunk {
    fn drop(&mut self) {
        let mut buffer = self.recycle.lock().expect("stage buffer lock poisoned");
        if buffer.is_none() {
            *buffer = Some(std::mem::take(&mut self.selection));
        }
    }
}

/// Consuming survivor cursor; dropping it returns the selection buffer.
pub struct Survivors {
    chunk: Chunk,
    next: usize,
}

impl Iterator for Survivors {
    type Item = RowId;

    fn next(&mut self) -> Option<RowId> {
        let id = self.chunk.selection.get(self.next).copied()?;
        self.next += 1;
        Some(id)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.chunk.selection.len() - self.next;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for Survivors {}

impl IntoIterator for Chunk {
    type Item = RowId;
    type IntoIter = Survivors;

    fn into_iter(self) -> Survivors {
        Survivors {
            chunk: self,
            next: 0,
        }
    }
}

/// Partition an immutable positional host into nonempty chunks of at most `N`
/// occurrences. Construction is lazy; each demanded chunk counts as one
/// considered and visited staging block. Row tests are counted by `select_with`.
/// Sequential consumers reuse one selection buffer. Holding several chunks
/// concurrently gives each its own buffer until it is dropped.
///
/// # Panics
/// Panics if `N` is zero.
pub fn for_chunks<const N: usize, H: PositionalRows>(host: H) -> impl Iterator<Item = Chunk> {
    assert!(N > 0, "stage chunk size must be positive");
    let len = host.len();
    let recycle = Arc::new(Mutex::new(Some(Vec::with_capacity(N.min(len)))));
    (0..len).step_by(N).map(move |start| {
        crate::count!(blocks_considered);
        crate::count!(blocks_visited);
        let end = start.saturating_add(N).min(len);
        let mut selection = recycle
            .lock()
            .expect("stage buffer lock poisoned")
            .take()
            .unwrap_or_default();
        selection.clear();
        selection.extend((start..end).map(RowId));
        Chunk {
            range: RowId(start)..RowId(end),
            selection,
            recycle: Arc::clone(&recycle),
        }
    })
}

/// Bounded total lookahead for retained rows and total membership segments.
/// One deque is reused; downstream partial steps receive rows only after fill.
///
/// # Panics
/// Panics if `N` is zero.
pub fn buffered<const N: usize, I: Iterator>(mut rows: I) -> impl Iterator<Item = I::Item> {
    assert!(N > 0, "stage chunk size must be positive");
    let mut buffer = std::collections::VecDeque::with_capacity(N);
    std::iter::from_fn(move || {
        if buffer.is_empty() {
            buffer.extend(rows.by_ref().take(N));
        }
        buffer.pop_front()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_cursors_remain_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Chunk>();
        assert_send_sync::<Survivors>();
    }

    #[test]
    fn sequential_chunks_reuse_the_selection_allocation() {
        let rows = [(0,); 19];
        let mut chunks = for_chunks::<8, _>(&rows);
        let first = chunks.next().unwrap();
        let allocation = first.selection.as_ptr();
        let mut survivors = first.into_iter();
        assert_eq!(survivors.next(), Some(RowId(0)));
        drop(survivors);
        let mut second = chunks.next().unwrap();
        assert_eq!(second.selection.as_ptr(), allocation);
        second.select_with(|id| id.0 % 2 == 0);
        assert_eq!(
            second.into_iter().collect::<Vec<_>>(),
            vec![RowId(8), RowId(10), RowId(12), RowId(14)]
        );
        assert_eq!(chunks.next().unwrap().selection.as_ptr(), allocation);
    }

    #[test]
    fn simultaneously_held_chunks_keep_independent_selections() {
        let rows = [(0,); 6];
        let mut chunks = for_chunks::<3, _>(&rows);
        let mut first = chunks.next().unwrap();
        let second = chunks.next().unwrap();
        first.select_with(|id| id.0 == 1);
        assert_eq!(first.survivors().collect::<Vec<_>>(), vec![RowId(1)]);
        assert_eq!(
            second.survivors().collect::<Vec<_>>(),
            vec![RowId(3), RowId(4), RowId(5)]
        );
    }

    #[test]
    fn total_buffer_fills_before_yielding_and_keeps_order() {
        let visited = std::cell::Cell::new(0);
        let mut rows = buffered::<4, _>((0..9).inspect(|_| visited.set(visited.get() + 1)));
        assert_eq!(rows.next(), Some(0));
        assert_eq!(visited.get(), 4);
        assert_eq!(rows.collect::<Vec<_>>(), (1..9).collect::<Vec<_>>());
    }
}
