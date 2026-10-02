//! A caller-owned built index, shared by the generated tests and UI consumers.

use std::collections::BTreeSet;
use std::rc::Rc;

type Bucket<K, V> = (K, Rc<Vec<V>>);
pub struct SortedIndex<K, V>(Rc<Vec<Bucket<K, V>>>);

impl<K, V> Clone for SortedIndex<K, V> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<K: Ord, V, I: Iterator<Item = (K, V)>> cq_storage::Build<I> for SortedIndex<K, V> {
    fn build(source: I) -> Self {
        let mut rows: Vec<_> = source.collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let mut groups: Vec<(K, Vec<V>)> = Vec::new();
        for (key, value) in rows {
            match groups.last_mut() {
                Some((previous, values)) if *previous == key => values.push(value),
                _ => groups.push((key, vec![value])),
            }
        }
        Self(Rc::new(
            groups
                .into_iter()
                .map(|(key, values)| (key, Rc::new(values)))
                .collect(),
        ))
    }
}

impl<K: Ord, V> SortedIndex<K, V> {
    fn matching(&self, key: &K) -> Option<&Rc<Vec<V>>> {
        self.0
            .binary_search_by(|(k, _)| k.cmp(key))
            .ok()
            .map(|position| &self.0[position].1)
    }
}

/// An independent lookup cursor keeps its bucket alive without cloning referents.
pub struct SortedCursor<V> {
    rows: Option<Rc<Vec<V>>>,
    position: usize,
}

impl<V: Copy> Iterator for SortedCursor<V> {
    type Item = V;

    fn next(&mut self) -> Option<V> {
        let value = self.rows.as_ref()?.get(self.position).copied()?;
        self.position += 1;
        Some(value)
    }
}

impl<K: Ord, V: Copy> cq_storage::IndexRead for SortedIndex<K, V> {
    type Key = K;
    type Item = V;
    type Cursor = SortedCursor<V>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        SortedCursor {
            rows: self.matching(key).cloned(),
            position: 0,
        }
    }
}

/// Flat candidate enumeration preserves first occurrence order and set semantics.
pub struct SortedCandidates<V> {
    source: SortedCursor<V>,
    seen: BTreeSet<V>,
}

impl<V: Copy + Ord> Iterator for SortedCandidates<V> {
    type Item = V;

    fn next(&mut self) -> Option<V> {
        self.source.find(|value| self.seen.insert(*value))
    }
}

impl<K: Ord, V: Copy + Ord> cq_storage::Candidates for SortedIndex<K, V> {
    type Key = K;
    type Item = V;
    type Iter = SortedCandidates<V>;

    fn count(&self, key: &K) -> usize {
        // Bucket length is an admissible upper bound; counting never enumerates.
        self.matching(key).map_or(0, |rows| rows.len())
    }

    fn keys(&self, key: &K) -> Self::Iter {
        SortedCandidates {
            source: cq_storage::IndexRead::lookup(self, key),
            seen: BTreeSet::new(),
        }
    }
}

impl<K, V> cq_storage::Keys for SortedIndex<K, V> {
    type Key = K;
    type Iter<'a>
        = std::iter::Map<std::slice::Iter<'a, Bucket<K, V>>, fn(&'a Bucket<K, V>) -> &'a K>
    where
        Self: 'a;

    fn key_count(&self) -> usize {
        self.0.len()
    }

    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.0.iter().map(|(key, _)| key)
    }
}

impl<K: Ord, V> cq_storage::ExactMembership<K> for SortedIndex<K, V> {
    fn contains(&self, key: &K) -> bool {
        self.matching(key).is_some()
    }
}
