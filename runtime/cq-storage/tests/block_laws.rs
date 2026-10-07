//! A test-only generic host: no cq-load or concrete zone-map implementation.
use cq_storage::rows::Rows;
use cq_storage::*;
use std::{cell::Cell, num::NonZeroUsize};
#[derive(Clone, Copy)]
struct Host<'a> {
    rows: &'a [(i64,)],
    reads: &'a Cell<usize>,
    unknown: bool,
}
impl<'a> Rows for Host<'a> {
    type Row = (&'a i64,);
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.rows.iter().map(|r| (&r.0,))
    }
}
impl PositionalRows for Host<'_> {
    fn len(self) -> usize {
        self.rows.len()
    }
    fn row_at(self, id: RowId) -> Option<Self::Row> {
        self.rows.get(id.offset()).map(|r| (&r.0,))
    }
    fn block_size(self) -> NonZeroUsize {
        NonZeroUsize::new(2).unwrap()
    }
}
#[derive(Clone, Copy)]
struct Col;
impl Column<Host<'_>> for Col {
    type Key = i64;
    fn key(self, host: Host<'_>, id: RowId) -> i64 {
        host.reads.set(host.reads.get() + 1);
        *host.row_at(id).unwrap().0
    }
}
impl BlockRead<Col> for Host<'_> {
    fn bounds(self, _: Col, block: BlockId) -> Option<(i64, i64)> {
        if self.unknown {
            return None;
        }
        let values: Vec<_> = self
            .block_positions(block)
            .map(|id| *self.row_at(id).unwrap().0)
            .collect();
        Some((*values.iter().min()?, *values.iter().max()?))
    }
}
#[test]
fn unknown_bounds_partial_blocks_and_empty_selection_skips_are_sound() {
    let source = [(0,), (1,), (10,), (11,), (20,)];
    let reads = Cell::new(0);
    for unknown in [false, true] {
        let host = Host {
            rows: &source,
            reads: &reads,
            unknown,
        };
        reads.set(0);
        let selected = Selection::select(host, Col, &Point(20), SelectionSummary::PerBlock);
        assert_eq!(selected.rows().collect::<Vec<_>>(), [(&20,)]);
        assert_eq!(reads.get(), if unknown { 5 } else { 1 });
        reads.set(0);
        let narrow = selected.narrow(Col, &Point(0), SelectionSummary::PerBlock);
        assert_eq!(narrow.rows().next(), None);
        assert_eq!(reads.get(), usize::from(unknown));
        assert_eq!(selected.rows().collect::<Vec<_>>(), [(&20,)]);
        let other = Host {
            rows: &[(9,)],
            reads: &reads,
            unknown,
        };
        let outside = host.positions().last().unwrap();
        assert!(other.row_at(outside).is_none());
        let tiny = Selection::select(other, Col, &Point(9), SelectionSummary::None);
        assert!(!tiny.contains(outside));
    }
}
#[cfg(feature = "counters")]
#[test]
fn skip_partition_and_memory_lifetimes_are_observable() {
    use cq_storage::counters;
    let source = [(0,), (1,), (10,), (11,), (20,)];
    let reads = Cell::new(0);
    let host = Host {
        rows: &source,
        reads: &reads,
        unknown: false,
    };
    counters::reset();
    let selected = Selection::select(host, Col, &Point(20), SelectionSummary::PerBlock);
    let first = counters::snapshot();
    assert_eq!(
        (
            first.blocks_considered,
            first.blocks_visited,
            first.blocks_skipped,
            first.rows_tested
        ),
        (3, 1, 2, 1)
    );
    assert_eq!(first.rows_copied, 0);
    assert_eq!(
        first.bytes_retained_current,
        selected.retained_bytes() as u64
    );
    let narrowed = selected.narrow(Col, &Point(0), SelectionSummary::PerBlock);
    let second = counters::snapshot();
    assert_eq!(
        second.blocks_considered,
        second.blocks_visited + second.blocks_skipped
    );
    assert_eq!(second.blocks_skipped_empty_selection, 2);
    assert_eq!(second.blocks_skipped_metadata, 3);
    drop(narrowed);
    drop(selected);
    assert_eq!(counters::snapshot().bytes_retained_current, 0);
    let dense = AdaptiveKeySet::<_, _, 65>::from_keys([0i64, 64], I64Codec);
    let c = counters::snapshot();
    assert_eq!(c.bytes_retained_current, 16);
    assert!(c.bytes_retained_peak >= c.keyset_staging_bytes_peak + 16);
    drop(dense);
    assert_eq!(counters::snapshot().bytes_retained_current, 0);
}
