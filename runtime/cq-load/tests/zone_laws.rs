use cq_load::{MinMax, NoBounds, ZoneRows, ZoneSchema, column};
use cq_storage::rows::Rows;
use cq_storage::{
    AdaptiveKeySet, BlockRead, Column, I64Codec, Membership, NoDense, Point, PositionalRows,
    ReadPredicate, RowId, RowPredicate, Selection, SelectionSummary, Window, counters,
};
use std::num::NonZeroUsize;
use std::ops::Bound::{Excluded, Included, Unbounded};

#[derive(Clone, Copy)]
struct Scalar<H>(H);
impl<H: PositionalRows> Rows for Scalar<H> {
    type Row = H::Row;
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.0.rows()
    }
}
impl<H: PositionalRows> PositionalRows for Scalar<H> {
    fn len(self) -> usize {
        self.0.len()
    }
    fn row_at(self, id: RowId) -> Option<Self::Row> {
        self.0.row_at(id)
    }
}
#[derive(Clone, Copy)]
struct ScalarColumn<C>(C);
impl<H: PositionalRows, C: Column<H>> Column<Scalar<H>> for ScalarColumn<C> {
    type Key = C::Key;
    fn key(self, host: Scalar<H>, id: RowId) -> Self::Key {
        self.0.key(host.0, id)
    }
}
impl<H: PositionalRows, C: Column<H>> BlockRead<ScalarColumn<C>> for Scalar<H> {}

fn laws<H, C, P>(host: H, column: C, predicate: &P)
where
    H: BlockRead<C>,
    C: Column<H>,
    P: ReadPredicate<C::Key>,
{
    let scalar: Vec<_> = Scalar(host).read(ScalarColumn(column), predicate).collect();
    struct Borrowed<'a, P>(&'a P);
    impl<K, P: ReadPredicate<K>> ReadPredicate<K> for Borrowed<'_, P> {
        fn test(&self, key: &K) -> bool {
            self.0.test(key)
        }
        fn any_in(&self, lo: &K, hi: &K) -> bool {
            self.0.any_in(lo, hi)
        }
    }
    let owned: Vec<_> = host.read_owned(column, Borrowed(predicate)).collect();
    assert_eq!(owned, scalar);

    let mut expected_visited = 0;
    let mut expected_tested = 0;
    for block in host.blocks() {
        let bounds = host.bounds(column, block);
        let skip = bounds
            .as_ref()
            .is_some_and(|(lo, hi)| !predicate.any_in(lo, hi));
        for id in host.block_positions(block) {
            let key = column.key(host, id);
            if let Some((lo, hi)) = &bounds {
                assert!(lo <= &key && &key <= hi, "bounds must enclose every key");
            }
            if skip {
                assert!(!predicate.test(&key), "rejected block contains a match");
            } else {
                expected_tested += 1;
            }
        }
        if !skip {
            expected_visited += 1;
        }
    }
    counters::reset();
    let actual: Vec<_> = host.read(column, predicate).collect();
    let costs = counters::snapshot();
    assert_eq!(actual, scalar, "exact positions, multiplicity and order");
    assert_eq!(costs.blocks_considered, host.blocks().count() as u64);
    assert_eq!(costs.blocks_visited, expected_visited);
    assert_eq!(
        costs.blocks_visited + costs.blocks_skipped,
        costs.blocks_considered
    );
    assert_eq!(costs.blocks_skipped, costs.blocks_skipped_metadata);
    assert_eq!(costs.blocks_skipped_empty_selection, 0);
    assert_eq!(costs.rows_tested, expected_tested);
    assert_eq!(host.read(column, predicate).collect::<Vec<_>>(), actual);
    counters::reset();
    assert_eq!(
        host.read_owned(column, Borrowed(predicate))
            .collect::<Vec<_>>(),
        actual
    );
    assert_eq!(
        counters::snapshot(),
        costs,
        "owned and borrowed reads count the same work"
    );
}

