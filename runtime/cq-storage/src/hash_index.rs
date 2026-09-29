//! Deferred hash buckets with independently owned lookup cursors.
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, HashMap, HashSet},
    hash::Hash,
    rc::Rc,
};

use crate::ExactMembership;

/// Exact lookup preserves every stored occurrence. The cursor owns its bucket
/// and remains valid after the index handle is dropped.
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
        source.map(|(key, _)| key).collect()
    }
}

impl<K: Ord, V, I: Iterator<Item = (K, V)>> Build<I> for BTreeMap<K, Vec<V>> {
    fn build(source: I) -> Self {
        let mut map = BTreeMap::new();
        for (key, value) in source {
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
    source: RefCell<Option<Source<'a, K, V>>>,
    buckets: OnceCell<HashMap<K, Rc<Vec<V>>>>,
}
impl<K, V> Clone for HashIndex<'_, K, V> {
    fn clone(&self) -> Self {
        Self {
            state: Rc::clone(&self.state),
        }
    }
}
impl<'a, K: Eq + Hash + 'a, V: Copy + 'a, I: Iterator<Item = (K, V)> + 'a> Build<I>
    for HashIndex<'a, K, V>
{
    fn build(source: I) -> Self {
        Self {
            state: Rc::new(State {
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
        let buckets = self.buckets();
        BucketCursor {
            bucket: buckets.get(key).cloned(),
            position: 0,
        }
    }
}

impl<K: Eq + Hash, V> HashIndex<'_, K, V> {
    fn buckets(&self) -> &HashMap<K, Rc<Vec<V>>> {
        self.state.buckets.get_or_init(|| {
            let source = self
                .state
                .source
                .borrow_mut()
                .take()
                .expect("the source is consumed only by this initialization");
            let mut buckets = HashMap::<K, Vec<V>>::new();
            for (key, value) in source {
                buckets.entry(key).or_default().push(value);
            }
            buckets
                .into_iter()
                .map(|(key, values)| (key, Rc::new(values)))
                .collect()
        })
    }
}

impl<K: Eq + Hash, V> crate::Keys for HashIndex<'_, K, V> {
    type Key = K;
    type Iter<'a>
        = std::collections::hash_map::Keys<'a, K, Rc<Vec<V>>>
    where
        Self: 'a;
    fn key_count(&self) -> usize {
        self.buckets().len()
    }
    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.buckets().keys()
    }
}

impl<K: Eq + Hash, V: Copy + Eq + Hash> crate::Candidates for &HashIndex<'_, K, V> {
    type Key = K;
    type Item = V;
    type Iter = crate::candidates::Distinct<BucketCursor<V>>;
    fn count(&self, key: &K) -> usize {
        self.buckets().get(key).map_or(0, |v| v.len())
    }
    fn keys(&self, key: &K) -> Self::Iter {
        crate::candidates::Distinct::new(self.lookup(key))
    }
}

pub struct BucketCursor<V> {
    bucket: Option<Rc<Vec<V>>>,
    position: usize,
}
impl<V: Copy> Iterator for BucketCursor<V> {
    type Item = V;
    fn next(&mut self) -> Option<V> {
        let item = self.bucket.as_ref()?.get(self.position).copied();
        if item.is_some() {
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
        assert_eq!(polls.get(), 3);
        assert_eq!(shared.lookup(&2).collect::<Vec<_>>(), vec![&values[2]]);
        assert_eq!(shared.lookup(&9).next(), None);
        assert_eq!(polls.get(), 3);
        drop(index);
        drop(shared);
        let rows = cursor.collect::<Vec<_>>();
        assert!(std::ptr::eq(rows[0], &values[0]));
        assert!(std::ptr::eq(rows[1], &values[1]));
    }
}
