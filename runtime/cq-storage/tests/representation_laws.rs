//! Each representation is admissible only because an abstraction function `α`
//! commutes with its operations. These tests state `α` through the public
//! traits and check the squares exhaustively over small universes.

use cq_storage::{Build, HashIndex, IndexRead, SortedRun};
use std::collections::BTreeMap;

type Bag<T> = BTreeMap<T, usize>;

fn bag<T: Ord>(items: impl IntoIterator<Item = T>) -> Bag<T> {
    let mut bag = Bag::new();
    for item in items {
        *bag.entry(item).or_default() += 1;
    }
    bag
}

/// Every subset of `universe`, as the bit patterns of a small universe.
fn subsets<T: Copy>(universe: &[T]) -> impl Iterator<Item = Vec<T>> + '_ {
    (0..1u32 << universe.len()).map(move |mask| {
        universe
            .iter()
            .enumerate()
            .filter_map(|(i, &x)| (mask & (1 << i) != 0).then_some(x))
            .collect()
    })
}

fn index_laws<T>(build: impl Fn(Vec<(i32, char)>) -> T)
where
    for<'a> &'a T: IndexRead<Key = i32, Item = char>,
{
    let universe = [(0, 'a'), (0, 'b'), (1, 'a'), (1, 'c'), (2, 'a')];
    for pairs in subsets(&universe) {
        // Multiplicity: duplicate every second pair.
        let source: Vec<_> = pairs
            .iter()
            .enumerate()
            .flat_map(|(i, &p)| std::iter::repeat_n(p, 1 + i % 2))
            .collect();
        let index = build(source.clone());
        let mut alpha = Bag::new();
        for key in -1..=3 {
            let items: Vec<_> = (&index).lookup(&key).collect();
            let expected: Vec<_> = source
                .iter()
                .filter(|(k, _)| *k == key)
                .map(|&(_, v)| v)
                .collect();
            assert_eq!(
                items, expected,
                "lookup({key}) must be σ_key in source order"
            );
            for item in items {
                *alpha.entry((key, item)).or_default() += 1;
            }
        }
        assert_eq!(alpha, bag(source.iter().copied()));
    }
}

#[test]
fn index_abstraction_is_the_source_bag_and_lookup_is_exact_selection_in_order() {
    index_laws(|source| HashIndex::build(source.into_iter()));
    index_laws(|source| SortedRun::<_, _>::build(source.into_iter()));
    index_laws(|source| SortedRun::<_, _, cq_storage::I64Codec>::build(source.into_iter()));
}

#[derive(Clone, Copy)]
struct First;
impl<H: cq_storage::PositionalRows> cq_storage::Column<H> for First
where
    H::Row: FirstKey,
{
    type Key = i64;
    fn key(self, host: H, id: cq_storage::RowId) -> i64 {
        host.row_at(id).unwrap().first()
    }
}
trait FirstKey {
    fn first(self) -> i64;
}
impl FirstKey for (&i64, &char) {
    fn first(self) -> i64 {
        *self.0
    }
}

