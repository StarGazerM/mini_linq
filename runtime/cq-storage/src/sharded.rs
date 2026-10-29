//! Immutable hash-routed providers for the task-pool target.
//!
//! The checked Sequence partition routes by a lookup-key projection using
//! [`route_index`], then gives each part to exactly one builder. The retained
//! [`Router`] must apply exactly that projection and field order. Part order
//! preserves serial occurrence order, including duplicates. The checked plan
//! fixes P, possibly through recorded guards; workers schedule the chosen build.
//! Published providers contain no locks or lazy state. Hash routing does not
//! confer ordered/range access, even when an individual shard supports it.
//! Eager hash buckets keep their first payload inline, so distinct-key builds
//! do not allocate a separate vector per row. Duplicate tails retain input order.

use crate::instrument::HashMap;
use std::hash::Hash;
use std::sync::Mutex;

use crate::parallel::{for_each_index, map_index};

use crate::{Build, ExactMembership, IndexRead};

/// Routing policy recorded with a provider; must match its input partition.
pub trait Router<K: ?Sized> {
    fn route(&self, key: &K, count: usize) -> usize;
}

/// Default policy hashes the complete lookup key.
#[derive(Debug, Clone, Copy, Default)]
pub struct FullKey;

impl<K: Hash + ?Sized> Router<K> for FullKey {
    fn route(&self, key: &K, count: usize) -> usize {
        route_index(key, count)
    }
}

impl<K: ?Sized, F: Fn(&K, usize) -> usize> Router<K> for F {
    fn route(&self, key: &K, count: usize) -> usize {
        self(key, count)
    }
}

/// A plan-sized family of immutable indexes with deterministic key routing.
///
/// Routing uniqueness law: all occurrences of an equal lookup key belong to
/// exactly one shard, selected by the retained router (or the sole shard).
/// Constructors accept already partitioned input and rely on this law. Thus
/// `Keys` concatenates shard directories and sums their counts without
/// deduplicating keys, routing again, or enumerating payloads.
#[derive(Debug)]
pub struct Sharded<I, F = FullKey> {
    shards: Vec<I>,
    router: F,
    release: fn(&mut Vec<I>),
}

/// A checked provider cover over an immutable input.
///
/// `None` records a singleton cover: no row-part buffer is allocated or copied.
/// `Some` owns the already routed parts. Usually the source closure replays an
/// existing publication. A checked single-consumer `DeferredRows` publication
/// may instead consume its pure producer once, or replay its lazily owned rows
/// after a length/routing demand. This is not general lazy Store semantics.
pub struct ProviderParts<S, R> {
    source: S,
    routed: Option<Vec<Vec<R>>>,
}

impl<S, R, I> ProviderParts<S, R>
where
    S: Fn() -> I,
    I: Iterator<Item = R>,
    R: Clone,
{
    pub fn new(source: S, routed: Option<Vec<Vec<R>>>) -> Self {
        Self { source, routed }
    }

    /// Owned row buffers exist only for the recorded routed arm.
    pub fn routed(&self) -> Option<&[Vec<R>]> {
        self.routed.as_deref()
    }

    /// Read the checked source for a singleton native build.
    pub fn source(&self) -> I {
        (self.source)()
    }

    pub fn part_count(&self) -> usize {
        self.routed.as_ref().map_or(1, Vec::len)
    }

    /// Replay one checked part without allocating a row buffer.
    pub fn part<'a>(&'a self, index: usize) -> impl Iterator<Item = R> + 'a
    where
        I: 'a,
    {
        assert!(index < self.part_count(), "provider part index");
        self.routed
            .is_none()
            .then(|| self.source())
            .into_iter()
            .flatten()
            .chain(
                self.routed
                    .as_ref()
                    .into_iter()
                    .flat_map(move |parts| parts[index].iter().cloned()),
            )
    }

    pub fn rows<'a>(&'a self) -> impl Iterator<Item = R> + 'a
    where
        I: 'a,
    {
        (0..self.part_count()).flat_map(|index| self.part(index))
    }
}

impl<I> Sharded<I> {
    /// Wrap indexes built from parts routed by [`route_index`].
    pub fn from_parts(shards: Vec<I>) -> Self {
        Self::from_parts_with_router(shards, FullKey)
    }

