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