#[test]
fn positions_and_selection_preserve_occurrences_snapshots_and_projected_builds() {
    use cq_storage::rows::Rows;
    use cq_storage::{
        AdaptiveKeySet, ExactMembership, I64Codec, PositionalRows, RowPredicate, Selection,
        SelectionSummary,
    };
    let universe = [(0i64, 'a'), (0, 'b'), (1, 'a'), (2, 'c')];
    for pairs in subsets(&universe) {
        let source: Vec<_> = pairs.iter().flat_map(|&p| [p, p]).collect();
        let host = &source;
        assert_eq!(
            host.positions()
                .map(|id| host.row_at(id).unwrap())
                .collect::<Vec<_>>(),
            host.rows().collect::<Vec<_>>()
        );
        for summary in [SelectionSummary::None, SelectionSummary::PerBlock] {
            let selected =
                Selection::select(host, First, &RowPredicate(|k: &i64| *k != 1), summary);
            let original = selected.rows().collect::<Vec<_>>();
            assert_eq!(
                original,
                host.rows().filter(|&(k, _)| *k != 1).collect::<Vec<_>>()
            );
            let narrower = selected.narrow(First, &RowPredicate(|k: &i64| *k == 0), summary);
            assert_eq!(
                narrower.rows().collect::<Vec<_>>(),
                original
                    .iter()
                    .copied()
                    .filter(|&(k, _)| *k == 0)
                    .collect::<Vec<_>>()
            );
            assert_eq!(selected.rows().collect::<Vec<_>>(), original);
            for id in host.positions() {
                assert_eq!(
                    narrower.contains(id),
                    selected.contains(id) && *host.row_at(id).unwrap().0 == 0
                );
            }
            let probes = [0i64, 1, 2];
            let a: HashIndex<_, _> = selected.build(|(k, v)| (k, v));
            let b: HashIndex<_, _> = selected.build(|(k, v)| (k, v));
            for key in &probes {
                let expected: Vec<_> = original
                    .iter()
                    .filter(|(k, _)| **k == *key)
                    .map(|(_, v)| *v)
                    .collect();
                assert_eq!(a.lookup(&key).collect::<Vec<_>>(), expected);
                assert_eq!(b.lookup(&key).collect::<Vec<_>>(), expected);
            }
            let keys: AdaptiveKeySet<i64, I64Codec, 64> = selected.build(|(k, _)| (*k, ()));
            for key in 0..=2 {
                assert_eq!(
                    keys.contains(&key),
                    original.iter().any(|(k, _)| **k == key)
                );
            }
        }
    }
    let array = [(1i64, 'a'), (1, 'a')];
    for id in (&array).positions() {
        assert_eq!((&array).row_at(id), (&array[..]).row_at(id));
    }
    let large: Vec<_> = (0..130).map(|i| (i, 'x')).collect();
    let selected = Selection::select(
        &large,
        First,
        &RowPredicate(|k: &i64| [0, 63, 64, 65, 127, 128, 129].contains(k)),
        SelectionSummary::PerBlock,
    );
    assert_eq!(
        selected.rows().map(|(k, _)| *k).collect::<Vec<_>>(),
        [0, 63, 64, 65, 127, 128, 129]
    );
    assert_eq!(selected.retained_bytes(), 32);
}

#[test]
fn fetch_is_the_named_column_of_every_live_occurrence() {
    use cq_load::ColumnAt;
    use cq_storage::{Column, Fetch, PositionalRows};

    fn check<'a, H>(host: H)
    where
        H: PositionalRows<Row = (&'a i64, &'a String)> + Fetch<ColumnAt<0>> + Fetch<ColumnAt<1>>,
        ColumnAt<0>: Column<H, Key = i64>,
        ColumnAt<1>: Column<H, Key = &'a str>,
    {
        for id in host.positions() {
            let row = host.row_at(id).unwrap();
            assert_eq!(host.fetch(ColumnAt::<0>, id), *row.0);
            assert_eq!(host.fetch(ColumnAt::<1>, id), row.1.as_str());
            assert!(std::ptr::eq(host.fetch(ColumnAt::<1>, id), row.1.as_str()));
        }
    }

    let source = [
        (7i64, String::from("same")),
        (-2, String::from("other")),
        (7, String::from("same")),
    ];
    check(&source);
    for len in 0..=source.len() {
        check(&source[..len]);
        check(&source[..len].to_vec());
    }
    let empty: [(i64, String); 0] = [];
    check(&empty);
}