    /// Build one index per already routed part, without rerouting or copying.
    pub fn build_parts<K: Send, V: Send>(parts: Vec<Vec<(K, V)>>) -> Self
    where
        I: Build<std::vec::IntoIter<(K, V)>> + Send,
    {
        Self::build_parts_with_router(parts, FullKey)
    }
}

impl<I, F> Sharded<I, F> {
    /// Wrap parts with the routing policy used to partition their source.
    pub fn from_parts_with_router(shards: Vec<I>, router: F) -> Self {
        assert!(
            !shards.is_empty(),
            "routed provider needs positive partition count"
        );
        Self {
            shards,
            router,
            release: Vec::clear,
        }
    }

    /// Build already routed parts and retain the lookup-key projection policy.
    pub fn build_parts_with_router<K: Send, V: Send>(parts: Vec<Vec<(K, V)>>, router: F) -> Self
    where
        I: Build<std::vec::IntoIter<(K, V)>> + Send,
    {
        if parts.len() == 1 {
            let part = parts.into_iter().next().expect("singleton cover");
            return Self::from_parts_with_router(vec![I::build(part.into_iter())], router);
        }
        #[cfg(any(feature = "counters", feature = "operator-timing"))]
        let context = crate::counters::TaskContext::capture();
        let parts: Vec<_> = parts
            .into_iter()
            .map(|part| Mutex::new(Some(part)))
            .collect();
        Self::from_parts_with_router(
            map_index(parts.len(), &|index| {
                let part = parts[index].lock().unwrap().take().expect("owned part");
                #[cfg(any(feature = "counters", feature = "operator-timing"))]
                {
                    context.run(|| I::build(part.into_iter()))
                }
                #[cfg(not(any(feature = "counters", feature = "operator-timing")))]
                {
                    I::build(part.into_iter())
                }
            }),
            router,
        )
        .with_parallel_release()
    }

    /// Build each recorded row part directly, without first copying all pairs
    /// into another set of buffers. A singleton cover is a serial host build.
    pub fn build_from_parts_with_router<'a, R: Sync>(
        parts: &'a [Vec<R>],
        build: impl Fn(&'a [R]) -> I + Sync + Send,
        router: F,
    ) -> Self
    where
        I: Send,
    {
        if parts.len() == 1 {
            return Self::from_parts_with_router(vec![build(&parts[0])], router);
        }
        #[cfg(any(feature = "counters", feature = "operator-timing"))]
        let context = crate::counters::TaskContext::capture();
        Self::from_parts_with_router(
            map_index(parts.len(), &|index| {
                let part = &parts[index];
                #[cfg(any(feature = "counters", feature = "operator-timing"))]
                {
                    context.run(|| build(part))
                }
                #[cfg(not(any(feature = "counters", feature = "operator-timing")))]
                {
                    build(part)
                }
            }),
            router,
        )
        .with_parallel_release()
    }

    fn with_parallel_release(mut self) -> Self
    where
        I: Send,
    {
        self.release = release_parallel::<I>;
        self
    }
    /// Number of independently built indexes fixed by the plan.
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    #[inline]
    fn shard<K: ?Sized>(&self, key: &K) -> &I
    where
        F: Router<K>,
    {
        if self.shards.len() == 1 {
            &self.shards[0]
        } else {
            &self.shards[self.router.route(key, self.shards.len())]
        }
    }
}

impl<I, F> Drop for Sharded<I, F> {
    fn drop(&mut self) {
        // The constructor installs this only where Send was proved. Manual
        // from_parts wrappers retain their ordinary serial destruction.
        (self.release)(&mut self.shards);
    }
}

fn release_parallel<I: Send>(shards: &mut Vec<I>) {
    if shards.len() <= 1 || rayon::current_num_threads() == 1 {
        shards.clear();
        return;
    }
    #[cfg(any(feature = "counters", feature = "operator-timing"))]
    let context = crate::counters::TaskContext::capture();
    let slots: Vec<_> = std::mem::take(shards)
        .into_iter()
        .map(|shard| Mutex::new(Some(shard)))
        .collect();
    for_each_index(slots.len(), &|index| {
        let shard = slots[index].lock().unwrap().take().expect("owned shard");
        #[cfg(any(feature = "counters", feature = "operator-timing"))]
        {
            context.run(|| drop(shard));
        }
        #[cfg(not(any(feature = "counters", feature = "operator-timing")))]
        {
            drop(shard);
        }
    });
}

