//! Hash-partitioned native layout: a grouped map `K -> [V]` split into a fixed
//! power-of-two set of disjoint shards (DL-ShardedLayout).
//!
//! It denotes exactly the grouped map obtained by inserting the same rows into
//! one `HashMap<K, Vec<V>, S>`: each key lives in the one shard selected by
//! its hash, and a bucket keeps its rows' insertion order. Reads hash a key
//! once and probe one shard. Bulk insertion partitions rows by shard and fills
//! every shard in parallel, each by one task with exclusive access, so one
//! layout's publication is no longer a single serial loop.
use cq_storage::rows::{Columns, Concat, Rows};
use cq_storage::{Candidates, ExactMembership, HashState, IndexRead, Keys};
use hashbrown::HashTable;
use hashbrown::hash_table::Entry;
use rayon::prelude::*;
use std::borrow::Borrow;
use std::collections::HashSet;
use std::hash::{BuildHasher, Hash};

/// Rows below this count are inserted on the calling thread.
const PARALLEL_ROWS: usize = 4096;

type Shard<K, V> = HashTable<(K, Vec<V>)>;

pub struct ShardedLayout<K, V, S = HashState> {
    shards: Vec<Shard<K, V>>,
    hasher: S,
}

fn shard_count() -> usize {
    rayon::current_num_threads().next_power_of_two()
}

impl<K, V, S: Default> Default for ShardedLayout<K, V, S> {
    fn default() -> Self {
        Self::with_hasher(S::default())
    }
}

impl<K, V, S> ShardedLayout<K, V, S> {
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            shards: (0..shard_count()).map(|_| HashTable::new()).collect(),
            hasher,
        }
    }

    /// Route by the middle hash bits: hashbrown indexes buckets by the low bits
    /// and tags by the top seven, so neither loses entropy within a shard.
    fn shard(&self, hash: u64) -> usize {
        (hash >> 32) as usize & (self.shards.len() - 1)
    }

    /// Remove every row, keeping shard capacity.
    pub fn clear(&mut self) {
        for shard in &mut self.shards {
            shard.clear();
        }
    }

    /// Number of distinct keys.
    pub fn len(&self) -> usize {
        self.shards.iter().map(HashTable::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.shards.iter().all(HashTable::is_empty)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &Vec<V>)> {
        self.shards
            .iter()
            .flat_map(|shard| shard.iter().map(|(k, vs)| (k, vs)))
    }
}

impl<K: Eq + Hash, V, S: BuildHasher> ShardedLayout<K, V, S> {
    pub fn get<Q>(&self, key: &Q) -> Option<&Vec<V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let hash = self.hasher.hash_one(key);
        self.shards[self.shard(hash)]
            .find(hash, |(k, _)| k.borrow() == key)
            .map(|(_, values)| values)
    }

    fn push_hashed(shard: &mut Shard<K, V>, hasher: &S, hash: u64, key: K, value: V) {
        match shard.entry(hash, |(k, _)| *k == key, |(k, _)| hasher.hash_one(k)) {
            Entry::Occupied(mut entry) => entry.get_mut().1.push(value),
            Entry::Vacant(entry) => {
                entry.insert((key, vec![value]));
            }
        }
    }

    /// Append one row to its key's bucket.
    pub fn push(&mut self, key: K, value: V) {
        let hash = self.hasher.hash_one(&key);
        let shard = self.shard(hash);
        Self::push_hashed(&mut self.shards[shard], &self.hasher, hash, key, value);
    }

    /// Formal rule DL-ShardedLayout: route each row to its key's shard and fill every shard by one exclusive task, preserving row order within buckets.
    ///
    /// Appends `key(row) -> value(row)` for every row. Large inputs are hashed
    /// and inserted in parallel; the result equals sequential [`Self::push`].
    pub fn extend_from<T, FK, FV>(&mut self, rows: &[T], key: FK, value: FV)
    where
        T: Sync,
        K: Send,
        V: Send,
        S: Sync,
        FK: Fn(&T) -> K + Sync,
        FV: Fn(&T) -> V + Sync,
    {
        if rows.len() < PARALLEL_ROWS || self.shards.len() == 1 {
            for row in rows {
                self.push(key(row), value(row));
            }
            return;
        }
        let hasher = &self.hasher;
        let hashes: Vec<u64> = rows
            .par_iter()
            .map(|row| hasher.hash_one(key(row)))
            .collect();
        let mut routed: Vec<Vec<u32>> = vec![Vec::new(); self.shards.len()];
        for (index, &hash) in hashes.iter().enumerate() {
            routed[self.shard(hash)].push(index as u32);
        }
        let hasher = &self.hasher;
        self.shards
            .par_iter_mut()
            .zip(routed.into_par_iter())
            .for_each(|(shard, indices)| {
                for index in indices {
                    let row = &rows[index as usize];
                    Self::push_hashed(shard, hasher, hashes[index as usize], key(row), value(row));
                }
            });
    }

    /// The stored key and its bucket, for point lookups returning full rows.
    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&K, &Vec<V>)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let hash = self.hasher.hash_one(key);
        self.shards[self.shard(hash)]
            .find(hash, |(k, _)| k.borrow() == key)
            .map(|(k, values)| (k, values))
    }
}