/// The Filter square, exhaustively: for every host bag and every key set, a
/// semijoin filter keeps exactly the matching occurrences in source order on
/// every read (rows, positions, contains, compact build); filtering twice is
/// filtering by the intersection.
#[test]
fn filter_by_membership_commutes_with_alpha_on_every_read() {
    use cq_storage::rows::Rows;
    use cq_storage::{AdaptiveKeySet, Filter, I64Codec, Membership, PositionalRows};
    let universe = [(0i64, 'a'), (0, 'b'), (1, 'a'), (2, 'c')];
    let keys = [0i64, 1, 2, 3];
    for pairs in subsets(&universe) {
        let source: Vec<_> = pairs.iter().flat_map(|&p| [p, p]).collect();
        let host = &source;
        let alive = |first: &[i64], id| first.contains(host.row_at(id).unwrap().0);
        for first in subsets(&keys) {
            let m1: AdaptiveKeySet<i64, I64Codec, 64> =
                AdaptiveKeySet::build(first.iter().map(|&k| (k, ())));
            let filtered = host.filter(First, &Membership(&m1));
            let expected: Vec<_> = host.rows().filter(|(k, _)| first.contains(k)).collect();
            assert_eq!(
                filtered.rows().collect::<Vec<_>>(),
                expected,
                "rows of filter"
            );
            for id in host.positions() {
                assert_eq!(filtered.contains(id), alive(&first, id));
            }
            assert_eq!(
                filtered.positions().collect::<Vec<_>>(),
                host.positions()
                    .filter(|&id| alive(&first, id))
                    .collect::<Vec<_>>()
            );
            let compact: HashIndex<_, _> = filtered.build(|(k, v)| (*k, v));
            for key in -1..=3i64 {
                let want: Vec<_> = expected
                    .iter()
                    .filter(|(k, _)| **k == key)
                    .map(|(_, v)| *v)
                    .collect();
                assert_eq!(compact.lookup(&key).collect::<Vec<_>>(), want);
            }
            for second in subsets(&keys) {
                let m2: AdaptiveKeySet<i64, I64Codec, 64> =
                    AdaptiveKeySet::build(second.iter().map(|&k| (k, ())));
                let both: Vec<_> = first
                    .iter()
                    .filter(|k| second.contains(k))
                    .copied()
                    .collect();
                let mb: AdaptiveKeySet<i64, I64Codec, 64> =
                    AdaptiveKeySet::build(both.iter().map(|&k| (k, ())));
                assert_eq!(
                    (&filtered)
                        .filter(First, &Membership(&m2))
                        .rows()
                        .collect::<Vec<_>>(),
                    host.filter(First, &Membership(&mb))
                        .rows()
                        .collect::<Vec<_>>(),
                    "filtering twice is filtering by the intersection"
                );
            }
        }
    }
}

#[test]
fn adaptive_membership_dispatch_retains_only_the_dense_bitmap() {
    use cq_storage::{
        AdaptiveMembership, ConservativeMembership, ExactMembership, JoinFilterPredicate,
        NoJoinFilter, ReadPredicate,
    };

    let dense = cq_storage::build!(AdaptiveMembership<_, 16777216>, (0i64..4096).map(|k| (k, ())));
    let hash = AdaptiveMembership::<_, 16777216>::build((0i64..4096).map(|k| (k, ())));
    assert!(dense.is_dense());
    assert!(!hash.is_dense());
    assert_eq!(dense.retained_bytes(), 512);
    assert!(dense.retained_bytes() < hash.retained_bytes());
    for key in -1..=4096 {
        assert_eq!(dense.contains(&key), hash.contains(&key));
        assert_eq!(dense.test(&key), (0..4096).contains(&key));
    }
    let large_budget =
        cq_storage::build!(AdaptiveMembership<_, 67108864>, (0i64..4096).map(|k| (k, ())));
    assert!(large_budget.is_dense());
    assert_eq!(large_budget.retained_bytes(), 512);

    // A key-independent identity predicate does not pin the later filter to
    // the factory's generic fallback type.
    let filter = NoJoinFilter;
    assert!(filter.test(&4096));
    let filter = JoinFilterPredicate(Some(&dense));
    assert!(filter.test(&63));
    assert!(!filter.test(&4096));
    let filter = JoinFilterPredicate(Some(&hash));
    assert!(filter.test(&63));
    assert!(!filter.test(&4096));
}