impl<K, V, F> Sharded<Index<K, V>, F> {
    // Retained inherent accessors share the same routing uniqueness premise.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.shards.iter().flat_map(Index::keys)
    }
    pub fn len(&self) -> usize {
        self.shards.iter().map(Index::len).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.shards.iter().all(Index::is_empty)
    }
}

impl<K: Ord + Copy, V: Copy, C: crate::KeyCodec<K>, F> Sharded<crate::SortedRun<K, V, C>, F> {
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.shards.iter().flat_map(crate::SortedRun::keys)
    }
    pub fn len(&self) -> usize {
        self.shards.iter().map(crate::SortedRun::len).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.shards.iter().all(crate::SortedRun::is_empty)
    }
}

/// Borrowed shard directories, traversed without allocating or copying keys.
pub struct ShardKeys<'a, I: crate::Keys + 'a> {
    shards: std::slice::Iter<'a, I>,
    current: Option<I::Iter<'a>>,
}

impl<'a, I: crate::Keys + 'a> Iterator for ShardKeys<'a, I> {
    type Item = &'a I::Key;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(key) = self.current.as_mut().and_then(Iterator::next) {
                return Some(key);
            }
            self.current = Some(self.shards.next()?.distinct_keys());
        }
    }
}

impl<I: crate::Keys, F> crate::Keys for Sharded<I, F> {
    type Key = I::Key;
    type Iter<'a>
        = ShardKeys<'a, I>
    where
        Self: 'a;

    fn key_count(&self) -> usize {
        self.shards.iter().map(crate::Keys::key_count).sum()
    }

    fn distinct_keys(&self) -> Self::Iter<'_> {
        ShardKeys {
            shards: self.shards.iter(),
            current: None,
        }
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
            #[inline]
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
    if count == 1 {
        return 0;
    }
    crate::count!(hash_computations);
    (ahash::RandomState::with_seeds(0, 0, 0, 0).hash_one(key) % count as u64) as usize
}

/// Eager, shareable hash buckets with the pull emitter's copy-payload ABI.
/// Unlike the serial lazy `HashIndex`, this provider has no Rc/RefCell state.
pub struct Index<K, V> {
    buckets: HashMap<K, Bucket<V>>,
    #[cfg(feature = "counters")]
    _allocation: crate::counters_memory::Allocation,
}

#[derive(Debug)]
struct Bucket<V> {
    first: V,
    tail: Vec<V>,
}
impl<K: std::fmt::Debug, V: std::fmt::Debug> std::fmt::Debug for Index<K, V> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Index")
            .field("buckets", &self.buckets)
            .finish()
    }
}

impl<K, V> Index<K, V> {
    /// Exact distinct build keys, used by conservative covering join filters.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.buckets.keys()
    }

    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }
}

/// Borrowed hash-directory keys; bucket payloads are never traversed.
pub struct IndexKeys<'a, K, V> {
    source: crate::instrument::Cursor<std::collections::hash_map::Keys<'a, K, Bucket<V>>>,
}

impl<'a, K, V> Iterator for IndexKeys<'a, K, V> {
    type Item = &'a K;

    fn next(&mut self) -> Option<Self::Item> {
        self.source.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.source.size_hint()
    }
}

impl<K, V> crate::Keys for Index<K, V> {
    type Key = K;
    type Iter<'a>
        = IndexKeys<'a, K, V>
    where
        Self: 'a;

    fn key_count(&self) -> usize {
        self.buckets.len()
    }

    fn distinct_keys(&self) -> Self::Iter<'_> {
        IndexKeys {
            source: crate::instrument::cursor!(self.buckets.keys()),
        }
    }
}

