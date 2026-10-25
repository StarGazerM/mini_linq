//! Immutable hash-routed providers for the task-pool target.
//!
//! The checked Sequence partition routes by a lookup-key projection using
//! [`route_index`], then gives each part to exactly one builder. The retained
//! [`Router`] must apply exactly that projection and field order. Part order
//! preserves serial occurrence order, including duplicates. The plan fixes P;
//! runtime worker count only schedules the independent builds.
//! Published providers contain no locks or lazy state. Hash routing does not
//! confer ordered/range access, even when an individual shard supports it.

use crate::instrument::HashMap;
use std::hash::Hash;

use rayon::prelude::*;

use crate::{Build, ExactMembership, IndexRead};

/// Routing policy recorded with a provider; must match its input partition.
pub trait Router<K: ?Sized> {
    fn route(&self, key: &K, count: usize) -> usize;
}

/// Default policy hashes the complete lookup key.
#[derive(Debug, Clone, Copy, Default)]
pub struct FullKey;

impl<K: Hash + ?Sized> Router<K> for FullKey {
    fn route(&self, key: &K, count: usize) -> usize { route_index(key, count) }
}

impl<K: ?Sized, F: Fn(&K, usize) -> usize> Router<K> for F {
    fn route(&self, key: &K, count: usize) -> usize { self(key, count) }
}

/// A plan-sized family of immutable indexes with deterministic key routing.
#[derive(Debug)]
pub struct Sharded<I, F = FullKey> {
    shards: Vec<I>,
    router: F,
}

impl<I> Sharded<I> {
    /// Wrap indexes built from parts routed by [`route_index`].
    pub fn from_parts(shards: Vec<I>) -> Self {
        Self::from_parts_with_router(shards, FullKey)
    }

    /// Build one index per already routed part, without rerouting or copying.
    pub fn build_parts<K: Send, V: Send>(parts: Vec<Vec<(K, V)>>) -> Self
    where I: Build<std::vec::IntoIter<(K, V)>> + Send {
        Self::build_parts_with_router(parts, FullKey)
    }
}

impl<I, F> Sharded<I, F> {
    /// Wrap parts with the routing policy used to partition their source.
    pub fn from_parts_with_router(shards: Vec<I>, router: F) -> Self {
        assert!(!shards.is_empty(), "routed provider needs positive partition count");
        Self { shards, router }
    }

    /// Build already routed parts and retain the lookup-key projection policy.
    pub fn build_parts_with_router<K: Send, V: Send>(
        parts: Vec<Vec<(K, V)>>, router: F,
    ) -> Self
    where I: Build<std::vec::IntoIter<(K, V)>> + Send {
        #[cfg(any(feature = "counters", feature = "operator-timing"))]
        let context = crate::counters::TaskContext::capture();
        Self::from_parts_with_router(
            parts.into_par_iter().map(|part| {
                #[cfg(any(feature = "counters", feature = "operator-timing"))]
                { context.run(|| I::build(part.into_iter())) }
                #[cfg(not(any(feature = "counters", feature = "operator-timing")))]
                { I::build(part.into_iter()) }
            }).collect(),
            router,
        )
    }
    /// Number of independently built indexes fixed by the plan.
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    #[inline]
    fn shard<K: ?Sized>(&self, key: &K) -> &I where F: Router<K> {
        &self.shards[self.router.route(key, self.shards.len())]
    }
}

impl<I, F, K: ?Sized> ExactMembership<K> for Sharded<I, F>
where
    I: ExactMembership<K>,
    F: Router<K>,
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
        impl<'a, F: 'a, $($generic: 'a),*> IndexRead for &'a Sharded<$index, F>
        where
            F: Router<K>,
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
    crate::count!(hash_computations);
    (ahash::RandomState::with_seeds(0, 0, 0, 0).hash_one(key) % count as u64) as usize
}

/// Eager, shareable hash buckets with the pull emitter's copy-payload ABI.
/// Unlike the serial lazy `HashIndex`, this provider has no Rc/RefCell state.
pub struct Index<K, V> {
    buckets: HashMap<K, Vec<V>>,
    #[cfg(feature = "counters")]
    _allocation: crate::counters_memory::Allocation,
}
impl<K: std::fmt::Debug, V: std::fmt::Debug> std::fmt::Debug for Index<K, V> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Index").field("buckets", &self.buckets).finish()
    }
}