impl<K: Eq + Hash + Send + Sync, S: BuildHasher + Clone + Sync> ShardedLayout<K, (), S> {
    /// Per-shard realization of DL-ShardedAdmission (the printer's `admit_chunks`):
    /// check each candidate against the committed shard, then insert it into the
    /// NEW shard, one exclusive task per shard.
    ///
    /// `self` is the committed canonical set. Returns the round's NEW version
    /// (fresh, with `self`'s hasher and shard count, so one hash routes a key to
    /// the same shard of both) holding every candidate key absent from `self`,
    /// and those candidates once each. Order: shards ascending, candidate order
    /// within a shard, identical inline and parallel. Fewer than `threshold`
    /// candidates run inline; zero forces the parallel path on nonempty input.
    pub fn admit<T, FK>(&self, chunks: Vec<Vec<T>>, key: FK, threshold: usize) -> (Self, Vec<T>)
    where
        T: Send,
        FK: Fn(&T) -> K + Sync,
    {
        self.admit_batches(Vec::new(), chunks, key, threshold)
    }

    /// [`Self::admit`] for two candidate batches of one head (DL-AdmitOnce):
    /// `covered` chunks come from `in_plan` holes, which already anti-joined
    /// their candidates with this committed layout in their plans, so they are
    /// only inserted-if-absent into NEW; `chunks` are also checked against
    /// `self`. Both batches share one NEW, so duplicates across them are
    /// admitted once. Order: shards ascending, covered before checked
    /// candidates within a shard. With `covered` empty this is exactly `admit`.
    pub fn admit_batches<T, FK>(
        &self,
        covered: Vec<Vec<T>>,
        chunks: Vec<Vec<T>>,
        key: FK,
        threshold: usize,
    ) -> (Self, Vec<T>)
    where
        T: Send,
        FK: Fn(&T) -> K + Sync,
    {
        let mut new = Self {
            shards: self.shards.iter().map(|_| HashTable::new()).collect(),
            hasher: self.hasher.clone(),
        };
        let count = covered.iter().chain(&chunks).map(Vec::len).sum::<usize>();
        if count == 0 {
            return (new, Vec::new());
        }
        // `true` marks chunks still owing the committed anti-join.
        let parts: Vec<(bool, Vec<T>)> = covered
            .into_iter()
            .map(|chunk| (false, chunk))
            .chain(chunks.into_iter().map(|chunk| (true, chunk)))
            .collect();
        let shards = self.shards.len();
        let admitted: Vec<Vec<T>> = if count < threshold
            || shards == 1
            || rayon::current_num_threads() == 1
        {
            let mut admitted: Vec<Vec<T>> = (0..shards).map(|_| Vec::new()).collect();
            for (checked, chunk) in parts {
                for row in chunk {
                    let hash = self.hasher.hash_one(key(&row));
                    let shard = self.shard(hash);
                    let fresh = if checked {
                        Self::admit_hashed(
                            &self.shards[shard],
                            &mut new.shards[shard],
                            &self.hasher,
                            hash,
                            key(&row),
                        )
                    } else {
                        Self::insert_new(&mut new.shards[shard], &self.hasher, hash, key(&row))
                    };
                    if fresh {
                        admitted[shard].push(row);
                    }
                }
            }
            admitted
        } else {
            // Hash and route every chunk in parallel; each row is moved once.
            let routed: Vec<(bool, Vec<Vec<(u64, T)>>)> = parts
                .into_par_iter()
                .map(|(checked, chunk)| {
                    let mut buckets: Vec<Vec<(u64, T)>> = (0..shards).map(|_| Vec::new()).collect();
                    for row in chunk {
                        let hash = self.hasher.hash_one(key(&row));
                        buckets[self.shard(hash)].push((hash, row));
                    }
                    (checked, buckets)
                })
                .collect();
            let mut by_shard: Vec<Vec<(bool, Vec<(u64, T)>)>> = (0..shards)
                .map(|_| Vec::with_capacity(routed.len()))
                .collect();
            for (checked, chunk) in routed {
                for (shard, bucket) in by_shard.iter_mut().zip(chunk) {
                    shard.push((checked, bucket));
                }
            }
            let hasher = &self.hasher;
            new.shards
                .par_iter_mut()
                .zip(self.shards.par_iter())
                .zip(by_shard.into_par_iter())
                .map(|((fresh, committed), mut buckets)| {
                    // Subtract the committed shard first so NEW is sized once
                    // for at most the survivors.
                    for (checked, bucket) in &mut buckets {
                        if *checked {
                            bucket.retain(|(hash, row)| {
                                !Self::committed(committed, *hash, &key(row))
                            });
                        }
                    }
                    fresh.reserve(buckets.iter().map(|(_, bucket)| bucket.len()).sum(), |(k, _)| {
                        hasher.hash_one(k)
                    });
                    let mut admitted = Vec::new();
                    for (hash, row) in buckets.into_iter().flat_map(|(_, bucket)| bucket) {
                        if Self::insert_new(fresh, hasher, hash, key(&row)) {
                            admitted.push(row);
                        }
                    }
                    admitted
                })
                .collect()
        };
        let mut rows = Vec::with_capacity(admitted.iter().map(Vec::len).sum());
        for shard in admitted {
            rows.extend(shard);
        }
        (new, rows)
    }

