//! Deferred hash buckets with independently owned lookup cursors.
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, HashSet},
    hash::{BuildHasher, Hash},
    rc::Rc,
};

use crate::ExactMembership;
use crate::instrument::RetainedVec;

/// Exact lookup preserves stored occurrences. A cursor is independent of the
/// temporary lookup handle: HashIndex cursors copy the first item and retain an owned tail reference,
/// while host and trie cursors borrow their backing value for its lifetime.
#[diagnostic::on_unimplemented(
    message = "this direct read requires IndexRead at the leading declared key type"
)]
pub trait IndexRead: Clone {
    type Key;
    type Item: Copy;
    type Cursor: Iterator<Item = Self::Item>;
    fn lookup(&self, key: &Self::Key) -> Self::Cursor;
    /// Stable probe realization with at most N rows of lookahead. N=1 preserves
    /// scalar demand; larger batches require a total source/key for consumers
    /// that may stop early. Outer and within-bucket occurrence order are exact.
    fn probe<const N: usize, I, F>(self, source: I, mut key: F) -> impl Iterator<Item = (I::Item, Self::Item)>
    where
        I: Iterator,
        I::Item: Copy,
        F: FnMut(&I::Item) -> Self::Key,
    {
        source.flat_map(move |row| self.lookup(&key(&row)).map(move |value| (row, value)))
    }
}

impl<T: IndexRead> IndexRead for &T {
    type Key = T::Key;
    type Item = T::Item;
    type Cursor = T::Cursor;

    fn lookup(&self, key: &Self::Key) -> Self::Cursor {
        T::lookup(*self, key)
    }
    fn probe<const N: usize, I, F>(self, source: I, key: F) -> impl Iterator<Item = (I::Item, Self::Item)>
    where
        I: Iterator,
        I::Item: Copy,
        F: FnMut(&I::Item) -> Self::Key,
    {
        T::probe::<N, _, _>(self.clone(), source, key)
    }
}

/// The one construction protocol for compiled structures: a value built from
/// `(key, item)` pairs, with `α(build(I)) = bag(I)` projected to whatever the
/// structure represents (a key set keeps only the keys). Any type with this
/// impl and the access traits it claims is a usable index structure; the
/// compiler names it, never inspects it.
pub trait Build<I: Iterator>: Sized {
    fn build(source: I) -> Self;
}

impl<K: Eq + Hash, V, S: Default + std::hash::BuildHasher, I: Iterator<Item = (K, V)>> Build<I>
    for HashSet<K, S>
{
    fn build(source: I) -> Self {
        crate::count!(index_builds);
        source
            .map(|(key, _)| {
                crate::count!(distinct_insertions);
                key
            })
            .collect()
    }
}

impl<K: Ord, V, I: Iterator<Item = (K, V)>> Build<I> for BTreeMap<K, Vec<V>> {
    fn build(source: I) -> Self {
        crate::count!(index_builds);
        let mut map = BTreeMap::new();
        for (key, value) in source {
            crate::count!(rows_copied);
            map.entry(key).or_insert_with(Vec::new).push(value);
        }
        map
    }
}

pub struct HashIndex<'a, K, V> {
    state: Rc<State<'a, K, V>>,
}
type Source<'a, K, V> = Box<dyn Iterator<Item = (K, V)> + 'a>;

struct State<'a, K, V> {
    /// Taken exactly once, by the initialization of `buckets`.
    #[cfg(feature = "counters")]
    allocation: RefCell<crate::counters_memory::Allocation>,
    source: RefCell<Option<Source<'a, K, V>>>,
    buckets: OnceCell<Buckets<K, V>>,
}
// Keys retain compact table slots; payloads occupy a contiguous arena. A
// singleton needs no allocation or reference-counted handle of its own.
struct Buckets<K, V> {
    index: Directory<K>,
    values: Vec<Bucket<V>>,
}
struct Bucket<V> {
    first: V,
    tail: Option<Rc<RetainedVec<V>>>,
}