impl<K: Eq + Hash, V, S: Iterator<Item = (K, V)>> Build<S> for Index<K, V> {
    fn build(source: S) -> Self {
        crate::count!(index_builds);
        let mut buckets = HashMap::default();
        for (key, value) in source {
            crate::count!(rows_copied);
            buckets.entry(key).or_insert_with(Vec::new).push(value);
        }
        #[cfg(feature = "counters")]
        let allocation = {
            crate::counters::build_cardinality(buckets.len());
            let bytes = buckets.capacity() * (std::mem::size_of::<(K, Vec<V>)>() + 1)
                + buckets.values().map(|values| values.capacity() * std::mem::size_of::<V>()).sum::<usize>();
            crate::counters_memory::Allocation::new(bytes)
        };
        Self {
            buckets,
            #[cfg(feature = "counters")]
            _allocation: allocation,
        }
    }
}

impl<'a, K: Eq + Hash, V: Copy> IndexRead for &'a Index<K, V> {
    type Key = K;
    type Item = V;
    type Cursor = crate::instrument::Cursor<std::iter::Copied<std::slice::Iter<'a, V>>>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        crate::instrument::cursor!(
            self.buckets.get(key).map_or(&[][..], Vec::as_slice).iter().copied()
        )
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
    fn projected_routing_preserves_full_lookup_keys() {
        for threads in [1, 24] {
            rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
                for count in [1, 7, 32] {
                    let rows = [((2u64, 7u64), 3u64), ((2, 8), 4), ((2, 7), 3), ((9, 7), 5)];
                    let router = |key: &(u64, u64), n| route_index(&(key.1,), n);
                    let mut parts: Vec<_> = (0..count).map(|_| vec![]).collect();
                    for row in rows { parts[router(&row.0, count)].push(row); }
                    let index = Sharded::<Index<(u64,u64),u64>, _>::build_parts_with_router(parts.clone(), router);
                    let seen = Sharded::<crate::HashSet<(u64,u64)>, _>::build_parts_with_router(parts, router);
                    for key in [(2,7), (2,8), (9,7), (9,8), (0,0)] {
                        let expected: Vec<_> = rows.iter().filter(|r| r.0 == key).map(|r| r.1).collect();
                        assert_eq!((&index).lookup(&key).collect::<Vec<_>>(), expected);
                        assert_eq!(seen.contains(&key), !expected.is_empty());
                    }
                }
            });
        }
    }
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
    #[cfg(feature = "counters")]
    #[test]
    fn profile_accumulates_shard_builds_cardinality_and_cursor_work() {
        use crate::counters::{self, OperatorLabel, OperatorProgram};
        counters::reset();
        let profile = OperatorProgram::new(&[OperatorLabel {
            result: "shards", kind: "INDEX_BUILD", key: "k", inputs: &[],
        }]);
        let index = profile.build(0, || {
            let mut parts = vec![Vec::new(); 7];
            for pair in profile.inserting(0, [(1u64, 7u64), (1, 8), (9, 3)].into_iter()) {
                parts[route_index(&pair.0, 7)].push(pair);
            }
            Sharded::<Index<u64, u64>>::build_parts(parts)
        });
        assert_eq!((&index).lookup(&1).collect::<Vec<_>>(), [7, 8]);
        let counts = counters::snapshot();
        assert_eq!(counts.index_builds, 7);
        assert_eq!(counts.rows_copied, 3);
        assert_eq!(counts.lookups, 1);
        assert_eq!(counts.cursor_steps, 3);
        assert_eq!(counts.rows_enumerated, 2);
        assert!(counts.hash_computations > 0);
        assert!(counts.bytes_retained_current > 0);
        let operators = counters::operators();
        assert_eq!(operators[0].rows_inserted, 3);
        assert_eq!(operators[0].rows_out, 2);
        drop(index);
        assert_eq!(counters::snapshot().bytes_retained_current, 0);
    }
}