#[test]
fn positional_read_laws_empty_singleton_duplicates_extremes_and_all_block_shapes() {
    let keys = [i64::MIN, -9, -9, 0, 1, 2, 63, 64, 65, 128, i64::MAX];
    let dense = AdaptiveKeySet::<_, I64Codec, 256>::from_keys([-9, 0, 64, 128], I64Codec);
    let hash = AdaptiveKeySet::<_, NoDense, 0>::from_keys([i64::MIN, 0, 65, i64::MAX], NoDense);
    for len in 0..=keys.len() {
        for size in [1, 2, 3, 4, 12, 2048, usize::MAX] {
            let rows: Vec<_> = keys[..len]
                .iter()
                .enumerate()
                .map(|(i, &k)| (k, i))
                .collect();
            let host =
                ZoneRows::from_rows::<(MinMax<i64>, MinMax<usize>)>(rows, size.try_into().unwrap());
            assert_eq!(host.block_rows().get(), size);
            assert_eq!((&host).block_size().get(), size);
            assert_eq!(
                host.rows().map(|(k, _)| *k).collect::<Vec<_>>(),
                keys[..len]
            );
            assert_eq!(
                host.positions().map(|id| id.offset()).collect::<Vec<_>>(),
                (0..len).collect::<Vec<_>>()
            );
            laws(&host, column::<0>(), &Membership(&dense));
            laws(&host, column::<0>(), &Membership(&hash));
            laws(&host, column::<0>(), &RowPredicate(|k: &i64| k % 2 == 0));
            for key in keys {
                laws(&host, column::<0>(), &Point(key));
            }
            for (low, high) in [
                (Unbounded, Unbounded),
                (Included(-9), Excluded(65)),
                (Excluded(0), Included(0)),
                (Included(128), Included(-9)),
            ] {
                laws(&host, column::<0>(), &Window { low, high });
            }
            // Nonleading columns have independent bounds, not lexicographic tuples.
            laws(&host, column::<1>(), &Point(2));
        }
    }
}

#[test]
fn all_integer_types_and_exact_scalars() {
    macro_rules! check {
        ($($ty:ty),*) => {$ (
            let host = ZoneRows::from_rows::<(MinMax<$ty>,)>(
                vec![(<$ty>::MAX,), (<$ty>::MIN,), (<$ty>::MAX,)], 2.try_into().unwrap());
            laws(&host, column::<0>(), &Point(<$ty>::MAX));
        )* };
    }
    check!(
        i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
    );
    use cq_load::{Date, Decimal};
    let date = |s: &str| s.parse::<Date>().unwrap();
    let price = |s: &str| s.parse::<Decimal<2>>().unwrap();
    let host = ZoneRows::from_rows::<(MinMax<Date>, MinMax<Decimal<2>>, MinMax<bool>)>(
        vec![
            (date("1969-12-31"), price("-0.01"), true),
            (date("2000-02-29"), price("92233720368547758.07"), false),
            (date("0000-01-01"), price("-92233720368547758.08"), true),
        ],
        2.try_into().unwrap(),
    );
    laws(
        &host,
        column::<0>(),
        &Window {
            low: Included(date("1970-01-01")),
            high: Unbounded,
        },
    );
    laws(&host, column::<1>(), &Point(price("-0.01")));
    laws(&host, column::<2>(), &Point(true));
}

#[test]
fn strings_use_exact_rust_order_and_owned_extrema() {
    let input = ["é", "e\u{301}", "a", "A", "", "z", "a\0", "中", "a"];
    let rows: Vec<_> = input.iter().map(|&s| (s.to_string(), s)).collect();
    let host = ZoneRows::from_rows::<(MinMax<String>, MinMax<String>)>(rows, 2.try_into().unwrap());
    for key in input {
        laws(&host, column::<0>(), &Point(key));
        laws(&host, column::<1>(), &Point(key));
    }
    laws(
        &host,
        column::<1>(),
        &Window {
            low: Excluded("a"),
            high: Included("é"),
        },
    );
    let block = host.blocks().next().unwrap();
    let (lo, hi) = host.bounds(column::<1>(), block).unwrap();
    assert_eq!((lo, hi), ("e\u{301}", "é"));
    // Borrowed input text is copied into metadata ownership, not retained as a
    // pointer to the source's bytes. Bounds themselves borrow that ownership.
    assert_ne!(hi.as_ptr(), input[0].as_ptr());
    assert!(
        host.metadata_bytes()
            > host.blocks().count() * std::mem::size_of::<(MinMax<String>, MinMax<String>)>()
    );
}

#[test]
fn unknown_stats_nulls_unordered_and_nonclone_payloads() {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Opaque(i32);
    type Schema = (
        MinMax<i64>,
        NoBounds<Opaque>,
        NoBounds<Option<i64>>,
        NoBounds<f64>,
    );
    let host = ZoneRows::from_rows::<Schema>(
        vec![
            (1, Opaque(9), None, f64::NAN),
            (3, Opaque(7), Some(3), -0.0),
            (5, Opaque(9), Some(9), f64::INFINITY),
        ],
        2.try_into().unwrap(),
    );
    for block in host.blocks() {
        assert!(host.bounds(column::<1>(), block).is_none());
        assert!(host.bounds(column::<2>(), block).is_none());
    }
    laws(&host, column::<0>(), &Point(5));
    laws(&host, column::<1>(), &Point(&Opaque(9)));
    laws(&host, column::<2>(), &Point(&None));
    assert_eq!(host.rows().filter(|r| r.3.is_nan()).count(), 1);
    // Even a predicate whose interval summary rejects everything must test
    // every unknown block. The empty represented set is a lawful example.
    laws(
        &host,
        column::<1>(),
        &Window {
            low: Excluded(&Opaque(1)),
            high: Included(&Opaque(1)),
        },
    );
}