#[test]
fn adaptive_membership_accepts_non_ord_non_clone_hash_keys() {
    use cq_storage::{AdaptiveMembership, ExactMembership};

    #[derive(Debug, PartialEq, Eq, Hash)]
    struct Key(u8);

    let set = cq_storage::build!(
        AdaptiveMembership<_, 16777216>,
        [Key(1), Key(3), Key(1)].into_iter().map(|key| (key, "ignored")),
    );
    assert!(!set.is_dense());
    for key in 0..=4 {
        assert_eq!(set.contains(&Key(key)), key == 1 || key == 3);
    }
}

#[test]
fn adaptive_membership_dense_and_hash_commute_with_alpha() {
    use cq_storage::{AdaptiveMembership, ExactMembership};

    let universe = [-65i64, -64, -1, 0, 63, 64, 65];
    for keys in subsets(&universe) {
        let dense = cq_storage::build!(
            AdaptiveMembership<_, 131>,
            keys.iter().flat_map(|&key| [(key, ()), (key, ())]),
        );
        let hash = AdaptiveMembership::<_, 131>::build(
            keys.iter().flat_map(|&key| [(key, ()), (key, ())]),
        );
        assert_eq!(dense.is_dense(), !keys.is_empty());
        for key in -66..=66 {
            assert_eq!(dense.contains(&key), keys.contains(&key));
            assert_eq!(hash.contains(&key), keys.contains(&key));
        }
    }
}

#[test]
fn adaptive_membership_respects_span_budget_and_integer_boundaries() {
    use cq_storage::{AdaptiveMembership, ExactMembership};

    let exact = cq_storage::build!(
        AdaptiveMembership<_, 64>,
        [-32i64, 31, -32].into_iter().map(|key| (key, ())),
    );
    assert!(exact.is_dense());
    assert_eq!(exact.retained_bytes(), 8);
    assert!(exact.contains(&-32));
    assert!(exact.contains(&31));
    assert!(!exact.contains(&0));
    assert!(!exact.contains(&32));

    let over = cq_storage::build!(
        AdaptiveMembership<_, 64>,
        [-32i64, 32].into_iter().map(|key| (key, ())),
    );
    assert!(!over.is_dense());
    assert!(over.contains(&-32));
    assert!(over.contains(&32));
    assert!(!over.contains(&0));

    let sparse = cq_storage::build!(
        AdaptiveMembership<_, 67108864>,
        [i64::MIN, i64::MAX, i64::MIN].into_iter().map(|key| (key, ())),
    );
    assert!(!sparse.is_dense());
    assert!(sparse.contains(&i64::MIN));
    assert!(sparse.contains(&i64::MAX));
    assert!(!sparse.contains(&0));
    for keys in [[i64::MIN, i64::MIN + 63], [i64::MAX - 63, i64::MAX]] {
        let set = cq_storage::build!(
            AdaptiveMembership<_, 64>,
            keys.into_iter().map(|key| (key, ())),
        );
        assert!(set.is_dense());
        assert_eq!(set.retained_bytes(), 8);
        assert!(set.contains(&keys[0]));
        assert!(set.contains(&keys[1]));
        assert!(!set.contains(&(keys[0] + 1)));
        assert!(!set.contains(&0));
    }

    let empty = cq_storage::build!(
        AdaptiveMembership<i64, 64>,
        std::iter::empty::<(i64, ())>(),
    );
    assert!(!empty.is_dense());
    assert_eq!(empty.retained_bytes(), 0);
    assert!(!empty.contains(&0));
    let disabled = cq_storage::build!(AdaptiveMembership<_, 0>, [(0i64, ())].into_iter());
    assert!(!disabled.is_dense());
    assert!(disabled.contains(&0));
    assert!(!disabled.contains(&1));
}

