//! Immutable hash-routed providers for the task-pool target.
//!
//! The checked Sequence partition routes by the full lookup-key tuple using
//! [`route_index`], then gives each part to exactly one builder. Part order
//! preserves serial occurrence order, including duplicates. The plan fixes P;
//! runtime worker count only schedules the independent builds.
//! Published providers contain no locks or lazy state. Hash routing does not
//! confer ordered/range access, even when an individual shard supports it.

use std::collections::HashMap;
use std::hash::Hash;

use rayon::prelude::*;

use crate::{Build, ExactMembership, HashState, IndexRead};

/// A plan-sized family of immutable indexes with deterministic key routing.
#[derive(Debug)]
pub struct Sharded<I> {
    shards: Vec<I>,
}

impl<I> Sharded<I> {
    /// Wrap indexes built from parts routed by [`route_index`].
    pub fn from_parts(shards: Vec<I>) -> Self {
        assert!(!shards.is_empty(), "routed provider needs positive partition count");
        Self { shards }
    }

    /// Build one index per already routed part, without rerouting or copying.
    pub fn build_parts<K: Send, V: Send>(parts: Vec<Vec<(K, V)>>) -> Self
    where I: Build<std::vec::IntoIter<(K, V)>> + Send {
        Self::from_parts(parts.into_par_iter().map(|part| I::build(part.into_iter())).collect())
    }
    /// Number of independently built indexes fixed by the plan.
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    #[inline]
    fn shard<K: Hash + ?Sized>(&self, key: &K) -> &I {
        &self.shards[route_index(key, self.shards.len())]
    }
}

impl<I, K: Hash + ?Sized> ExactMembership<K> for Sharded<I>
where
    I: ExactMembership<K>,
{
    #[inline]
    fn contains(&self, key: &K) -> bool {
        self.shard(key).contains(key)
    }
}

// Concrete native families keep trait selection non-recursive: a blanket
// `&I: IndexRead` wrapper makes higher-ranked cursor inference recurse forever.
macro_rules! routed_read {
    ([$($generic:ident),*] $index:ty) => {
        impl<'a, $($generic: 'a),*> IndexRead for &'a Sharded<$index>
        where
            K: Hash,
            &'a $index: IndexRead<Key = K>,
        {
            type Key = K;
            type Item = <&'a $index as IndexRead>::Item;
            type Cursor = <&'a $index as IndexRead>::Cursor;
            fn lookup(&self, key: &K) -> Self::Cursor {
                let shard: &'a $index = self.shard(key);
                shard.lookup(key)
            }
        }
    };
}
routed_read!([K, V] Index<K, V>);
routed_read!([K, V, C] crate::SortedRun<K, V, C>);
routed_read!([K, V] std::collections::BTreeMap<K, Vec<V>>);

/// Shared routing function used by the Sequence key partition and its provider.
#[inline]
pub fn route_index<K: Hash + ?Sized>(key: &K, count: usize) -> usize {
    assert!(count > 0, "key partition count must be positive");
    (ahash::RandomState::with_seeds(0, 0, 0, 0).hash_one(key) % count as u64) as usize
}

/// Eager, shareable hash buckets with the pull emitter's copy-payload ABI.
/// Unlike the serial lazy `HashIndex`, this provider has no Rc/RefCell state.
#[derive(Debug)]
pub struct Index<K, V> {
    buckets: HashMap<K, Vec<V>, HashState>,
}

impl<K: Eq + Hash, V, S: Iterator<Item = (K, V)>> Build<S> for Index<K, V> {
    fn build(source: S) -> Self {
        crate::count!(index_builds);
        let mut buckets = HashMap::with_hasher(HashState::default());
        for (key, value) in source {
            crate::count!(rows_copied);
            buckets.entry(key).or_insert_with(Vec::new).push(value);
        }
        Self { buckets }
    }
}

impl<'a, K: Eq + Hash, V: Copy> IndexRead for &'a Index<K, V> {
    type Key = K;
    type Item = V;
    type Cursor = std::iter::Copied<std::slice::Iter<'a, V>>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        self.buckets.get(key).map_or(&[][..], Vec::as_slice).iter().copied()
    }
}

impl<K, Q: Eq + Hash + ?Sized, V> ExactMembership<Q> for Index<K, V>
where
    K: Eq + Hash + std::borrow::Borrow<Q>,
{
    fn contains(&self, key: &Q) -> bool {
        crate::count!(lookups);
        self.buckets.contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routed_hits_misses_duplicates_and_empty() {
        for threads in [1, 24] {
            rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
                for count in [1, 7, 32] {
                    for rows in [vec![], vec![(1u64, 2u64), (1, 2), (1, 3), (9, 4)]] {
                        let mut parts: Vec<_> = (0..count).map(|_| vec![]).collect();
                        for &pair in &rows { parts[route_index(&pair.0, count)].push(pair); }
                        let index = Sharded::<Index<u64,u64>>::build_parts(parts.clone());
                        let seen = Sharded::<crate::HashSet<u64>>::build_parts(parts);
                        assert_eq!(index.shard_count(), count);
                        for key in 0..12 {
                            let expected: Vec<_> = rows.iter().filter(|r| r.0 == key).map(|r| r.1).collect();
                            assert_eq!((&index).lookup(&key).collect::<Vec<_>>(), expected);
                            assert_eq!(seen.contains(&key), !expected.is_empty());
                        }
                    }
                }
            });
        }
    }
}