// Swiss-table control groups retain fast scalar misses. Batched access hashes
// first, visits control groups to prefetch candidate buckets, then resolves with
// full equality. A prefetch is only a cache hint, never a membership decision.
struct Directory<K> {
    table: hashbrown::HashTable<(K, usize)>,
    hasher: crate::HashState,
}
impl<K: Eq + Hash> Directory<K> {
    fn new() -> Self {
        Self { table: hashbrown::HashTable::new(), hasher: crate::HashState::default() }
    }
    fn entry(&mut self, key: K) -> (usize, bool) {
        let hash = self.hash(&key);
        let slot = self.table.len();
        let hasher = &self.hasher;
        match self.table.entry(hash, |(stored, _)| stored == &key, |(stored, _)| Self::hash_with(hasher, stored)) {
            hashbrown::hash_table::Entry::Occupied(entry) => (entry.get().1, false),
            hashbrown::hash_table::Entry::Vacant(entry) => {
                entry.insert((key, slot));
                (slot, true)
            }
        }
    }
    #[inline]
    fn hash_with(hasher: &crate::HashState, key: &K) -> u64 {
        crate::count!(hash_computations);
        hasher.hash_one(key)
    }
    #[inline]
    fn hash(&self, key: &K) -> u64 {
        Self::hash_with(&self.hasher, key)
    }
    #[inline]
    fn get_hashed(&self, key: &K, hash: u64) -> Option<usize> {
        self.table.find(hash, |(stored, _)| stored == key).map(|entry| entry.1)
    }
    #[inline]
    fn get(&self, key: &K) -> Option<usize> {
        if self.table.is_empty() {
            None
        } else {
            self.get_hashed(key, self.hash(key))
        }
    }
    #[inline]
    fn prefetch(&self, hash: u64) {
        let Some(candidate) = self.table.iter_hash(hash).next() else { return };
        let address = candidate as *const (K, usize);
        // SAFETY: this immutable candidate reference names an allocated bucket;
        // prefetch neither dereferences it nor changes memory.
        #[cfg(target_arch = "aarch64")]
        unsafe {
            std::arch::asm!("prfm pldl1keep, [{address}]", address = in(reg) address, options(readonly, nostack, preserves_flags));
        }
        #[cfg(target_arch = "x86_64")]
        unsafe {
            std::arch::x86_64::_mm_prefetch(address.cast(), std::arch::x86_64::_MM_HINT_T0);
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        let _ = address;
    }
}
impl<K, V> Clone for HashIndex<'_, K, V> {
    fn clone(&self) -> Self {
        Self {
            state: Rc::clone(&self.state),
        }
    }
}
impl<'a, K: 'a, V: Copy + 'a, I: Iterator<Item = (K, V)> + 'a> Build<I> for HashIndex<'a, K, V> {
    fn build(source: I) -> Self {
        Self {
            state: Rc::new(State {
                #[cfg(feature = "counters")]
                allocation: RefCell::new(crate::counters_memory::Allocation::default()),
                source: RefCell::new(Some(Box::new(source))),
                buckets: OnceCell::new(),
            }),
        }
    }
}
impl<K: Eq + Hash, V: Copy> IndexRead for HashIndex<'_, K, V> {
    type Key = K;
    type Item = V;
    type Cursor = BucketCursor<V>;
    fn lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        let buckets = self.buckets();
        let bucket = buckets.index.get(key).map(|slot| &buckets.values[slot]);
        BucketCursor {
            first: bucket.map(|bucket| bucket.first),
            tail: bucket.and_then(|bucket| bucket.tail.clone()),
            position: 0,
        }
    }
    fn probe<const N: usize, I, F>(self, source: I, key: F) -> impl Iterator<Item = (I::Item, V)>
    where
        I: Iterator,
        I::Item: Copy,
        F: FnMut(&I::Item) -> K,
    {
        const { assert!(N > 0, "probe batch must be nonempty"); }
        BatchedProbe {
            index: self,
            source,
            key,
            rows: std::array::from_fn(|_| None),
            slots: [None; N],
            len: 0,
            row: 0,
            value: 0,
            finished: false,
        }
    }
}