#[test]
fn adaptive_membership_supports_integer_and_borrowed_singleton_keys() {
    use cq_storage::{AdaptiveMembership, DenseOrdinal, ExactMembership};

    fn native_word<K: DenseOrdinal<Ordinal = u64>>() {}
    native_word::<i64>();
    native_word::<i32>();
    native_word::<u32>();
    native_word::<&i64>();
    native_word::<(&i32,)>();
    assert_eq!(i64::MIN.dense_ordinal(), 0u64);
    assert_eq!(i64::MAX.dense_ordinal(), u64::MAX);

    fn check<K: DenseOrdinal + std::hash::Hash>(keys: [K; 2], absent: K) {
        let scalar = cq_storage::build!(
            AdaptiveMembership<_, 64>,
            keys.iter().map(|key| (key, ())),
        );
        let tuple = cq_storage::build!(
            AdaptiveMembership<_, 64>,
            keys.iter().map(|key| ((key,), ())),
        );
        assert!(scalar.is_dense());
        assert!(tuple.is_dense());
        assert_eq!(scalar.retained_bytes(), 8);
        assert_eq!(tuple.retained_bytes(), 8);
        for key in &keys {
            assert!(scalar.contains(&key));
            assert!(tuple.contains(&(key,)));
        }
        assert!(!scalar.contains(&&absent));
        assert!(!tuple.contains(&(&absent,)));
    }
    check([-32i64, 31], 0);
    check([i32::MIN, i32::MIN + 63], i32::MAX);
    check([u32::MAX - 63, u32::MAX], 0);
    let i32_set = cq_storage::build!(
        AdaptiveMembership<_, 64>,
        [(i32::MIN, ()), (i32::MIN + 63, ())].into_iter(),
    );
    let u32_set = cq_storage::build!(
        AdaptiveMembership<_, 64>,
        [(u32::MAX - 63, ()), (u32::MAX, ())].into_iter(),
    );
    assert!(i32_set.is_dense());
    assert!(u32_set.is_dense());
    assert!(!i32_set.contains(&i32::MAX));
    assert!(!u32_set.contains(&0));
}

#[test]
fn adaptive_membership_custom_ordinal_is_exact_without_ordering() {
    use cq_storage::{AdaptiveMembership, DenseOrdinal, ExactMembership};

    #[derive(PartialEq, Eq, Hash)]
    struct Key(u128);
    impl DenseOrdinal for Key {
        type Ordinal = u128;

        fn dense_ordinal(&self) -> Self::Ordinal {
            self.0 ^ 63
        }
    }
    let dense = cq_storage::build!(
        AdaptiveMembership<_, 64>,
        [Key(0), Key(63), Key(0)].into_iter().map(|key| (key, ())),
    );
    assert!(dense.is_dense());
    assert!(dense.contains(&Key(0)));
    assert!(dense.contains(&Key(63)));
    assert!(!dense.contains(&Key(1)));
    assert!(!dense.contains(&Key(u128::MAX)));

    // A full-width ordinal span cannot wrap into a deceptively small bitmap.
    let overflow = cq_storage::build!(
        AdaptiveMembership<_, 64>,
        [Key(63), Key(u128::MAX ^ 63)].into_iter().map(|key| (key, ())),
    );
    assert!(!overflow.is_dense());
    assert!(overflow.contains(&Key(63)));
    assert!(overflow.contains(&Key(u128::MAX ^ 63)));
    assert!(!overflow.contains(&Key(0)));
    // Dense windows above u64::MAX retain their high bits, including absent
    // keys with the same low word and windows crossing the u64 boundary.
    for (low, high) in [
        (1u128 << 80, (1u128 << 80) + 63),
        (u128::from(u64::MAX) - 31, u128::from(u64::MAX) + 32),
        (u128::MAX - 63, u128::MAX),
    ] {
        let keys = [Key(low ^ 63), Key(high ^ 63)];
        let set = cq_storage::build!(
            AdaptiveMembership<_, 64>,
            keys.into_iter().map(|key| (key, ())),
        );
        assert!(set.is_dense());
        assert_eq!(set.retained_bytes(), 8);
        assert!(set.contains(&Key(low ^ 63)));
        assert!(set.contains(&Key(high ^ 63)));
        assert!(!set.contains(&Key((low + 1) ^ 63)));
        assert!(!set.contains(&Key((high as u64 as u128) ^ 63)));
    }
}

