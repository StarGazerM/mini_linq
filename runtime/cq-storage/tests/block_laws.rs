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

#[test]
fn owned_predicates_escape_their_scope_without_reads_or_buffers() {
    struct Threshold(i64);
    impl ReadPredicate<i64> for Threshold {
        fn test(&self, key: &i64) -> bool {
            *key >= self.0
        }
        fn any_in(&self, _lo: &i64, hi: &i64) -> bool {
            *hi >= self.0
        }
    }
    let data = [(-3,), (-2,), (0,), (1,), (4,), (5,), (9,)];
    for unknown in [false, true] {
        let reads = Cell::new(0);
        let host = Host {
            rows: &data,
            reads: &reads,
            unknown,
        };
        let mut ids = {
            let predicate = Threshold(4);
            host.read_owned(Col, predicate)
        };
        assert_eq!(reads.get(), 0);
        assert_eq!(ids.next().unwrap().offset(), 4);
        assert_eq!(reads.get(), if unknown { 5 } else { 1 });
        assert_eq!(ids.map(RowId::offset).collect::<Vec<_>>(), vec![5, 6]);
        assert_eq!(reads.get(), if unknown { 7 } else { 3 });
    }
}

#[test]
fn chunk_masks_are_scalar_tests_anded_with_the_existing_selection() {
    use std::ops::Bound::{Excluded, Included};

    fn check<P: ReadPredicate<i64>>(predicate: P) {
        let keys = [-2, 0, 0, 1, 4];
        for len in 0..=keys.len() {
            for bits in 0..1usize << len {
                let initial: Vec<_> = (0..len).map(|i| bits & (1 << i) != 0).collect();
                let expected: Vec<_> = keys[..len]
                    .iter()
                    .zip(&initial)
                    .map(|(key, selected)| *selected && predicate.test(key))
                    .collect();
                let mut mask = initial;
                predicate.test_chunk(&keys[..len], &mut mask);
                assert_eq!(mask, expected);
                Point(0).test_chunk(&keys[..len], &mut mask);
                assert_eq!(
                    mask,
                    expected
                        .iter()
                        .zip(&keys[..len])
                        .map(|(selected, key)| *selected && *key == 0)
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    check(Point(0));
    check(Window {
        low: Excluded(-2),
        high: Included(1),
    });
    let membership = AdaptiveKeySet::<_, _, 64>::from_keys([-2i64, 1], I64Codec);
    check(Membership(&membership));
    check(RowPredicate(|key: &i64| key % 2 == 0));
}

#[test]
fn chunked_reads_equal_owned_reads_in_order_and_metadata_pruning() {
    use std::ops::Bound::{Excluded, Included, Unbounded};

    fn check<P: ReadPredicate<i64>>(host: Host<'_>, predicate: P) {
        host.reads.set(0);
        let scalar: Vec<_> = host.read_owned(Col, &predicate).collect();
        let scalar_reads = host.reads.get();
        host.reads.set(0);
        let mut chunked = host.read_chunked(Col, &predicate);
        assert_eq!(host.reads.get(), 0, "construction is lazy");
        assert_eq!(chunked.by_ref().collect::<Vec<_>>(), scalar);
        assert_eq!(chunked.next(), None);
        assert_eq!(host.reads.get(), scalar_reads);
        for block in host.blocks() {
            let keys = host.column_chunk(Col, block);
            assert_eq!(keys.len(), host.block_positions(block).count());
            assert_eq!(
                keys.collect::<Vec<_>>(),
                host.block_positions(block)
                    .map(|id| Col.key(host, id))
                    .collect::<Vec<_>>()
            );
        }
    }

    let data = [(-3,), (0,), (0,), (1,), (4,), (5,), (9,)];
    let reads = Cell::new(0);
    let membership = AdaptiveKeySet::<_, _, 64>::from_keys([0i64, 5], I64Codec);
    for unknown in [false, true] {
        for len in 0..=data.len() {
            let host = Host {
                rows: &data[..len],
                reads: &reads,
                unknown,
            };
            for point in -4..=10 {
                check(host, Point(point));
            }
            check(host, Membership(&membership));
            check(host, RowPredicate(|key: &i64| key % 2 == 0));
            for (low, high) in [
                (Unbounded, Unbounded),
                (Included(0), Excluded(5)),
                (Excluded(0), Included(5)),
                (Included(9), Included(-3)),
            ] {
                check(host, Window { low, high });
            }
        }
    }
}

#[test]
fn borrowed_chunk_predicates_preserve_the_override_and_block_lookahead() {
    struct CountChunks<'a>(&'a Cell<usize>);
    impl ReadPredicate<i64> for CountChunks<'_> {
        fn test(&self, key: &i64) -> bool {
            *key >= 0
        }
        fn test_chunk(&self, keys: &[i64], mask: &mut [bool]) {
            self.0.set(self.0.get() + 1);
            for (key, selected) in keys.iter().zip(mask) {
                *selected &= self.test(key);
            }
        }
    }
    let reads = Cell::new(0);
    let calls = Cell::new(0);
    let host = Host {
        rows: &[(0,), (1,), (2,)],
        reads: &reads,
        unknown: true,
    };
    let predicate = CountChunks(&calls);
    let mut ids = host.read_chunked(Col, &predicate);
    assert_eq!(calls.get(), 0);
    assert_eq!(ids.next().unwrap().offset(), 0);
    assert_eq!((calls.get(), reads.get()), (1, 2));
    assert_eq!(ids.next().unwrap().offset(), 1);
    assert_eq!((calls.get(), reads.get()), (1, 2));
    assert_eq!(ids.next().unwrap().offset(), 2);
    assert_eq!((calls.get(), reads.get()), (2, 3));
    assert_eq!(ids.next(), None);
}

#[test]
fn staged_chunks_partition_occurrences_and_intersect_selections() {
    fn check<const N: usize>(host: &[(i64,)]) {
        let mut all = Vec::new();
        let mut selected = Vec::new();
        for mut chunk in for_chunks::<N, _>(host) {
            let range = chunk.range();
            let original: Vec<_> = chunk.survivors().collect();
            assert!(!original.is_empty());
            assert!(original.len() <= N);
            assert_eq!(
                original.iter().map(|id| id.offset()).collect::<Vec<_>>(),
                (range.start.offset()..range.end.offset()).collect::<Vec<_>>()
            );
            all.extend(original);
            chunk.select_with(|id| id.offset() % 2 == 0);
            chunk.select_with(|id| *host.row_at(id).unwrap().0 >= 0);
            assert_eq!(chunk.range(), range);
            selected.extend(chunk.survivors());
        }
        assert_eq!(all, host.positions().collect::<Vec<_>>());
        assert_eq!(
            selected,
            host.positions()
                .filter(|id| id.offset() % 2 == 0 && *host.row_at(*id).unwrap().0 >= 0)
                .collect::<Vec<_>>()
        );
    }
    let data = [(-3,), (0,), (0,), (1,), (4,), (5,), (9,)];
    for len in 0..=data.len() {
        check::<1>(&data[..len]);
        check::<2>(&data[..len]);
        check::<3>(&data[..len]);
        check::<128>(&data[..len]);
    }
}

#[test]
#[should_panic(expected = "stage chunk size must be positive")]
fn zero_sized_staging_chunks_are_rejected() {
    let data: [(i64,); 0] = [];
    let _ = for_chunks::<0, _>(&data);
}

#[cfg(feature = "counters")]
#[test]
fn chunked_read_and_staging_counters_count_actual_tests_and_blocks() {
    use cq_storage::counters;

    let data = [(0,), (1,), (10,), (11,), (20,)];
    let reads = Cell::new(0);
    let host = Host {
        rows: &data,
        reads: &reads,
        unknown: false,
    };
    counters::reset();
    let scalar = host.read_owned(Col, Point(20)).collect::<Vec<_>>();
    let expected = counters::snapshot();
    counters::reset();
    assert_eq!(
        host.read_chunked(Col, Point(20)).collect::<Vec<_>>(),
        scalar
    );
    assert_eq!(counters::snapshot(), expected);

    counters::reset();
    let mut chunks = for_chunks::<2, _>(host);
    assert_eq!(counters::snapshot().blocks_visited, 0);
    let mut first = chunks.next().unwrap();
    assert_eq!(counters::snapshot().blocks_visited, 1);
    first.select_with(|id| id.offset() == 0);
    assert_eq!(counters::snapshot().rows_tested, 2);
    first.select_with(|_| true);
    assert_eq!(counters::snapshot().rows_tested, 3);
    for mut chunk in chunks {
        chunk.select_with(|_| true);
    }
    let counts = counters::snapshot();
    assert_eq!((counts.blocks_considered, counts.blocks_visited), (3, 3));
    assert_eq!(counts.rows_tested, 6);
    assert_eq!(counts.blocks_skipped, 0);
}