impl<K: Eq + Hash, V> HashIndex<'_, K, V> {
    /// Borrow distinct stored keys without copying them or cloning index handles.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        crate::instrument::cursor!(self.buckets().index.table.iter().map(|(key, _)| key))
    }
    fn buckets(&self) -> &Buckets<K, V> {
        self.state.buckets.get_or_init(|| {
            crate::count!(index_builds);
            let source = self
                .state
                .source
                .borrow_mut()
                .take()
                .expect("the source is consumed only by this initialization");
            let mut index = Directory::new();
            let mut values: Vec<Bucket<V>> = Vec::new();
            for (key, value) in source {
                crate::count!(rows_copied);
                let (slot, fresh) = index.entry(key);
                if fresh {
                    values.push(Bucket { first: value, tail: None });
                } else {
                    let tail = values[slot].tail.get_or_insert_with(Default::default);
                    // No cursor can see these buckets until initialization finishes.
                    Rc::get_mut(tail)
                        .expect("unpublished bucket is uniquely owned")
                        .push(value);
                }
            }
            #[cfg(feature = "counters")]
            {
                for bucket in &mut values {
                    if let Some(tail) = &mut bucket.tail {
                        Rc::get_mut(tail).expect("unpublished tail").account();
                    }
                }
                *self.state.allocation.borrow_mut() = crate::counters_memory::Allocation::new(
                    index.table.allocation_size()
                        + values.capacity() * std::mem::size_of::<Bucket<V>>(),
                );
            }
            Buckets { index, values }
        })
    }
}

impl<K: Eq + Hash, V> crate::Keys for HashIndex<'_, K, V> {
    type Key = K;
    type Iter<'a>
        = crate::instrument::Cursor<std::iter::Map<hashbrown::hash_table::Iter<'a, (K, usize)>, fn(&'a (K, usize)) -> &'a K>>
    where
        Self: 'a;
    fn key_count(&self) -> usize {
        self.buckets().index.table.len()
    }
    fn distinct_keys(&self) -> Self::Iter<'_> {
        crate::instrument::cursor!(self.buckets().index.table.iter().map((|(key, _)| key) as fn(&(K, usize)) -> &K))
    }
}

impl<K: Eq + Hash, V: Copy + Eq + Hash> crate::Candidates for HashIndex<'_, K, V> {
    type Key = K;
    type Item = V;
    type Iter = crate::candidates::Distinct<BucketCursor<V>>;
    fn count(&self, key: &K) -> usize {
        crate::count!(lookups);
        {
            let buckets = self.buckets();
            buckets.index.get(key).map_or(0, |slot| {
                1 + buckets.values[slot]
                    .tail
                    .as_ref()
                    .map_or(0, |tail| tail.len())
            })
        }
    }
    fn keys(&self, key: &K) -> Self::Iter {
        crate::candidates::Distinct::new(self.lookup(key))
    }
}

pub struct BucketCursor<V> {
    first: Option<V>,
    tail: Option<Rc<RetainedVec<V>>>,
    position: usize,
}
// A cursor exposes copied items, never a pinned projection into its first item.
// Retain the unconditional Unpin contract of the previous Rc-backed cursor.
impl<V> Unpin for BucketCursor<V> {}
impl<V: Copy> Iterator for BucketCursor<V> {
    type Item = V;
    fn next(&mut self) -> Option<V> {
        crate::count!(cursor_steps);
        if let Some(item) = self.first.take() {
            crate::count!(rows_enumerated);
            return Some(item);
        }
        let item = self.tail.as_ref()?.get(self.position).copied();
        if item.is_some() {
            crate::count!(rows_enumerated);
            self.position += 1;
        }
        item
    }
}
impl<V: Copy> std::iter::FusedIterator for BucketCursor<V> {}

/// A hash index answers membership by the nonemptiness of its bucket.
impl<K: Eq + Hash, V: Copy> ExactMembership<K> for HashIndex<'_, K, V> {
    fn contains(&self, key: &K) -> bool {
        crate::count!(lookups);
        self.buckets().index.get(key).is_some()
    }
}

