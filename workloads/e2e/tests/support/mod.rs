//! A caller-owned built index, shared by the generated tests and UI consumers.
#![allow(dead_code)]

pub struct SortedIndex<K, V>(Vec<(K, Vec<V>)>);

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
        Self(groups)
    }
}

impl<K: Ord, V> SortedIndex<K, V> {
    fn matching(&self, key: &K) -> &[V] {
        self.0
            .binary_search_by(|(k, _)| k.cmp(key))
            .map_or(&[], |position| self.0[position].1.as_slice())
    }
}

impl<'a, K: Ord, V: Copy> cq_storage::IndexRead for &'a SortedIndex<K, V> {
    type Key = K;
    type Item = V;
    type Cursor = std::iter::Copied<std::slice::Iter<'a, V>>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        self.matching(key).iter().copied()
    }
}

impl<K, V> cq_storage::Keys for SortedIndex<K, V> {
    type Key = K;
    type Iter<'a>
        = std::iter::Map<std::slice::Iter<'a, (K, Vec<V>)>, fn(&'a (K, Vec<V>)) -> &'a K>
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
        !self.matching(key).is_empty()
    }
}
