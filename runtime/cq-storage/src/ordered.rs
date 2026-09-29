//! Key-ordered cursors with lower-bound seeking, and their co-iteration.
//!
//! `α(ordered) = bag of (key, item)`; [`OrderedRead::ordered`] enumerates it
//! in nondecreasing key order, and [`OrderedCursor::seek`] discards exactly
//! the remaining pairs whose key is below the bound. [`merge`] enumerates
//! `{ (k, a, b) | (k, a) ∈ α(l), (k, b) ∈ α(r) }` with multiplicity; it is the
//! one co-iteration primitive, and it uses `seek` to skip runs of unmatched
//! keys instead of stepping through them.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

/// A cursor over `(key, item)` pairs in nondecreasing key order.
pub trait OrderedCursor: Iterator<Item = (Self::Key, Self::Value)> {
    type Key: Ord + Copy;
    type Value: Copy;

    /// Advance so that the next pair, if any, has `key >= bound`. Pairs
    /// already yielded are unaffected; pairs below the bound are never yielded.
    fn seek(&mut self, bound: &Self::Key);
}

/// A representation whose pairs can be read in key order from any bound.
///
/// Implementations are on borrowed collections so a cursor lives as long as
/// the borrow it was created from, not as long as the handle it was called on.
#[diagnostic::on_unimplemented(
    message = "this read requires ordered inputs",
    note = "join = merge on a leading host key requires OrderedRead; materialize a let to build an ordered index"
)]
pub trait OrderedRead {
    type Cursor<'a>: OrderedCursor
    where
        Self: 'a;

    fn ordered(&self) -> Self::Cursor<'_>;
}

impl<T: OrderedRead + ?Sized> OrderedRead for &T {
    type Cursor<'a>
        = T::Cursor<'a>
    where
        Self: 'a;
    fn ordered(&self) -> Self::Cursor<'_> {
        T::ordered(*self)
    }
}

/// Ordered cursor over `BTreeMap<K, Vec<V>>`: every value under a key shares it.
pub struct GroupedCursor<'a, K, V> {
    map: &'a BTreeMap<K, Vec<V>>,
    keys: std::collections::btree_map::Range<'a, K, Vec<V>>,
    current: Option<(&'a K, std::slice::Iter<'a, V>)>,
}

impl<'a, K: Ord, V> Iterator for GroupedCursor<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((key, values)) = &mut self.current {
                if let Some(value) = values.next() {
                    return Some((key, value));
                }
                self.current = None;
            }
            let (key, values) = self.keys.next()?;
            self.current = Some((key, values.iter()));
        }
    }
}

impl<'a, K: Ord, V> OrderedCursor for GroupedCursor<'a, K, V> {
    type Key = &'a K;
    type Value = &'a V;

    fn seek(&mut self, bound: &&'a K) {
        if self.current.as_ref().is_some_and(|(key, _)| *key >= *bound) {
            return;
        }
        self.current = None;
        self.keys = self.map.range((Bound::Included(*bound), Bound::Unbounded));
    }
}

impl<'a, K: Ord, V> OrderedRead for &'a BTreeMap<K, Vec<V>> {
    type Cursor<'b>
        = GroupedCursor<'a, K, V>
    where
        Self: 'b;

    fn ordered(&self) -> Self::Cursor<'_> {
        GroupedCursor {
            map: self,
            keys: self.range::<K, _>(..),
            current: None,
        }
    }
}

/// Ordered cursor over `BTreeSet<K>`: keys only, with the unit item.
pub struct KeyCursor<'a, K> {
    set: &'a BTreeSet<K>,
    keys: std::collections::btree_set::Range<'a, K>,
}

impl<'a, K: Ord> Iterator for KeyCursor<'a, K> {
    type Item = (&'a K, ());

    fn next(&mut self) -> Option<Self::Item> {
        self.keys.next().map(|key| (key, ()))
    }
}

impl<'a, K: Ord> OrderedCursor for KeyCursor<'a, K> {
    type Key = &'a K;
    type Value = ();

    fn seek(&mut self, bound: &&'a K) {
        self.keys = self.set.range((Bound::Included(*bound), Bound::Unbounded));
    }
}

impl<'a, K: Ord> OrderedRead for &'a BTreeSet<K> {
    type Cursor<'b>
        = KeyCursor<'a, K>
    where
        Self: 'b;