struct BatchedProbe<'a, K, V, I: Iterator, F, const N: usize> {
    index: HashIndex<'a, K, V>,
    source: I,
    key: F,
    rows: [Option<I::Item>; N],
    slots: [Option<usize>; N],
    len: usize,
    row: usize,
    value: usize,
    finished: bool,
}
impl<K: Eq + Hash, V: Copy, I: Iterator, F, const N: usize> Iterator for BatchedProbe<'_, K, V, I, F, N>
where
    I::Item: Copy,
    F: FnMut(&I::Item) -> K,
{
    type Item = (I::Item, V);
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.row == self.len {
                if self.finished {
                    return None;
                }
                self.len = 0;
                let mut keys = std::array::from_fn::<_, N, _>(|_| None);
                let mut hashes = [0; N];
                for key in &mut keys {
                    let Some(row) = self.source.next() else {
                        self.finished = true;
                        break;
                    };
                    *key = Some((self.key)(&row));
                    self.rows[self.len] = Some(row);
                    self.len += 1;
                }
                if self.len == 0 {
                    return None;
                }
                // Retain lazy-build timing: an empty outer stream never builds.
                let buckets = self.index.buckets();
                if N == 1 {
                    crate::count!(lookups);
                    self.slots[0] = buckets.index.get(keys[0].as_ref().unwrap());
                } else if buckets.index.table.is_empty() {
                    for slot in &mut self.slots[..self.len] {
                        crate::count!(lookups);
                        *slot = None;
                    }
                } else {
                    for i in 0..self.len {
                        hashes[i] = buckets.index.hash(keys[i].as_ref().unwrap());
                    }
                    for &hash in &hashes[..self.len] {
                        buckets.index.prefetch(hash);
                    }
                    for i in 0..self.len {
                        crate::count!(lookups);
                        self.slots[i] = buckets.index.get_hashed(keys[i].as_ref().unwrap(), hashes[i]);
                    }
                }
                self.row = 0;
                self.value = 0;
            }
            let Some(slot) = self.slots[self.row] else {
                self.row += 1;
                continue;
            };
            let bucket = &self.index.buckets().values[slot];
            crate::count!(cursor_steps);
            let value = if self.value == 0 {
                Some(bucket.first)
            } else {
                bucket.tail.as_ref().and_then(|tail| tail.get(self.value - 1)).copied()
            };
            if let Some(value) = value {
                self.value += 1;
                crate::count!(rows_enumerated);
                return Some((self.rows[self.row].unwrap(), value));
            }
            self.row += 1;
            self.value = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    const PROBE_BATCH: usize = 128;
    #[test]
    fn batched_probe_preserves_outer_and_multi_match_order() {
        let index = HashIndex::build([(1, 10), (1, 10), (2, 20), (1, 11)].into_iter());
        let outer: Vec<_> = (0..(PROBE_BATCH * 3 + 7)).map(|i| (i, i % 4)).collect();
        let scalar: Vec<_> = outer.iter().copied()
            .flat_map(|row| index.lookup(&row.1).map(move |value| (row, value)))
            .collect();
        let batched: Vec<_> = (&index).probe::<PROBE_BATCH, _, _>(outer.iter().copied(), |row| row.1).collect();
        assert_eq!(batched, scalar);
        assert_eq!((&index).probe::<1, _, _>(outer.iter().copied(), |row| row.1).collect::<Vec<_>>(), scalar);
        let borrowed = &index;
        assert_eq!(borrowed.probe::<PROBE_BATCH, _, _>(outer.into_iter(), |row| row.1).collect::<Vec<_>>(), scalar);
    }

    #[test]
    fn batched_probe_retains_one_index_handle_and_no_bucket_handles() {
        let index = HashIndex::build([(1, 10), (1, 11)].into_iter());
        let tail = index.buckets().values[0].tail.as_ref().unwrap();
        let mut probe = (&index).probe::<PROBE_BATCH, _, _>(std::iter::repeat_n(1, PROBE_BATCH + 1), |key| *key);
        assert_eq!(Rc::strong_count(&index.state), 2);
        assert_eq!(Rc::strong_count(tail), 1);
        for _ in 0..PROBE_BATCH + 1 {
            assert_eq!(probe.next(), Some((1, 10)));
            assert_eq!(probe.next(), Some((1, 11)));
            assert_eq!(Rc::strong_count(tail), 1);
            assert_eq!(Rc::strong_count(&index.state), 2);
        }
        assert_eq!(probe.next(), None);
        assert_eq!(probe.next(), None);
        drop(probe);
        assert_eq!(Rc::strong_count(&index.state), 1);
    }

    #[test]
    fn scalar_streaming_probe_does_not_pull_a_later_partial_row() {
        let index = HashIndex::build([(1, 10), (1, 11)].into_iter());
        let source = std::iter::once(1).chain(std::iter::once_with(|| panic!("unvisited row")));
        let mut probe = (&index).probe::<1, _, _>(source, |key| *key);
        assert_eq!(probe.next(), Some((1, 10)));
        assert_eq!(probe.next(), Some((1, 11)));
    }

    #[test]
    fn empty_probe_does_not_initialize_deferred_index() {
        let index = HashIndex::build(std::iter::once_with(|| -> (i32, i32) {
            panic!("empty outer must not build the index")
        }));
        assert_eq!(index.probe::<PROBE_BATCH, _, _>(std::iter::empty::<i32>(), |row| *row).next(), None);
    }

    #[test]
    fn empty_index_handles_multiple_missing_batches_without_a_directory_allocation() {
        let index = HashIndex::build(std::iter::empty::<(i32, i32)>());
        assert_eq!(index.lookup(&1).next(), None);
        assert!(!index.contains(&1));
        assert_eq!((&index).probe::<PROBE_BATCH, _, _>(0..300, |key| *key).count(), 0);
        assert_eq!(index.buckets().index.table.allocation_size(), 0);
    }

    #[test]
    fn colliding_keys_still_require_exact_equality() {
        #[derive(PartialEq, Eq)]
        struct Collision(i32);
        impl Hash for Collision {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                0.hash(state);
            }
        }
        let index = HashIndex::build((0..200).map(|i| (Collision(i), i)));
        let scalar: Vec<_> = (0..300).flat_map(|i| index.lookup(&Collision(i)).map(move |v| (i, v))).collect();
        assert_eq!((&index).probe::<PROBE_BATCH, _, _>(0..300, |i| Collision(*i)).collect::<Vec<_>>(), scalar);
        assert!(!index.contains(&Collision(400)));
        assert_eq!(index.keys().count(), 200);
    }
    use super::*;
    use std::cell::Cell;

    #[test]
    fn cursors_are_independent_fused_and_unconditionally_unpin() {
        fn unpin<T: Unpin>() {}
        unpin::<BucketCursor<std::marker::PhantomPinned>>();
        let index = HashIndex::build([(1, 10), (1, 10), (2, 20), (1, 11)].into_iter());
        let mut first = index.lookup(&1);
        let second = index.lookup(&1);
        let singleton = index.lookup(&2);
        let mut missing = index.lookup(&3);
        assert_eq!(crate::Candidates::count(&index, &1), 3);
        assert_eq!(crate::Candidates::count(&index, &2), 1);
        assert_eq!(crate::Candidates::count(&index, &3), 0);
        assert_eq!(crate::Keys::key_count(&index), 2);
        assert_eq!(first.next(), Some(10));
        drop(index);
        assert_eq!(second.collect::<Vec<_>>(), [10, 10, 11]);
        assert_eq!(singleton.collect::<Vec<_>>(), [20]);
        assert_eq!(first.by_ref().collect::<Vec<_>>(), [10, 11]);
        assert_eq!(first.next(), None);
        assert_eq!(first.next(), None);
        assert_eq!(missing.next(), None);
        assert_eq!(missing.next(), None);
    }
    #[test]
    fn deferred_build_is_shared_and_cursors_own_buckets() {
        let polls = Cell::new(0);
        let values = [String::from("a"), String::from("b"), String::from("c")];
        let source = [(1, &values[0]), (1, &values[1]), (2, &values[2])]
            .into_iter()
            .inspect(|_| polls.set(polls.get() + 1));
        let index = HashIndex::build(source);
        let shared = index.clone();
        assert_eq!(polls.get(), 0);
        let cursor = index.lookup(&1);
        let singleton = shared.lookup(&2);
        assert_eq!(polls.get(), 3);
        assert_eq!(shared.lookup(&2).collect::<Vec<_>>(), vec![&values[2]]);
        assert_eq!(shared.lookup(&9).next(), None);
        assert_eq!(polls.get(), 3);
        drop(index);
        drop(shared);
        let unique = singleton.collect::<Vec<_>>();
        assert!(std::ptr::eq(unique[0], &values[2]));
        let rows = cursor.collect::<Vec<_>>();
        assert!(std::ptr::eq(rows[0], &values[0]));
        assert!(std::ptr::eq(rows[1], &values[1]));
    }
}
