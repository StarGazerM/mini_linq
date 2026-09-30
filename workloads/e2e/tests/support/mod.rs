//! A caller-owned built index, shared by the generated tests and UI consumers.
#![allow(dead_code)]

pub struct SortedIndex<K, V>(Vec<(K, V)>);

impl<K: Ord, V, I: Iterator<Item = (K, V)>> cq_storage::Build<I> for SortedIndex<K, V> {
    fn build(source: I) -> Self {
        let mut rows: Vec<_> = source.collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        Self(rows)
    }
}

impl<K: Ord, V> SortedIndex<K, V> {
    fn matching(&self, key: &K) -> &[(K, V)] {
        let low = self.0.partition_point(|(k, _)| k < key);
        let high = self.0.partition_point(|(k, _)| k <= key);
        &self.0[low..high]
    }
}

impl<'a, K: Ord, V: Copy> cq_storage::IndexRead for &'a SortedIndex<K, V> {
    type Key = K;
    type Item = V;
    type Cursor = std::iter::Map<std::slice::Iter<'a, (K, V)>, fn(&'a (K, V)) -> V>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        self.matching(key).iter().map(|(_, v)| *v)
    }
}

impl<K: Ord, V> cq_storage::ExactMembership<K> for SortedIndex<K, V> {
    fn contains(&self, key: &K) -> bool {
        !self.matching(key).is_empty()
    }
}