impl<K: Eq + Hash, V, S: Iterator<Item = (K, V)>> Build<S> for Index<K, V> {
    fn build(source: S) -> Self {
        crate::count!(index_builds);
        let mut buckets: HashMap<K, Bucket<V>> = HashMap::default();
        for (key, value) in source {
            crate::count!(rows_copied);
            match buckets.entry(key) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().tail.push(value);
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(Bucket {
                        first: value,
                        tail: Vec::new(),
                    });
                }
            }
        }
        #[cfg(feature = "counters")]
        let allocation = {
            crate::counters::build_cardinality(buckets.len());
            let bytes = buckets.capacity() * (std::mem::size_of::<(K, Bucket<V>)>() + 1)
                + buckets
                    .values()
                    .map(|bucket| bucket.tail.capacity() * std::mem::size_of::<V>())
                    .sum::<usize>();
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
    type Cursor = crate::instrument::Cursor<
        std::iter::Chain<std::option::IntoIter<V>, std::iter::Copied<std::slice::Iter<'a, V>>>,
    >;

    fn lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        let bucket = self.buckets.get(key);
        crate::instrument::cursor!(
            bucket.map(|bucket| bucket.first).into_iter().chain(
                bucket
                    .map_or(&[][..], |bucket| bucket.tail.as_slice())
                    .iter()
                    .copied()
            )
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

    fn assert_distinct_keys<I: crate::Keys<Key = u64>>(index: &I, expected: &[u64]) {
        let mut keys = crate::Keys::distinct_keys(index)
            .copied()
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(crate::Keys::key_count(index), keys.len());
    }

    #[test]
    fn key_directories_ignore_duplicate_payloads_and_empty_shards() {
        for count in [1, 7, 32] {
            for rows in [vec![], vec![(1u64, 2u64), (1, 2), (1, 3), (9, 4)]] {
                let expected: &[u64] = if rows.is_empty() { &[] } else { &[1, 9] };
                let native = Index::build(rows.iter().copied());
                assert_distinct_keys(&native, expected);
                let mut parts: Vec<_> = (0..count).map(|_| vec![]).collect();
                for &pair in &rows {
                    parts[route_index(&pair.0, count)].push(pair);
                }
                let hash = Sharded::from_parts(
                    parts
                        .iter()
                        .map(|part| Index::build(part.iter().copied()))
                        .collect(),
                );
                let sorted = Sharded::from_parts(
                    parts
                        .iter()
                        .map(|part| crate::SortedRun::<_, _>::build(part.iter().copied()))
                        .collect(),
                );
                // Exercise the generic capability with a provider that has
                // no specialized Sharded inherent key accessors.
                let trees = Sharded::from_parts(
                    parts
                        .iter()
                        .map(|part| {
                            let mut tree = std::collections::BTreeMap::<_, Vec<_>>::new();
                            for &(key, value) in part {
                                tree.entry(key).or_default().push(value);
                            }
                            tree
                        })
                        .collect(),
                );
                assert_distinct_keys(&hash, expected);
                assert_distinct_keys(&sorted, expected);
                assert_distinct_keys(&trees, expected);
            }
        }
    }

    #[test]
    fn owned_build_preserves_non_clone_non_sync_rows_and_releases_once() {
        use std::cell::Cell;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Payload {
            value: Cell<usize>,
            dropped: Arc<AtomicUsize>,
        }
        impl Drop for Payload {
            fn drop(&mut self) {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        let dropped = Arc::new(AtomicUsize::new(0));
        let parts = (0..8)
            .map(|part| {
                (0..3)
                    .map(|offset| {
                        (
                            part,
                            Payload {
                                value: Cell::new(part * 3 + offset),
                                dropped: dropped.clone(),
                            },
                        )
                    })
                    .collect()
            })
            .collect();
        let provider = Sharded::<Index<usize, Payload>>::build_parts(parts);
        for (part, shard) in provider.shards.iter().enumerate() {
            let bucket = &shard.buckets[&part];
            assert_eq!(bucket.first.value.get(), part * 3);
            assert_eq!(
                bucket
                    .tail
                    .iter()
                    .map(|row| row.value.get())
                    .collect::<Vec<_>>(),
                [part * 3 + 1, part * 3 + 2],
            );
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
        drop(provider);
        assert_eq!(dropped.load(Ordering::Relaxed), 24);
    }

    #[test]
    fn identity_cover_builds_on_host_and_never_routes_reads() {
        let host = std::thread::current().id();
        let parts = vec![vec![(1u32, 7u32), (1, 8), (2, 9)]];
        let provider = Sharded::build_from_parts_with_router(
            &parts,
            |rows| {
                assert_eq!(std::thread::current().id(), host);
                Index::build(rows.iter().copied())
            },
            |_: &u32, _: usize| -> usize { panic!("identity cover must not hash-route") },
        );
        assert_eq!(provider.shard_count(), 1);
        assert!(provider.contains(&1));
        assert_eq!((&provider).lookup(&1).collect::<Vec<_>>(), vec![7, 8]);
        let mut keys = provider.keys().copied().collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, [1, 2]);
        assert_eq!(provider.len(), 2);
    }

    #[test]
    fn parallel_provider_release_joins_all_native_destructors() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Native(Arc<AtomicUsize>);
        impl Drop for Native {
            fn drop(&mut self) {
                assert!(rayon::current_thread_index().is_some());
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let count = Arc::new(AtomicUsize::new(0));
        rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
            .install(|| {
                let parts: Vec<Vec<u8>> = (0..64).map(|_| Vec::new()).collect();
                let provider = Sharded::build_from_parts_with_router(
                    &parts,
                    |_| Native(count.clone()),
                    FullKey,
                );
                drop(provider);
            });
        assert_eq!(count.load(Ordering::Relaxed), 64);
    }
    #[test]
    fn singleton_buckets_do_not_allocate_payload_vectors() {
        let index = Index::build((0..128).map(|key| (key, key * 2)));
        assert_eq!(index.buckets.len(), 128);
        assert!(
            index
                .buckets
                .values()
                .all(|bucket| bucket.tail.capacity() == 0)
        );
        for key in 0..128 {
            assert_eq!((&index).lookup(&key).collect::<Vec<_>>(), [key * 2]);
        }
    }

    #[test]
    fn projected_routing_preserves_full_lookup_keys() {
        for threads in [1, 24] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    for count in [1, 7, 32] {
                        let rows = [((2u64, 7u64), 3u64), ((2, 8), 4), ((2, 7), 3), ((9, 7), 5)];
                        let router = |key: &(u64, u64), n| route_index(&(key.1,), n);
                        let mut parts: Vec<_> = (0..count).map(|_| vec![]).collect();
                        for row in rows {
                            parts[router(&row.0, count)].push(row);
                        }
                        let index = Sharded::<Index<(u64, u64), u64>, _>::build_parts_with_router(
                            parts.clone(),
                            router,
                        );
                        let seen =
                            Sharded::<crate::HashSet<(u64, u64)>, _>::build_parts_with_router(
                                parts, router,
                            );
                        let mut keys = crate::Keys::distinct_keys(&index)
                            .copied()
                            .collect::<Vec<_>>();
                        keys.sort_unstable();
                        assert_eq!(keys, [(2, 7), (2, 8), (9, 7)]);
                        assert_eq!(crate::Keys::key_count(&index), keys.len());
                        for key in [(2, 7), (2, 8), (9, 7), (9, 8), (0, 0)] {
                            let expected: Vec<_> =
                                rows.iter().filter(|r| r.0 == key).map(|r| r.1).collect();
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
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    for count in [1, 7, 32] {
                        for rows in [vec![], vec![(1u64, 2u64), (1, 2), (1, 3), (9, 4)]] {
                            let mut parts: Vec<_> = (0..count).map(|_| vec![]).collect();
                            for &pair in &rows {
                                parts[route_index(&pair.0, count)].push(pair);
                            }
                            let index = Sharded::<Index<u64, u64>>::build_parts(parts.clone());
                            let seen = Sharded::<crate::HashSet<u64>>::build_parts(parts);
                            assert_eq!(index.shard_count(), count);
                            for key in 0..12 {
                                let expected: Vec<_> =
                                    rows.iter().filter(|r| r.0 == key).map(|r| r.1).collect();
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
            result: "shards",
            kind: "INDEX_BUILD",
            key: "k",
            inputs: &[],
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
