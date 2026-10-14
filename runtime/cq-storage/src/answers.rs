//! Materialized answer sets at the output/input boundary.
use crate::HashSet;
use std::collections::{hash_map, hash_set};
type HashMap<K, V> = std::collections::HashMap<K, V, crate::HashState>;
use std::hash::Hash;

use crate::{Canonical, ExactMembership, IndexRead};

/// A distinct set of answer rows, iterable as an unkeyed host.
///
/// An answer set can be bound to an ordinary relation declaration.
/// Rows may contain borrowed columns (`&T`) or owned columns; collecting
/// borrowed tuples retains those references without cloning their referents.
#[derive(Clone, Debug)]
pub struct Answers<T> {
    rows: HashSet<T>,
}

impl<T: Eq + Hash> Answers<T> {
    /// Formal rule IF-Output: Materialize any answer iterator as a canonical host.
    pub fn collect_from(rows: impl IntoIterator<Item = T>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
        }
    }
}
impl<T> Answers<T> {
    pub fn iter(&self) -> hash_set::Iter<'_, T> {
        self.rows.iter()
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}
impl<T> Canonical for Answers<T> {}
impl<T> IntoIterator for Answers<T> {
    type Item = T;
    type IntoIter = hash_set::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.into_iter()
    }
}
impl<'a, T> IntoIterator for &'a Answers<T> {
    type Item = &'a T;
    type IntoIter = hash_set::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Distinct `(key, payload)` rows grouped for in-place keyed host reads.
///
/// The key occupies the leading declared columns and the payload the rest.
/// For `relation R(a: A, b: B, c: C)`, grouping by `(a,b)` supplies
/// `((a,b), (c,))`. To group by `b`, declare `b` first. Both tuples may own
/// or borrow their fields; collection clones neither key nor payload.
#[derive(Clone, Debug)]
pub struct GroupedAnswers<K, V> {
    groups: HashMap<K, Answers<V>>,
}
impl<K: Eq + Hash, V: Eq + Hash> GroupedAnswers<K, V> {
    pub fn collect_from(rows: impl IntoIterator<Item = (K, V)>) -> Self {
        let mut groups: HashMap<K, Answers<V>> = HashMap::default();
        for (key, value) in rows {
            groups
                .entry(key)
                .or_insert_with(|| Answers {
                    rows: HashSet::default(),
                })
                .rows
                .insert(value);
        }
        Self { groups }
    }
}
impl<K, V> GroupedAnswers<K, V> {
    pub fn iter(&self) -> hash_map::Iter<'_, K, Answers<V>> {
        self.groups.iter()
    }
}
impl<K, V> Canonical for GroupedAnswers<K, V> {}
impl<'a, K: Eq + Hash, V> IndexRead for &'a GroupedAnswers<K, V> {
    type Key = K;
    type Item = &'a V;
    type Cursor = std::iter::Flatten<std::option::IntoIter<hash_set::Iter<'a, V>>>;
    fn lookup(&self, key: &K) -> Self::Cursor {
        self.groups
            .get(key)
            .map(Answers::iter)
            .into_iter()
            .flatten()
    }
}
impl<K: Eq + Hash, V> ExactMembership<K> for GroupedAnswers<K, V> {
    fn contains(&self, key: &K) -> bool {
        self.groups.contains_key(key)
    }
}

impl<'a, K: Eq + Hash, V> crate::PointLookup for &'a GroupedAnswers<K, V>
where
    &'a K: crate::rows::Columns,
    &'a V: crate::rows::Columns,
    <&'a K as crate::rows::Columns>::Row: crate::rows::Concat<<&'a V as crate::rows::Columns>::Row>,
{
    type Key = K;
    type Row = <<&'a K as crate::rows::Columns>::Row as crate::rows::Concat<
        <&'a V as crate::rows::Columns>::Row,
    >>::Row;
    type Cursor =
        crate::PointRows<&'a K, std::iter::Flatten<std::option::IntoIter<hash_set::Iter<'a, V>>>>;
    fn point_lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        let found = self.groups.get_key_value(key);
        crate::PointRows {
            key: found.map(|(k, _)| k),
            values: found.map(|(_, v)| v.iter()).into_iter().flatten(),
        }
    }
}
impl<'a, K: Eq + Hash> crate::PointLookup for &'a Answers<K>
where
    &'a K: crate::rows::Columns,
{
    type Key = K;
    type Row = <&'a K as crate::rows::Columns>::Row;
    type Cursor = std::iter::Map<std::option::IntoIter<&'a K>, fn(&'a K) -> Self::Row>;
    fn point_lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        self.rows
            .get(key)
            .into_iter()
            .map(crate::rows::Columns::columns)
    }
}

/// A checked constant-key aggregate: zero or one group, with no hashing.
#[derive(Clone, Debug)]
pub struct SingleGroup<K, V> {
    group: Option<(K, [V; 1])>,
}
impl<K, V> SingleGroup<K, V> {
    pub fn from_option(group: Option<(K, V)>) -> Self {
        Self {
            group: group.map(|(key, value)| (key, [value])),
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = (&K, &[V; 1])> {
        self.group.iter().map(|(key, values)| (key, values))
    }
}
impl<K, V> Canonical for SingleGroup<K, V> {}
impl<K: Eq, V> ExactMembership<K> for SingleGroup<K, V> {
    fn contains(&self, key: &K) -> bool {
        self.group.as_ref().is_some_and(|(stored, _)| stored == key)
    }
}
impl<'a, K: Eq, V> IndexRead for &'a SingleGroup<K, V> {
    type Key = K;
    type Item = &'a V;
    type Cursor = std::option::IntoIter<&'a V>;
    fn lookup(&self, key: &K) -> Self::Cursor {
        self.group
            .as_ref()
            .filter(|(stored, _)| stored == key)
            .map(|(_, values)| &values[0])
            .into_iter()
    }
}
impl<'a, K: Eq, V> crate::PointLookup for &'a SingleGroup<K, V>
where
    &'a K: crate::rows::Columns,
    &'a V: crate::rows::Columns,
    <&'a K as crate::rows::Columns>::Row: crate::rows::Concat<<&'a V as crate::rows::Columns>::Row>,
{
    type Key = K;
    type Row = <<&'a K as crate::rows::Columns>::Row as crate::rows::Concat<
        <&'a V as crate::rows::Columns>::Row,
    >>::Row;
    type Cursor = crate::PointRows<&'a K, std::option::IntoIter<&'a V>>;
    fn point_lookup(&self, key: &K) -> Self::Cursor {
        let found = self.group.as_ref().filter(|(stored, _)| stored == key);
        crate::PointRows {
            key: found.map(|(key, _)| key),
            values: found.map(|(_, values)| &values[0]).into_iter(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collection_deduplicates_complete_rows_and_lookup_selects_the_group() {
        let answers = Answers::collect_from([(1, 2), (1, 2), (1, 3), (2, 4)]);
        assert_eq!(answers.len(), 3);
        let grouped = GroupedAnswers::collect_from(answers.iter().map(|(k, v)| ((k,), (v,))));
        let values: HashSet<_> = (&grouped).lookup(&(&1,)).copied().collect();
        assert_eq!(values, [(&2,), (&3,)].into_iter().collect());
        assert_eq!((&grouped).lookup(&(&9,)).count(), 0);
        assert!(grouped.contains(&(&2,)));
        assert_eq!(grouped.iter().map(|(_, v)| v.len()).sum::<usize>(), 3);
        let duplicates = GroupedAnswers::collect_from([(1, 2), (1, 2), (1, 3)]);
        assert_eq!((&duplicates).lookup(&1).count(), 2);
    }
}
