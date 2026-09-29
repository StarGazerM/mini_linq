//! Candidate sets at a bound prefix. Unlike `IndexRead`, enumeration is
//! distinct even when the represented relation has repeated rows.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, Hash};

/// Distinct keys of an inner level. `key_count()` is exactly the number of
/// items returned by `distinct_keys()`, including for a map of bag buckets.
pub trait Keys {
    type Key;
    type Iter<'a>: Iterator<Item = &'a Self::Key>
    where
        Self: 'a;
    fn key_count(&self) -> usize;
    fn distinct_keys(&self) -> Self::Iter<'_>;
}

impl<K, V, S> Keys for HashMap<K, V, S> {
    type Key = K;
    type Iter<'a>
        = std::collections::hash_map::Keys<'a, K, V>
    where
        Self: 'a;
    fn key_count(&self) -> usize {
        self.len()
    }
    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.keys()
    }
}
impl<K, S> Keys for HashSet<K, S> {
    type Key = K;
    type Iter<'a>
        = std::collections::hash_set::Iter<'a, K>
    where
        Self: 'a;
    fn key_count(&self) -> usize {
        self.len()
    }
    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.iter()
    }
}
impl<K, V> Keys for BTreeMap<K, V> {
    type Key = K;
    type Iter<'a>
        = std::collections::btree_map::Keys<'a, K, V>
    where
        Self: 'a;
    fn key_count(&self) -> usize {
        self.len()
    }
    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.keys()
    }
}
impl<K> Keys for BTreeSet<K> {
    type Key = K;
    type Iter<'a>
        = std::collections::btree_set::Iter<'a, K>
    where
        Self: 'a;
    fn key_count(&self) -> usize {
        self.len()
    }
    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.iter()
    }
}

/// A keyed read at a prefix of a relation `α`.
///
/// `keys(k)` enumerates exactly once each next value `v` for which
/// `(k, v, …) ∈ α`, and nothing else. `count(k)` is an **upper bound** on
/// that cardinality: trie levels count distinct inner keys exactly; flat
/// bag indexes count payload occurrences (bucket length). Missing keys
/// count zero and enumerate nothing. Calls agree during an immutable borrow.
/// Counting must not enumerate the bucket; deferred construction may happen
/// once on first access. Flat bag enumeration deduplicates as it is consumed.
pub trait Candidates {
    type Key;
    type Item;
    type Iter: Iterator<Item = Self::Item>;
    fn count(&self, key: &Self::Key) -> usize;
    fn keys(&self, key: &Self::Key) -> Self::Iter;
}

impl<'a, K: Eq + Hash, V: Eq + Hash, S: BuildHasher> Candidates for &'a HashMap<K, Vec<V>, S> {
    type Key = K;
    type Item = &'a V;
    type Iter = Distinct<std::slice::Iter<'a, V>>;
    fn count(&self, key: &K) -> usize {
        self.get(key).map_or(0, Vec::len)
    }
    fn keys(&self, key: &K) -> Self::Iter {
        Distinct::new(self.get(key).map_or([].iter(), |values| values.iter()))
    }
}
impl<'a, K: Ord, V: Eq + Hash> Candidates for &'a BTreeMap<K, Vec<V>> {
    type Key = K;
    type Item = &'a V;
    type Iter = Distinct<std::slice::Iter<'a, V>>;
    fn count(&self, key: &K) -> usize {
        self.get(key).map_or(0, Vec::len)
    }
    fn keys(&self, key: &K) -> Self::Iter {
        Distinct::new(self.get(key).map_or([].iter(), |values| values.iter()))
    }
}

/// A bag cursor enumerated as a set, preserving first occurrence order.
pub struct Distinct<I: Iterator> {
    source: I,
    seen: HashSet<I::Item>,
}
impl<I: Iterator> Distinct<I> {
    pub(crate) fn new(source: I) -> Self {
        Self {
            source,
            seen: HashSet::new(),
        }
    }
}
impl<I: Iterator> Iterator for Distinct<I>
where
    I::Item: Copy + Eq + Hash,
{
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> {
        self.source.find(|v| self.seen.insert(*v))
    }
}

impl<T: Candidates> Candidates for &T {
    type Key = T::Key;
    type Item = T::Item;
    type Iter = T::Iter;
    fn count(&self, key: &Self::Key) -> usize {
        T::count(*self, key)
    }
    fn keys(&self, key: &Self::Key) -> Self::Iter {
        T::keys(*self, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Build, HashIndex, Trie};

    #[test]
    fn flat_bags_enumerate_distinct_payloads_and_count_an_upper_bound() {
        let rows = [(1, 10), (1, 10), (1, 11), (2, 20)];
        let index: HashIndex<_, _> = Build::build(rows.into_iter());
        let hash: HashMap<_, _> = [(1, vec![10, 10, 11]), (2, vec![20])].into();
        let tree: BTreeMap<_, _> = [(1, vec![10, 10, 11]), (2, vec![20])].into();
        assert_eq!(Candidates::count(&&index, &1), 3);
        assert_eq!(
            Candidates::keys(&&index, &1).collect::<BTreeSet<_>>(),
            [10, 11].into()
        );
        for (count, keys) in [
            (
                Candidates::count(&&hash, &1),
                Candidates::keys(&&hash, &1)
                    .copied()
                    .collect::<BTreeSet<_>>(),
            ),
            (
                Candidates::count(&&tree, &1),
                Candidates::keys(&&tree, &1).copied().collect(),
            ),
        ] {
            assert_eq!(count, 3);
            assert_eq!(keys, [10, 11].into());
        }
        assert_eq!(Candidates::count(&&index, &9), 0);
        assert_eq!(Candidates::keys(&&index, &9).count(), 0);
        assert_eq!(Candidates::count(&&hash, &9), 0);
        assert_eq!(Candidates::keys(&&tree, &9).count(), 0);
        assert_eq!(index.key_count(), 2); // Bucket keys, not payload items.
    }

    #[test]
    fn trie_counts_distinct_next_keys_even_when_inner_buckets_repeat() {
        let rows = [
            (1, (10, 100)),
            (1, (10, 101)),
            (1, (11, 110)),
            (2, (20, 200)),
        ];
        let trie: Trie<'_, _, HashIndex<_, _>> = Build::build(rows.into_iter());
        assert_eq!(Candidates::count(&&trie, &1), 2);
        assert_eq!(
            Candidates::keys(&&trie, &1)
                .copied()
                .collect::<BTreeSet<_>>(),
            [10, 11].into()
        );
        assert_eq!(Candidates::count(&&trie, &9), 0);
        assert_eq!(Candidates::keys(&&trie, &9).count(), 0);
        let sets: Trie<'_, _, HashSet<_>> = Build::build(rows.into_iter());
        assert_eq!(Candidates::count(&&sets, &1), 2);
        assert_eq!(
            Candidates::keys(&&sets, &1)
                .copied()
                .collect::<BTreeSet<_>>(),
            [10, 11].into()
        );
        let map: BTreeMap<_, _> = [(10, vec![1, 2]), (11, vec![3])].into();
        let set: BTreeSet<_> = [10, 11].into();
        assert_eq!(map.key_count(), map.distinct_keys().count());
        assert_eq!(set.key_count(), set.distinct_keys().count());
    }
}
