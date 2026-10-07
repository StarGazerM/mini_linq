//! Each representation is admissible only because an abstraction function `α`
//! commutes with its operations. These tests state `α` through the public
//! traits and check the squares exhaustively over small universes.

use cq_storage::{Build, HashIndex, IndexRead};
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

#[test]
fn index_abstraction_is_the_source_bag_and_lookup_is_exact_selection_in_order() {
    let universe = [(0, 'a'), (0, 'b'), (1, 'a'), (1, 'c'), (2, 'a')];
    for pairs in subsets(&universe) {
        // Multiplicity: duplicate every second pair.
        let source: Vec<_> = pairs
            .iter()
            .enumerate()
            .flat_map(|(i, &p)| std::iter::repeat_n(p, 1 + i % 2))
            .collect();
        let index = HashIndex::build(source.iter().map(|&(k, v)| (k, v)));
        let mut alpha = Bag::new();
        for key in -1..=3 {
            let items: Vec<_> = index.lookup(&key).collect();
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