#[test]
fn build_dispatch_preserves_arbitrary_custom_build_and_generic_fallback() {
    use cq_storage::{AdaptiveMembership, ExactMembership};
    use std::cell::Cell;

    struct Custom(Vec<i32>);
    impl<I: Iterator<Item = i32>> Build<I> for Custom {
        fn build(source: I) -> Self {
            Self(source.collect())
        }
    }
    fn generic<S: Build<I>, I: Iterator>(source: I) -> S {
        cq_storage::build!(S, source)
    }
    let evaluations = Cell::new(0);
    let custom = cq_storage::build!(Custom, {
        evaluations.set(evaluations.get() + 1);
        [3, 1, 3].into_iter()
    });
    assert_eq!(evaluations.get(), 1);
    assert_eq!(custom.0, [3, 1, 3]);
    assert_eq!(generic::<Custom, _>([2, 4].into_iter()).0, [2, 4]);
    let hash: AdaptiveMembership<i64, 64> = generic([(1, ()), (2, ())].into_iter());
    assert!(!hash.is_dense());
    assert!(hash.contains(&1));
    assert!(hash.contains(&2));
    assert!(!hash.contains(&3));
}

#[test]
fn dense_codec_build_dispatch_resolves_aliases_and_preserves_exact_reads() {
    use cq_storage::{
        AdaptiveKeySet, AutoDense, BorrowedI64TupleCodec, ExactIntervalMembership, ExactMembership,
        I64Codec,
    };

    macro_rules! check {
        ($key:ty) => {{
            type Key = $key;
            let keys: [Key; 3] = [1, 3, 1];
            let run = cq_storage::build!(
                SortedRun<_, _, AutoDense>,
                keys.iter().enumerate().map(|(i, key)| ((key,), i)),
            );
            let run: SortedRun<(&Key,), usize, BorrowedI64TupleCodec> = run;
            assert_eq!((&run).lookup(&(&1,)).collect::<Vec<_>>(), [0, 2]);
            assert_eq!((&run).lookup(&(&3,)).collect::<Vec<_>>(), [1]);
            assert!((&run).lookup(&(&2,)).next().is_none());

            let set = cq_storage::build!(
                AdaptiveKeySet<_, AutoDense, 64>,
                keys.into_iter().map(|key| (key, ())),
            );
            let set: AdaptiveKeySet<Key, I64Codec, 64> = set;
            for key in 0..=4 {
                assert_eq!(set.contains(&key), key == 1 || key == 3);
            }
            assert!(!set.any_in(&2, &2));
            assert!(set.any_in(&2, &3));
        }};
    }
    check!(i64);
    check!(i32);
    check!(u32);
}

