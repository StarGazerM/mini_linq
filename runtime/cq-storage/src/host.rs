//! Ordinary standard collections as representations.
//!
//! A host program's own maps are representations too: `α(map) = { (k, v) |
//! v ∈ map[k] }`, and `lookup(k)` enumerates `map[k]` in stored order. The
//! implementations are on the borrowed map so cursors borrow the host data
//! and the handle is a cheap copy, matching [`IndexRead`]'s handle semantics.

use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hash};

use crate::IndexRead;

impl<'a, K: Eq + Hash, V, S: BuildHasher> IndexRead for &'a HashMap<K, Vec<V>, S> {
    type Key = K;
    type Item = &'a V;
    type Cursor = crate::instrument::Cursor<std::slice::Iter<'a, V>>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        crate::instrument::cursor!(self.get(key).map_or([].iter(), |values| values.iter()))
    }
}

impl<'a, K: Ord, V> IndexRead for &'a BTreeMap<K, Vec<V>> {
    type Key = K;
    type Item = &'a V;
    type Cursor = crate::instrument::Cursor<std::slice::Iter<'a, V>>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        crate::instrument::cursor!(self.get(key).map_or([].iter(), |values| values.iter()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lookup_is_selection<'a, I: IndexRead<Key = i32, Item = &'a char>>(
        index: I,
        source: &[(i32, char)],
    ) {
        for key in -1..=3 {
            let expected: Vec<_> = source
                .iter()
                .filter(|(k, _)| *k == key)
                .map(|(_, v)| *v)
                .collect();
            assert_eq!(index.lookup(&key).copied().collect::<Vec<_>>(), expected);
        }
    }
    #[test]
    fn host_maps_answer_lookup_as_exact_selection_in_stored_order() {
        let source = [(0, 'a'), (0, 'b'), (1, 'c'), (2, 'd'), (0, 'e')];
        let mut hash = HashMap::<i32, Vec<char>>::new();
        let mut sorted = BTreeMap::<i32, Vec<char>>::new();
        for (k, v) in source {
            hash.entry(k).or_default().push(v);
            sorted.entry(k).or_default().push(v);
        }
        lookup_is_selection(&hash, &source);
        lookup_is_selection(&sorted, &source);
        // A handle is a cheap copy; cursors keep borrowing the host map.
        let cursor = {
            let handle = &hash;
            handle.lookup(&0)
        };
        assert_eq!(cursor.copied().collect::<Vec<_>>(), ['a', 'b', 'e']);
    }
}
