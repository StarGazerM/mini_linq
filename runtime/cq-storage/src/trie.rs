//! A curried index: one level keyed on a prefix, whose items are indexes on
//! the remaining columns, built on first use.
//!
//! `α(trie) = { (k, r) | r ∈ α(inner[k]) }`, a bag; `lookup(k)` yields the
//! inner index for `k` exactly when `k` has rows, so the trie answers a
//! read on its first level by itself and a read on a deeper prefix by
//! applying each level in turn. Inner indexes are built lazily, so a prefix
//! no read ever reaches costs only the grouping of its rows.

use std::{
    cell::{OnceCell, RefCell},
    collections::HashMap,
    hash::Hash,
};

use crate::{Build, ExactMembership, IndexRead};

pub struct Trie<'a, K, Inner> {
    buckets: HashMap<K, Bucket<'a, Inner>>,
}

struct Bucket<'a, Inner> {
    pending: RefCell<Option<Box<dyn FnOnce() -> Inner + 'a>>>,
    built: OnceCell<Inner>,
}

impl<Inner> Bucket<'_, Inner> {
    fn get(&self) -> &Inner {
        self.built.get_or_init(|| {
            let build = self
                .pending
                .borrow_mut()
                .take()
                .expect("a bucket is built exactly once");
            build()
        })
    }
}

impl<'a, K, R, Inner, I> Build<I> for Trie<'a, K, Inner>
where
    K: Eq + Hash,
    R: 'a,
    Inner: Build<std::vec::IntoIter<R>> + 'a,
    I: Iterator<Item = (K, R)>,
{
    fn build(source: I) -> Self {
        let mut groups: HashMap<K, Vec<R>> = HashMap::new();
        for (key, rest) in source {
            groups.entry(key).or_default().push(rest);
        }
        Trie {
            buckets: groups
                .into_iter()
                .map(|(key, rows)| {
                    let pending: Box<dyn FnOnce() -> Inner + 'a> =
                        Box::new(move || Inner::build(rows.into_iter()));
                    (
                        key,
                        Bucket {
                            pending: RefCell::new(Some(pending)),
                            built: OnceCell::new(),
                        },
                    )
                })
                .collect(),
        }
    }
}

impl<'a, 'b, K: Eq + Hash, Inner> IndexRead for &'a Trie<'b, K, Inner> {
    type Key = K;
    type Item = &'a Inner;
    type Cursor = std::option::IntoIter<&'a Inner>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        self.buckets.get(key).map(Bucket::get).into_iter()
    }
}

impl<K: Eq + Hash, Inner> ExactMembership<K> for Trie<'_, K, Inner> {
    fn contains(&self, key: &K) -> bool {
        self.buckets.contains_key(key)
    }
}

impl<'a, K: Eq + Hash, Inner: crate::Keys> crate::Candidates for &'a Trie<'_, K, Inner> {
    type Key = K;
    type Item = &'a Inner::Key;
    type Iter = std::iter::FlatMap<
        std::option::IntoIter<&'a Inner>,
        Inner::Iter<'a>,
        fn(&'a Inner) -> Inner::Iter<'a>,
    >;
    fn count(&self, key: &K) -> usize {
        self.buckets
            .get(key)
            .map_or(0, |bucket| bucket.get().key_count())
    }
    fn keys(&self, key: &K) -> Self::Iter {
        self.buckets
            .get(key)
            .map(Bucket::get)
            .into_iter()
            .flat_map(Inner::distinct_keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HashIndex;
    use std::collections::HashSet;

    #[test]
    fn levels_apply_in_turn_and_inner_indexes_build_once_on_first_touch() {
        let rows = [
            ((1,), ((10,), (100,))),
            ((1,), ((11,), (101,))),
            ((2,), ((20,), (200,))),
        ];
        let trie: Trie<'_, (i32,), HashIndex<(i32,), (i32,)>> = Build::build(rows.into_iter());
        let inner: Vec<_> = (&trie).lookup(&(1,)).collect();
        assert_eq!(inner.len(), 1);
        let payload: Vec<_> = inner[0].lookup(&(11,)).collect();
        assert_eq!(payload, [(101,)]);
        assert!(trie.contains(&(2,)));
        assert!(!trie.contains(&(3,)));
        assert_eq!((&trie).lookup(&(3,)).count(), 0);
        // A key set as the innermost level answers membership only.
        let sets: Trie<'_, (i32,), HashSet<(i32,)>> =
            Build::build([((1,), ((10,), ())), ((1,), ((11,), ()))].into_iter());
        let level: Vec<_> = (&sets).lookup(&(1,)).collect();
        assert!(level[0].contains(&(11,)) && !level[0].contains(&(12,)));
    }
}