    /// Membership in a committed shard, as [`ExactMembership::contains`].
    fn committed(shard: &Shard<K, ()>, hash: u64, key: &K) -> bool {
        shard
            .find(hash, |(k, _)| k == key)
            .is_some_and(|(_, values)| !values.is_empty())
    }

    /// Insert-if-absent into a NEW shard; `true` when `key` was absent.
    fn insert_new(shard: &mut Shard<K, ()>, hasher: &S, hash: u64, key: K) -> bool {
        match shard.entry(hash, |(k, _)| *k == key, |(k, _)| hasher.hash_one(k)) {
            Entry::Occupied(_) => false,
            Entry::Vacant(entry) => {
                entry.insert((key, vec![()]));
                true
            }
        }
    }

    fn admit_hashed(
        committed: &Shard<K, ()>,
        new: &mut Shard<K, ()>,
        hasher: &S,
        hash: u64,
        key: K,
    ) -> bool {
        !Self::committed(committed, hash, &key) && Self::insert_new(new, hasher, hash, key)
    }
}

impl<'a, K: Eq + Hash, V, S: BuildHasher> cq_storage::PointLookup for &'a ShardedLayout<K, V, S>
where
    &'a K: Columns,
    &'a V: Columns,
    <&'a K as Columns>::Row: Concat<<&'a V as Columns>::Row>,
{
    type Key = K;
    type Row = <<&'a K as Columns>::Row as Concat<<&'a V as Columns>::Row>>::Row;
    type Cursor = cq_storage::PointRows<&'a K, std::slice::Iter<'a, V>>;
    fn point_lookup(&self, key: &K) -> Self::Cursor {
        let found = self.get_key_value(key);
        cq_storage::PointRows::new(
            found.map(|(k, _)| k),
            found.map_or([].iter(), |(_, values)| values.iter()),
        )
    }
}

impl<'a, K: Eq + Hash, V, S: BuildHasher> IndexRead for &'a ShardedLayout<K, V, S> {
    type Key = K;
    type Item = &'a V;
    type Cursor = std::slice::Iter<'a, V>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        self.get(key).map_or([].iter(), |values| values.iter())
    }
}

impl<K, Q, V, S> ExactMembership<Q> for ShardedLayout<K, V, S>
where
    K: Borrow<Q> + Eq + Hash,
    Q: Eq + Hash + ?Sized,
    S: BuildHasher,
{
    fn contains(&self, key: &Q) -> bool {
        self.get(key).is_some_and(|values| !values.is_empty())
    }
}