    fn ordered(&self) -> Self::Cursor<'_> {
        KeyCursor {
            set: self,
            keys: self.range::<K, _>(..),
        }
    }
}

/// A key-ordered iterator of `(key, item)` pairs presented as a cursor.
///
/// The order is asserted by the caller (the compiler's ordering guarantee of the iterator),
/// never checked: `seek` skips pairs while their key is below the bound,
/// which is exact precisely when the input is nondecreasing.
pub fn sorted_by<I>(pairs: I) -> Sorted<I::IntoIter>
where
    I: IntoIterator,
    I::IntoIter: Iterator,
{
    Sorted {
        pairs: pairs.into_iter().peekable(),
    }
}

pub struct Sorted<I: Iterator> {
    pairs: std::iter::Peekable<I>,
}

impl<I, K, V> Iterator for Sorted<I>
where
    I: Iterator<Item = (K, V)>,
{
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        self.pairs.next()
    }
}

impl<I, K: Ord + Copy, V: Copy> OrderedCursor for Sorted<I>
where
    I: Iterator<Item = (K, V)>,
{
    type Key = K;
    type Value = V;

    fn seek(&mut self, bound: &K) {
        while self.pairs.peek().is_some_and(|(key, _)| key < bound) {
            self.pairs.next();
        }
    }
}

/// One-slot lookahead that keeps `seek` exact: a peeked pair at or beyond
/// the bound is retained, one below it is discarded with the rest.
struct Peeked<C: OrderedCursor> {
    cursor: C,
    slot: Option<(C::Key, C::Value)>,
}

impl<C: OrderedCursor> Peeked<C> {
    fn peek(&mut self) -> Option<(C::Key, C::Value)> {
        if self.slot.is_none() {
            self.slot = self.cursor.next();
        }
        self.slot
    }

    fn take(&mut self) -> Option<(C::Key, C::Value)> {
        self.peek();
        self.slot.take()
    }

    fn seek(&mut self, bound: &C::Key) {
        if self.slot.is_some_and(|(key, _)| key >= *bound) {
            return;
        }
        self.slot = None;
        self.cursor.seek(bound);
    }
}

/// Co-iterate two ordered cursors on their shared key.
///
/// Yields `(key, left item, right item)` for every pair of left and right
/// entries with equal keys, in key order, left group outer. When keys
/// differ, the lagging cursor seeks to the leading key (leapfrog).
pub fn merge<L, R>(left: L, right: R) -> Merge<L, R>
where
    L: OrderedCursor,
    R: OrderedCursor<Key = L::Key>,
{
    Merge {
        left: Peeked {
            cursor: left,
            slot: None,
        },
        right: Peeked {
            cursor: right,
            slot: None,
        },
        pending: None,
        group: Vec::new(),
        position: 0,
    }
}

pub struct Merge<L: OrderedCursor, R: OrderedCursor> {
    left: Peeked<L>,
    right: Peeked<R>,
    /// The left pair currently being paired with the buffered right group.
    pending: Option<(L::Key, L::Value)>,
    /// Right items sharing the pending key, retained for the whole left run.
    group: Vec<R::Value>,
    position: usize,
}