#[test]
fn dense_codec_dispatch_preserves_unsupported_keys_and_explicit_codecs() {
    use cq_storage::{AdaptiveKeySet, AutoDense, ExactMembership, I64Codec, NoDense};

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct Key(u8);
    let keys = [Key(1), Key(3)];
    let run = cq_storage::build!(
        SortedRun<_, _, AutoDense>,
        keys.iter().map(|key| ((key,), key.0)),
    );
    let run: SortedRun<(&Key,), u8, NoDense> = run;
    assert_eq!((&run).lookup(&(&Key(1),)).collect::<Vec<_>>(), [1]);
    let set = cq_storage::build!(
        AdaptiveKeySet<_, AutoDense, 64>,
        keys.into_iter().map(|key| (key, ())),
    );
    let set: AdaptiveKeySet<Key, NoDense, 64> = set;
    assert!(set.contains(&Key(3)));
    assert!(!set.contains(&Key(2)));

    let explicit = cq_storage::build!(
        SortedRun<_, _, I64Codec>,
        [(1i64, 'a'), (1, 'b')].into_iter(),
    );
    let explicit: SortedRun<i64, char, I64Codec> = explicit;
    assert_eq!((&explicit).lookup(&1).collect::<Vec<_>>(), ['a', 'b']);
    fn generic<K: Ord + Copy, V: Copy, I: Iterator<Item = (K, V)>>(
        source: I,
    ) -> SortedRun<K, V, NoDense> {
        cq_storage::build!(SortedRun<K, V, AutoDense>, source)
    }
    let fallback = generic([((&1i64,), 'a')].into_iter());
    assert_eq!((&fallback).lookup(&(&1,)).collect::<Vec<_>>(), ['a']);
    let explicit = cq_storage::build!(
        SortedRun<_, _, NoDense>,
        [((&1i64,), 'a')].into_iter(),
    );
    let explicit: SortedRun<(&i64,), char, NoDense> = explicit;
    assert_eq!((&explicit).lookup(&(&1,)).collect::<Vec<_>>(), ['a']);
    let set = cq_storage::build!(
        AdaptiveKeySet<_, NoDense, 64>,
        [(1i64, ())].into_iter(),
    );
    let set: AdaptiveKeySet<i64, NoDense, 64> = set;
    assert!(set.contains(&1));
}

#[test]
fn routed_sorted_builds_retain_dense_codecs_and_partition_order() {
    use cq_storage::sharded::{Sharded, route_index};
    use cq_storage::{AutoDense, BorrowedI64TupleCodec, NoDense};

    let keys = [1i64, 3, 1, 2];
    for count in [1, 4] {
        let mut parts = vec![Vec::new(); count];
        for (payload, key) in keys.iter().enumerate() {
            parts[route_index(&(key,), count)].push(((key,), payload));
        }
        let run = Sharded::build_from_parts_with_router(
            &parts,
            |part| cq_storage::build!(SortedRun<_, _, AutoDense>, part.iter().copied()),
            |key: &(&i64,), count| route_index(key, count),
        );
        let run: Sharded<SortedRun<(&i64,), usize, BorrowedI64TupleCodec>, _> = run;
        assert_eq!((&run).lookup(&(&1,)).collect::<Vec<_>>(), [0, 2]);
        assert_eq!((&run).lookup(&(&3,)).collect::<Vec<_>>(), [1]);
        assert_eq!((&run).lookup(&(&2,)).collect::<Vec<_>>(), [3]);
        assert!((&run).lookup(&(&4,)).next().is_none());
        let default_run = Sharded::build_from_parts_with_router(
            &parts,
            |part| cq_storage::build!(SortedRun<_, _>, part.iter().copied()),
            |key: &(&i64,), count| route_index(key, count),
        );
        let default_run: Sharded<SortedRun<(&i64,), usize, NoDense>, _> = default_run;
        assert_eq!((&default_run).lookup(&(&1,)).collect::<Vec<_>>(), [0, 2]);
    }
}

#[test]
fn unbuilt_join_filter_is_key_independent_and_preserves_chunk_masks() {
    use cq_storage::{NoJoinFilter, ReadPredicate};

    struct Key;
    let filter = NoJoinFilter;
    assert!(filter.test(&Key));
    assert!(filter.test(&String::from("unbuilt")));
    assert!(filter.any_in(&Key, &Key));
    let mut mask = [true, false, true];
    filter.test_chunk(&[Key, Key, Key], &mut mask);
    assert_eq!(mask, [true, false, true]);
}
