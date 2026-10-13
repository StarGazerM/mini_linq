//! Deferred hash buckets with independently owned lookup cursors.
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, HashSet},
    hash::Hash,
    rc::Rc,
};

use crate::ExactMembership;
use crate::instrument::{HashMap, RetainedVec};

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
}

impl<T: IndexRead> IndexRead for &T {
    type Key = T::Key;
    type Item = T::Item;
    type Cursor = T::Cursor;

    fn lookup(&self, key: &Self::Key) -> Self::Cursor {
        T::lookup(*self, key)
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
    index: HashMap<K, usize>,
    values: Vec<Bucket<V>>,
}
struct Bucket<V> {
    first: V,
    tail: Option<Rc<RetainedVec<V>>>,
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
        let bucket = buckets.index.get(key).map(|&slot| &buckets.values[slot]);
        BucketCursor {
            first: bucket.map(|bucket| bucket.first),
            tail: bucket.and_then(|bucket| bucket.tail.clone()),
            position: 0,
        }
    }
}

impl<K: Eq + Hash, V> HashIndex<'_, K, V> {
    fn buckets(&self) -> &Buckets<K, V> {
        self.state.buckets.get_or_init(|| {
            crate::count!(index_builds);
            let source = self
                .state
                .source
                .borrow_mut()
                .take()
                .expect("the source is consumed only by this initialization");
            let mut index = HashMap::default();
            let mut values: Vec<Bucket<V>> = Vec::new();
            for (key, value) in source {
                crate::count!(rows_copied);
                match index.entry(key) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(values.len());
                        values.push(Bucket {
                            first: value,
                            tail: None,
                        });
                    }
                    std::collections::hash_map::Entry::Occupied(entry) => {
                        let tail = values[*entry.get()]
                            .tail
                            .get_or_insert_with(Default::default);
                        // No cursor can see these buckets until initialization finishes.
                        Rc::get_mut(tail)
                            .expect("unpublished bucket is uniquely owned")
                            .push(value);
                    }
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
                    index.capacity()
                        * (std::mem::size_of::<K>() + std::mem::size_of::<usize>() + 1)
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
        = crate::instrument::Cursor<std::collections::hash_map::Keys<'a, K, usize>>
    where
        Self: 'a;
    fn key_count(&self) -> usize {
        self.buckets().index.len()
    }
    fn distinct_keys(&self) -> Self::Iter<'_> {
        crate::instrument::cursor!(self.buckets().index.keys())
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
            buckets.index.get(key).map_or(0, |&slot| {
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
        self.lookup(key).next().is_some()
    }
}

#[cfg(test)]
mod tests {
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