impl<L, R> Iterator for Merge<L, R>
where
    L: OrderedCursor,
    R: OrderedCursor<Key = L::Key>,
{
    type Item = (L::Key, L::Value, R::Value);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((key, item)) = self.pending {
                if self.position < self.group.len() {
                    let matched = self.group[self.position];
                    self.position += 1;
                    return Some((key, item, matched));
                }
                self.pending = None;
                self.position = 0;
                // Continue the left run on the same key against the same group.
                if self.left.peek().is_some_and(|(next, _)| next == key) {
                    self.pending = self.left.take();
                    continue;
                }
                self.group.clear();
            }
            let (left_key, _) = self.left.peek()?;
            let (right_key, _) = self.right.peek()?;
            if left_key < right_key {
                self.left.seek(&right_key);
            } else if right_key < left_key {
                self.right.seek(&left_key);
            } else {
                while let Some((next, item)) = self.right.peek() {
                    if next != left_key {
                        break;
                    }
                    self.group.push(item);
                    self.right.take();
                }
                self.pending = self.left.take();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grouped(pairs: &[(i32, char)]) -> BTreeMap<i32, Vec<char>> {
        let mut map = BTreeMap::<i32, Vec<char>>::new();
        for &(key, value) in pairs {
            map.entry(key).or_default().push(value);
        }
        map
    }

    #[test]
    fn ordered_cursor_enumerates_in_key_order_and_seek_discards_below_bound() {
        let map = &grouped(&[(3, 'c'), (1, 'a'), (1, 'b'), (5, 'e')]);
        let pairs: Vec<_> = map.ordered().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(pairs, [(1, 'a'), (1, 'b'), (3, 'c'), (5, 'e')]);
        for bound in 0..=6 {
            let mut cursor = map.ordered();
            cursor.seek(&&bound);
            let rest: Vec<_> = cursor.map(|(k, _)| *k).collect();
            let expected: Vec<_> = [1, 1, 3, 5].into_iter().filter(|k| *k >= bound).collect();
            assert_eq!(rest, expected, "seek({bound})");
        }
        // Seeking within a group keeps the group's remaining values.
        let mut cursor = map.ordered();
        assert_eq!(cursor.next().map(|(k, v)| (*k, *v)), Some((1, 'a')));
        cursor.seek(&&1);
        assert_eq!(cursor.next().map(|(k, v)| (*k, *v)), Some((1, 'b')));
        cursor.seek(&&4);
        assert_eq!(cursor.next().map(|(k, v)| (*k, *v)), Some((5, 'e')));

        let set = &BTreeSet::from([2, 4, 6]);
        let mut keys = set.ordered();
        keys.seek(&&3);
        assert_eq!(keys.map(|(k, ())| *k).collect::<Vec<_>>(), [4, 6]);
    }

    #[test]
    fn merge_equals_nested_loop_join_on_keys_with_multiplicity() {
        let universe = [(0, 'a'), (0, 'b'), (1, 'c'), (2, 'd'), (2, 'e'), (4, 'f')];
        let pick = |mask: u32| -> Vec<(i32, char)> {
            universe
                .iter()
                .enumerate()
                .filter_map(|(i, &p)| (mask & (1 << i) != 0).then_some(p))
                .collect()
        };
        for left_mask in 0..1u32 << universe.len() {
            for right_mask in 0..1u32 << universe.len() {
                let (left, right) = (pick(left_mask), pick(right_mask));
                let expected: Vec<_> = left
                    .iter()
                    .flat_map(|&(k, a)| {
                        right
                            .iter()
                            .filter(move |(rk, _)| *rk == k)
                            .map(move |&(_, b)| (k, a, b))
                    })
                    .collect();
                let (l, r) = (&grouped(&left), &grouped(&right));
                let actual: Vec<_> = merge(l.ordered(), r.ordered())
                    .map(|(k, a, b)| (*k, *a, *b))
                    .collect();
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn sorted_stream_merges_like_an_ordered_map() {
        let left = grouped(&[(1, 'a'), (2, 'b'), (2, 'c'), (5, 'd')]);
        let stream = [(1, 'x'), (2, 'y'), (3, 'z'), (5, 'w')];
        let right = grouped(&stream);
        let via_map: Vec<_> = merge((&left).ordered(), (&right).ordered())
            .map(|(k, l, r)| (*k, *l, *r))
            .collect();
        let via_stream: Vec<_> = merge(
            (&left).ordered(),
            sorted_by(stream.iter().map(|(k, v)| (k, v))),
        )
        .map(|(k, l, r)| (*k, *l, *r))
        .collect();
        assert_eq!(via_map, via_stream);
    }

    #[test]
    fn merge_over_key_sets_intersects() {
        let left = &BTreeSet::from([1, 2, 3, 5, 8]);
        let right = &BTreeSet::from([2, 3, 4, 8, 9]);
        let keys: Vec<_> = merge(left.ordered(), right.ordered())
            .map(|(k, (), ())| *k)
            .collect();
        assert_eq!(keys, [2, 3, 8]);
    }
}