/// Distinct payloads of one bucket, deduplicated as they are consumed.
pub struct DistinctPayloads<'a, V> {
    source: std::slice::Iter<'a, V>,
    seen: HashSet<&'a V>,
}

impl<'a, V: Eq + Hash> Iterator for DistinctPayloads<'a, V> {
    type Item = &'a V;
    fn next(&mut self) -> Option<&'a V> {
        let seen = &mut self.seen;
        self.source.find(|value| seen.insert(*value))
    }
}

impl<'a, K: Eq + Hash, V: Eq + Hash, S: BuildHasher> Candidates for &'a ShardedLayout<K, V, S> {
    type Key = K;
    type Item = &'a V;
    type Iter = DistinctPayloads<'a, V>;
    fn count(&self, key: &K) -> usize {
        self.get(key).map_or(0, Vec::len)
    }
    fn keys(&self, key: &K) -> Self::Iter {
        DistinctPayloads {
            source: self.get(key).map_or([].iter(), |values| values.iter()),
            seen: HashSet::new(),
        }
    }
}

impl<K, V, S> Keys for ShardedLayout<K, V, S> {
    type Key = K;
    type Iter<'a>
        = Box<dyn Iterator<Item = &'a K> + 'a>
    where
        Self: 'a;
    fn key_count(&self) -> usize {
        self.len()
    }
    fn distinct_keys(&self) -> Self::Iter<'_> {
        Box::new(
            self.shards
                .iter()
                .flat_map(|shard| shard.iter().map(|(k, _)| k)),
        )
    }
}