#[test]
fn batches_preserve_global_alignment_empty_batches_and_short_tail() {
    type Schema = (MinMax<i64>, MinMax<String>);
    let input: Vec<_> = (0..23).map(|i| (i, format!("{i:02}"))).collect();
    for size in [1, 2, 3, 7, 23, 100] {
        for batch_size in [1, 2, 3, 8, 23, 40] {
            let batches = std::iter::once(Ok::<_, ()>(vec![])).chain(
                input
                    .chunks(batch_size)
                    .flat_map(|batch| [Ok(batch.to_vec()), Ok(vec![])]),
            );
            let host =
                ZoneRows::from_batches::<Schema, _, _>(batches, size.try_into().unwrap()).unwrap();
            let expected = ZoneRows::from_rows::<Schema>(input.clone(), size.try_into().unwrap());
            assert_eq!(
                host.rows().collect::<Vec<_>>(),
                expected.rows().collect::<Vec<_>>()
            );
            assert_eq!(host.blocks().count(), input.len().div_ceil(size));
            for block in host.blocks() {
                assert_eq!(
                    host.bounds(column::<0>(), block),
                    expected.bounds(column::<0>(), block)
                );
                assert_eq!(
                    host.bounds(column::<1>(), block),
                    expected.bounds(column::<1>(), block)
                );
                assert_eq!(
                    host.block_positions(block).count(),
                    size.min(input.len() - block.offset() * size)
                );
            }
            laws(&host, column::<0>(), &Point(7));
        }
    }
    let empty = ZoneRows::from_batches::<Schema, (), _>(
        [Ok(Vec::<(i64, String)>::new()), Ok(vec![])],
        7.try_into().unwrap(),
    )
    .unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.metadata_bytes(), 0);
    laws(&empty, column::<0>(), &Point(7));
}

#[test]
fn source_is_consumed_once_and_stops_on_error() {
    use std::cell::Cell;
    let calls = Cell::new(0);
    let batches = (0..10).map(|i| {
        calls.set(calls.get() + 1);
        if i == 3 {
            Err("decode failure")
        } else {
            Ok(vec![(i,)])
        }
    });
    let result = ZoneRows::from_batches::<(MinMax<i64>,), _, _>(batches, 2.try_into().unwrap());
    assert!(matches!(result, Err("decode failure")));
    assert_eq!(calls.get(), 4);
}

#[test]
fn summarizes_every_occurrence_exactly_once_and_reuses_the_input_allocation() {
    use std::cell::Cell;
    thread_local! { static SEEN: Cell<usize> = const { Cell::new(0) }; }
    struct Counting;
    impl ZoneSchema<(i64,)> for Counting {
        type Metadata = (MinMax<i64>,);
        fn summarize(rows: &[(i64,)]) -> Self::Metadata {
            SEEN.with(|n| n.set(n.get() + rows.len()));
            <Self::Metadata as ZoneSchema<(i64,)>>::summarize(rows)
        }
    }
    let mut rows = Vec::with_capacity(100);
    rows.extend((0..11).map(|i| (i,)));
    let first = &rows[0].0 as *const i64;
    let host = ZoneRows::from_rows::<Counting>(rows, 3.try_into().unwrap());
    assert_eq!(host.rows().next().unwrap().0 as *const i64, first);
    SEEN.with(|n| {
        assert_eq!(n.get(), 11);
        n.set(0);
    });
    let batches = (0..11).map(|i| Ok::<_, ()>(vec![(i,)]));
    let host = ZoneRows::from_batches::<Counting, _, _>(batches, 3.try_into().unwrap()).unwrap();
    SEEN.with(|n| assert_eq!(n.get(), 11));
    laws(&host, column::<0>(), &Point(9));
}

#[test]
fn selection_composes_with_host_bounds_and_preserves_old_snapshots() {
    let host = ZoneRows::from_rows::<(MinMax<i64>, MinMax<i64>)>(
        vec![(1, 8), (1, 8), (7, 2), (8, 3), (9, 8)],
        2.try_into().unwrap(),
    );
    let all = Selection::select(
        &host,
        column::<0>(),
        &RowPredicate(|_: &i64| true),
        SelectionSummary::PerBlock,
    );
    let selected = all.narrow(column::<1>(), &Point(8), SelectionSummary::PerBlock);
    assert_eq!(
        selected.rows().map(|(a, b)| (*a, *b)).collect::<Vec<_>>(),
        [(1, 8), (1, 8), (9, 8)]
    );
    assert_eq!(all.rows().count(), 5);
}

