//! Ordinary grouped hash maps with exact set-valued payload buckets.
//!
//! The layout is a provider choice, not a canonical-index kind. Full rows are
//! key plus payload; the inner set makes payload membership a hash probe.

use crate::rows::{Columns, Concat};
use crate::{Candidates, ExactMembership, IndexRead, PointLookup, PointRows};
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet, hash_set};
use std::hash::{BuildHasher, Hash};

impl<'a, K: Eq + Hash, V, S: BuildHasher, T> IndexRead for &'a HashMap<K, HashSet<V, T>, S> {
    type Key = K;
    type Item = &'a V;
    type Cursor = crate::instrument::Cursor<hash_set::Iter<'a, V>>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        crate::instrument::cursor!(
            self.get(key)
                .map_or_else(hash_set::Iter::default, HashSet::iter)
        )
    }
}

impl<'a, K: Eq + Hash, V, S: BuildHasher, T> PointLookup for &'a HashMap<K, HashSet<V, T>, S>
where
    &'a K: Columns,
    &'a V: Columns,
    <&'a K as Columns>::Row: Concat<<&'a V as Columns>::Row>,
{
    type Key = K;
    type Row = <<&'a K as Columns>::Row as Concat<<&'a V as Columns>::Row>>::Row;
    type Cursor = PointRows<&'a K, hash_set::Iter<'a, V>>;

    fn point_lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        let found = self.get_key_value(key);
        PointRows::new(
            found.map(|(key, _)| key),
            found.map_or_else(hash_set::Iter::default, |(_, values)| values.iter()),
        )
    }
}

impl<'a, K: Eq + Hash, V, S: BuildHasher, T> Candidates for &'a HashMap<K, HashSet<V, T>, S> {
    type Key = K;
    type Item = &'a V;
    type Iter = crate::instrument::Cursor<hash_set::Iter<'a, V>>;

    fn count(&self, key: &K) -> usize {
        crate::count!(lookups);
        self.get(key).map_or(0, HashSet::len)
    }

    fn keys(&self, key: &K) -> Self::Iter {
        crate::count!(lookups);
        crate::instrument::cursor!(
            self.get(key)
                .map_or_else(hash_set::Iter::default, HashSet::iter)
        )
    }
}

impl<K, Q: ?Sized, V, S, T> ExactMembership<Q> for HashMap<K, HashSet<V, T>, S>
where
    K: Borrow<Q> + Eq + Hash,
    Q: Eq + Hash,
    S: BuildHasher,
{
    fn contains(&self, key: &Q) -> bool {
        crate::count!(lookups);
        self.get(key).is_some_and(|values| !values.is_empty())
    }
}