impl<'a, K: 'a, V: 'a, S> Rows for &'a ShardedLayout<K, V, S>
where
    &'a K: Columns,
    &'a V: Columns,
    <&'a K as Columns>::Row: Concat<<&'a V as Columns>::Row>,
{
    type Row = <<&'a K as Columns>::Row as Concat<<&'a V as Columns>::Row>>::Row;
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.iter()
            .flat_map(|(k, vs)| vs.iter().map(move |v| k.columns().concat(v.columns())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn rows(n: u32) -> Vec<(u32, u32)> {
        (0..n).map(|i| (i % 97, i)).collect()
    }

    fn reference(rows: &[(u32, u32)]) -> HashMap<(u32,), Vec<(u32,)>> {
        let mut map: HashMap<(u32,), Vec<(u32,)>> = HashMap::new();
        for &(k, v) in rows {
            map.entry((k,)).or_default().push((v,));
        }
        map
    }

    #[test]
    fn parallel_extension_equals_grouped_map_in_bucket_order() {
        for threads in [1, 3, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    for n in [0, 10, PARALLEL_ROWS as u32 * 3 + 7] {
                        let input = rows(n);
                        let expected = reference(&input);
                        let mut layout: ShardedLayout<(u32,), (u32,)> = ShardedLayout::default();
                        layout.extend_from(&input, |r| (r.0,), |r| (r.1,));
                        assert_eq!(layout.len(), expected.len());
                        for (key, values) in &expected {
                            assert_eq!(layout.get(key), Some(values), "bucket order for {key:?}");
                            assert_eq!((&layout).lookup(key).copied().collect::<Vec<_>>(), *values);
                            assert!(layout.contains(key));
                            assert_eq!((&layout).count(key), values.len());
                        }
                        assert!(!layout.contains(&(1000,)));
                        let mut flat: Vec<(u32, u32)> =
                            (&layout).rows().map(|(k, v)| (*k, *v)).collect();
                        flat.sort_unstable();
                        let mut sorted = input.clone();
                        sorted.sort_unstable();
                        assert_eq!(flat, sorted);
                        // Appending again keeps earlier rows first in each bucket.
                        layout.extend_from(&input, |r| (r.0,), |r| (r.1 + 1_000_000,));
                        for (key, values) in &expected {
                            let bucket = layout.get(key).unwrap();
                            assert_eq!(&bucket[..values.len()], &values[..]);
                        }
                        layout.clear();
                        assert!(layout.is_empty());
                    }
                });
        }
    }

    #[test]
    fn admission_subtracts_committed_and_admits_each_fresh_key_once() {
        for threads in [1, 3, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    // Canonical columns (1, 0): the key permutes the row.
                    let mut committed: ShardedLayout<(u32, u32), (), HashState> =
                        ShardedLayout::default();
                    for i in (0..2000).step_by(3) {
                        committed.push((i, i % 7), ());
                    }
                    let chunks: Vec<Vec<(u32, u32)>> = (0..5)
                        .map(|c| (c * 300..c * 300 + 900).map(|i| (i % 7, i)).collect())
                        .collect();
                    let mut expected: Vec<(u32, u32)> = chunks
                        .iter()
                        .flatten()
                        .filter(|r| !committed.contains(&(r.1, r.0)))
                        .copied()
                        .collect();
                    expected.sort_unstable();
                    expected.dedup();
                    let mut orders = Vec::new();
                    for threshold in [0, usize::MAX] {
                        let (new, rows) =
                            committed.admit(chunks.clone(), |r| (r.1, r.0), threshold);
                        let mut sorted = rows.clone();
                        sorted.sort_unstable();
                        assert_eq!(sorted, expected);
                        assert_eq!(new.len(), expected.len());
                        assert!(expected.iter().all(|r| new.contains(&(r.1, r.0))));
                        orders.push(rows);
                    }
                    assert_eq!(orders[0], orders[1], "inline and parallel order");
                });
        }
    }

    /// DL-AdmitOnce law: when the covered batch is already disjoint from the
    /// committed layout (an in-plan anti-join), admitting it without the
    /// committed check together with the checked batch equals `admit` of the
    /// union; duplicates across and within both batches are admitted once.
    #[test]
    fn covered_batches_skip_the_committed_check_and_share_new() {
        for threads in [1, 3, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    let mut committed: ShardedLayout<(u32, u32), (), HashState> =
                        ShardedLayout::default();
                    for i in (0..2000).step_by(3) {
                        committed.push((i, i % 7), ());
                    }
                    let rows = |c: u32| -> Vec<(u32, u32)> {
                        (c * 300..c * 300 + 900).map(|i| (i % 7, i)).collect()
                    };
                    let chunks: Vec<Vec<(u32, u32)>> = (0..3).map(rows).collect();
                    // Covered candidates overlap each other and the checked
                    // batch, but none is committed.
                    let covered: Vec<Vec<(u32, u32)>> = (2..5)
                        .map(|c| {
                            rows(c)
                                .into_iter()
                                .filter(|r| !committed.contains(&(r.1, r.0)))
                                .collect()
                        })
                        .collect();
                    let union: Vec<_> = covered.iter().chain(&chunks).cloned().collect();
                    for threshold in [0, usize::MAX] {
                        let (expected_new, expected) =
                            committed.admit(union.clone(), |r| (r.1, r.0), threshold);
                        let (new, admitted) = committed.admit_batches(
                            covered.clone(),
                            chunks.clone(),
                            |r| (r.1, r.0),
                            threshold,
                        );
                        let mut sorted = admitted.clone();
                        sorted.sort_unstable();
                        let mut expected = expected.clone();
                        expected.sort_unstable();
                        assert_eq!(sorted, expected);
                        assert_eq!(new.len(), expected_new.len());
                        assert!(expected.iter().all(|r| new.contains(&(r.1, r.0))));
                    }
                    // The covered batch is trusted: it is not re-checked.
                    let (_, trusted) =
                        committed.admit_batches(vec![vec![(0, 0)]], Vec::new(), |r| (r.1, r.0), 0);
                    assert_eq!(trusted, [(0, 0)]);
                    let (_, checked) =
                        committed.admit_batches(Vec::new(), vec![vec![(0, 0)]], |r| (r.1, r.0), 0);
                    assert!(checked.is_empty());
                });
        }
    }

    #[test]
    fn candidates_deduplicate_payloads_but_count_occurrences() {
        let mut layout: ShardedLayout<(u32,), (u32,)> = ShardedLayout::default();
        for v in [1, 2, 1, 3, 2] {
            layout.push((0,), (v,));
        }
        assert_eq!((&layout).count(&(0,)), 5);
        assert_eq!(
            (&layout).keys(&(0,)).copied().collect::<Vec<_>>(),
            [(1,), (2,), (3,)]
        );
        assert_eq!(layout.key_count(), 1);
    }
}