#[test]
fn zero_and_maximum_supported_tuple_arity() {
    let empty_heading = ZoneRows::from_rows::<()>(vec![(), (), ()], NonZeroUsize::MIN);
    assert_eq!(empty_heading.rows().count(), 3);
    type Schema = (
        MinMax<i64>,
        MinMax<i64>,
        MinMax<i64>,
        MinMax<i64>,
        MinMax<i64>,
        MinMax<i64>,
        MinMax<i64>,
        MinMax<i64>,
        MinMax<i64>,
        MinMax<i64>,
        MinMax<i64>,
        MinMax<i64>,
    );
    let host = ZoneRows::from_rows::<Schema>(
        vec![(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11)],
        NonZeroUsize::MIN,
    );
    laws(&host, column::<11>(), &Point(11));
}

#[test]
fn unavailable_bounds_for_a_supported_column_retain_that_block() {
    struct PartlyUnknown;
    impl ZoneSchema<(i64,)> for PartlyUnknown {
        type Metadata = (MinMax<i64>,);
        fn summarize(rows: &[(i64,)]) -> Self::Metadata {
            if rows[0].0 == 7 {
                Self::Metadata::default()
            } else {
                <Self::Metadata as ZoneSchema<(i64,)>>::summarize(rows)
            }
        }
    }
    let host = ZoneRows::from_rows::<PartlyUnknown>(
        vec![(1,), (2,), (7,), (8,), (9,)],
        2.try_into().unwrap(),
    );
    laws(&host, column::<0>(), &Point(7));
    counters::reset();
    assert_eq!(
        host.read(column::<0>(), &Point(7))
            .map(|id| id.offset())
            .collect::<Vec<_>>(),
        [2]
    );
    let costs = counters::snapshot();
    assert_eq!(
        (
            costs.blocks_considered,
            costs.blocks_visited,
            costs.blocks_skipped
        ),
        (3, 1, 2)
    );
    assert_eq!(costs.rows_tested, 2);
}

#[test]
fn rejected_blocks_never_fetch_keys_and_partial_reads_count_only_demanded_blocks() {
    use cq_load::{ZoneField, ZoneMetadata};
    use std::cell::Cell;
    thread_local! { static KEYS: Cell<usize> = const { Cell::new(0) }; }
    #[derive(Default)]
    struct Counted(MinMax<i64>);
    impl ZoneMetadata for Counted {
        fn heap_bytes(&self) -> usize {
            0
        }
    }
    impl ZoneField<i64> for Counted {
        type Key<'a> = i64;
        fn observe(&mut self, key: &i64) {
            self.0.observe(key);
        }
        fn key(value: &i64) -> i64 {
            KEYS.with(|n| n.set(n.get() + 1));
            *value
        }
        fn bounds(&self) -> Option<(i64, i64)> {
            self.0.bounds()
        }
    }
    let host = ZoneRows::from_rows::<(Counted,)>(
        vec![(1,), (2,), (7,), (7,), (99,)],
        2.try_into().unwrap(),
    );
    counters::reset();
    let predicate = Point(7);
    let mut read = host.read(column::<0>(), &predicate);
    assert_eq!(counters::snapshot().blocks_considered, 0);
    assert_eq!(read.next().unwrap().offset(), 2);
    assert_eq!(counters::snapshot().blocks_considered, 2);
    KEYS.with(|n| assert_eq!(n.get(), 1));
    assert_eq!(read.map(|id| id.offset()).collect::<Vec<_>>(), [3]);
    KEYS.with(|n| assert_eq!(n.get(), 2));
    let costs = counters::snapshot();
    assert_eq!((costs.blocks_visited, costs.blocks_skipped), (1, 2));
}

#[test]
fn metadata_bytes_include_capacity_and_owned_text_but_not_rows() {
    let host = ZoneRows::from_rows::<(MinMax<i64>,)>(vec![(1,); 5], 2.try_into().unwrap());
    assert_eq!(
        host.metadata_bytes(),
        3 * std::mem::size_of::<MinMax<i64>>()
    );
    let host =
        ZoneRows::from_rows::<(MinMax<String>,)>(vec![("aa",), ("zzz",)], 1.try_into().unwrap());
    assert_eq!(
        host.metadata_bytes(),
        2 * std::mem::size_of::<MinMax<String>>() + 2 * (2 + 3)
    );
    let host = ZoneRows::from_rows::<(NoBounds<String>,)>(
        vec![("payload".repeat(100),)],
        1.try_into().unwrap(),
    );
    assert_eq!(host.metadata_bytes(), 0);
}
