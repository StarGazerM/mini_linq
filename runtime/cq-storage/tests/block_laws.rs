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

#[test]
fn sparse_narrowing_visits_bitmap_survivors_not_base_positions() {
    use std::cell::RefCell;

    #[derive(Clone, Copy)]
    struct SparseHost<'a> {
        rows: &'a [(i64,)],
        size: NonZeroUsize,
        enumerated: &'a Cell<usize>,
        events: &'a RefCell<Vec<(char, usize)>>,
    }
    impl<'a> Rows for SparseHost<'a> {
        type Row = (&'a i64,);
        fn rows(self) -> impl Iterator<Item = Self::Row> {
            self.rows.iter().map(|row| (&row.0,))
        }
    }
    impl PositionalRows for SparseHost<'_> {
        fn len(self) -> usize {
            self.rows.len()
        }
        fn row_at(self, id: RowId) -> Option<Self::Row> {
            self.rows.get(id.offset()).map(|row| (&row.0,))
        }
        fn block_size(self) -> NonZeroUsize {
            self.size
        }
        fn positions(self) -> impl Iterator<Item = RowId> {
            self.rows.positions().inspect(move |_| {
                self.enumerated.set(self.enumerated.get() + 1);
            })
        }
        fn block_positions(self, block: BlockId) -> impl Iterator<Item = RowId> {
            self.rows
                .positions()
                .skip(block.offset() * self.size.get())
                .take(self.size.get())
                .inspect(move |_| self.enumerated.set(self.enumerated.get() + 1))
        }
    }
    #[derive(Clone, Copy)]
    struct SparseCol;
    impl Column<SparseHost<'_>> for SparseCol {
        type Key = i64;
        fn key(self, host: SparseHost<'_>, id: RowId) -> i64 {
            host.events.borrow_mut().push(('k', id.offset()));
            host.rows[id.offset()].0
        }
    }
    impl BlockRead<SparseCol> for SparseHost<'_> {
        fn bounds(self, _: SparseCol, block: BlockId) -> Option<(i64, i64)> {
            self.events.borrow_mut().push(('b', block.offset()));
            let start = block.offset() * self.size.get();
            let end = (start + self.size.get()).min(self.rows.len());
            let rows = &self.rows[start..end];
            Some((
                rows.iter().map(|row| row.0).min()?,
                rows.iter().map(|row| row.0).max()?,
            ))
        }
    }
    struct AtLeastThree<'a>(&'a RefCell<Vec<(char, usize)>>);
    impl ReadPredicate<i64> for AtLeastThree<'_> {
        fn test(&self, key: &i64) -> bool {
            self.0.borrow_mut().push(('t', *key as usize));
            *key >= 3
        }
        fn any_in(&self, _: &i64, hi: &i64) -> bool {
            self.0.borrow_mut().push(('a', *hi as usize));
            *hi >= 3
        }
    }

    for len in [0usize, 1, 65, 257, 4097] {
        // Equal keys remain distinct occurrences, including bitmap boundaries.
        let data: Vec<_> = (0..len).map(|i| ((i % 7) as i64,)).collect();
        for size in [1, 3, 63, 64, 65, 130, 4097] {
            for summary in [SelectionSummary::None, SelectionSummary::PerBlock] {
                for output_summary in [SelectionSummary::None, SelectionSummary::PerBlock] {
                    let enumerated = Cell::new(0);
                    let events = RefCell::new(Vec::new());
                    let host = SparseHost {
                        rows: &data,
                        size: NonZeroUsize::new(size).unwrap(),
                        enumerated: &enumerated,
                        events: &events,
                    };
                    for empty in [false, true] {
                        let candidates: Vec<_> = host
                            .positions()
                            .filter(|id| {
                                !empty
                                    && [0, 63, 64, 65, 129, 130, len.saturating_sub(1)]
                                        .contains(&id.offset())
                            })
                            .collect();
                        let selected =
                            Selection::from_positions(host, candidates.iter().copied(), summary);
                        enumerated.set(0);
                        events.borrow_mut().clear();
                        let narrowed =
                            selected.narrow(SparseCol, &AtLeastThree(&events), output_summary);
                        let mut expected_events = Vec::new();
                        for block in host.blocks() {
                            let start = block.offset() * size;
                            let end = (start + size).min(len);
                            let live: Vec<_> = candidates
                                .iter()
                                .filter(|id| (start..end).contains(&id.offset()))
                                .collect();
                            if live.is_empty() {
                                continue;
                            }
                            let hi = data[start..end].iter().map(|row| row.0).max().unwrap();
                            expected_events.push(('b', block.offset()));
                            expected_events.push(('a', hi as usize));
                            if hi >= 3 {
                                for id in live {
                                    expected_events.push(('k', id.offset()));
                                    expected_events.push(('t', data[id.offset()].0 as usize));
                                }
                            }
                        }
                        assert_eq!(*events.borrow(), expected_events);
                        let expected: Vec<_> = candidates
                            .iter()
                            .copied()
                            .filter(|id| data[id.offset()].0 >= 3)
                            .collect();
                        assert_eq!(narrowed.positions().collect::<Vec<_>>(), expected);
                        assert_eq!(selected.positions().collect::<Vec<_>>(), candidates);
                        assert_eq!(
                            narrowed.rows().map(|(key,)| *key).collect::<Vec<_>>(),
                            expected
                                .iter()
                                .map(|id| data[id.offset()].0)
                                .collect::<Vec<_>>()
                        );
                        assert_eq!(
                            enumerated.get(),
                            0,
                            "narrow and snapshot reads must not enumerate base positions"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn identity_reads_do_not_extract_keys_evaluate_predicates_or_read_bounds() {
    #[derive(Clone, Copy)]
    struct NeverRead;
    impl Column<Host<'_>> for NeverRead {
        type Key = i64;
        fn key(self, _: Host<'_>, _: RowId) -> i64 {
            panic!("identity reads must not extract keys")
        }
    }
    impl BlockRead<NeverRead> for Host<'_> {
        fn bounds(self, _: NeverRead, _: BlockId) -> Option<(i64, i64)> {
            panic!("identity reads must not fetch bounds")
        }
    }
    struct Identity;
    impl ReadPredicate<i64> for Identity {
        fn is_identity(&self) -> bool {
            true
        }
        fn test(&self, _: &i64) -> bool {
            panic!("identity reads must not evaluate predicates")
        }
        fn any_in(&self, _: &i64, _: &i64) -> bool {
            panic!("identity reads must not evaluate interval predicates")
        }
    }
    fn check<P: ReadPredicate<i64>>(host: Host<'_>, predicate: P) {
        let expected: Vec<_> = host.positions().collect();
        assert_eq!(
            host.read(NeverRead, &predicate).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            host.read_owned(NeverRead, &predicate)
                .fold(Vec::new(), |mut ids, id| {
                    ids.push(id);
                    ids
                }),
            expected
        );
        let mut partial = host.read_owned(NeverRead, &predicate);
        assert_eq!(partial.next(), expected.first().copied());
        assert_eq!(
            partial.fold(Vec::new(), |mut ids, id| {
                ids.push(id);
                ids
            }),
            expected.iter().skip(1).copied().collect::<Vec<_>>()
        );
        assert_eq!(
            host.read_chunked(NeverRead, &predicate).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            read_with_filter(host, NeverRead, &predicate, NeverRead, &Identity).collect::<Vec<_>>(),
            expected
        );
        for id in expected {
            assert!(test_position(host, NeverRead, &predicate, id));
        }
    }
    let source = [(0,), (1,), (1,), (3,), (4,)];
    let reads = Cell::new(0);
    for len in [0, 1, source.len()] {
        let host = Host {
            rows: &source[..len],
            reads: &reads,
            unknown: false,
        };
        check(host, Identity);
        check(host, NoJoinFilter);
        check(
            host,
            JoinFilterPredicate::<std::collections::HashSet<i64>>(None),
        );
        let expected: Vec<_> = host.read(Col, &Point(1)).collect();
        assert_eq!(
            read_with_filter(host, Col, Point(1), NeverRead, &Identity).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            read_with_filter(host, NeverRead, Identity, Col, &Point(1)).collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn combined_read_caches_identity_modes_and_filters_static_survivors_only() {
    use std::cell::RefCell;

    struct Predicate<'a> {
        identity: bool,
        identity_calls: &'a Cell<usize>,
        visits: &'a RefCell<Vec<i64>>,
        keep: fn(i64) -> bool,
    }
    impl ReadPredicate<i64> for Predicate<'_> {
        fn is_identity(&self) -> bool {
            self.identity_calls.set(self.identity_calls.get() + 1);
            self.identity
        }
        fn test(&self, key: &i64) -> bool {
            self.visits.borrow_mut().push(*key);
            self.identity || (self.keep)(*key)
        }
    }

    let source = [(0,), (1,), (2,), (2,), (3,), (4,)];
    for len in [0, 1, source.len()] {
        for predicate_identity in [false, true] {
            for filter_identity in [false, true] {
                let reads = Cell::new(0);
                let predicate_calls = Cell::new(0);
                let filter_calls = Cell::new(0);
                let predicate_visits = RefCell::new(Vec::new());
                let filter_visits = RefCell::new(Vec::new());
                let host = Host {
                    rows: &source[..len],
                    reads: &reads,
                    unknown: false,
                };
                let predicate = Predicate {
                    identity: predicate_identity,
                    identity_calls: &predicate_calls,
                    visits: &predicate_visits,
                    keep: |key| key >= 2,
                };
                let filter = Predicate {
                    identity: filter_identity,
                    identity_calls: &filter_calls,
                    visits: &filter_visits,
                    keep: |key| key % 2 == 0,
                };
                let actual = read_with_filter(host, Col, predicate, Col, &filter)
                    .map(RowId::offset)
                    .collect::<Vec<_>>();
                let expected = (0..len)
                    .filter(|&i| {
                        (predicate_identity || source[i].0 >= 2)
                            && (filter_identity || source[i].0 % 2 == 0)
                    })
                    .collect::<Vec<_>>();
                assert_eq!(actual, expected);
                assert_eq!(predicate_calls.get(), 1);
                assert_eq!(filter_calls.get(), 1);
                assert_eq!(
                    *predicate_visits.borrow(),
                    source[..len]
                        .iter()
                        .filter(|_| !predicate_identity)
                        .map(|row| row.0)
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    *filter_visits.borrow(),
                    source[..len]
                        .iter()
                        .filter(|row| !filter_identity && (predicate_identity || row.0 >= 2))
                        .map(|row| row.0)
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    reads.get(),
                    predicate_visits.borrow().len() + filter_visits.borrow().len()
                );
            }
        }
    }
}
