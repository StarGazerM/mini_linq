//! Materialized answer sets at the output/input boundary.
use std::collections::{HashMap, HashSet, hash_map, hash_set};
use std::hash::Hash;

use crate::{Canonical, ExactMembership, IndexRead};

/// A distinct set of answer rows, iterable as an unkeyed host.
///
/// An answer set is a canonical host: declare it `#[host(canon)]`.
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
/// Use tuple keys and tuple payloads in declared column order, and declare
/// `#[host(keyed [key_columns], canon)]`. For example, an answer `(a,b,c)`
/// grouped by `b` is supplied as `((b,), (a,c))`. Both tuples may own or borrow
/// their fields. No key or payload is cloned during collection. Like ordinary
/// host maps, bind a grouped host by reference when probing it from a query.
#[derive(Clone, Debug)]
pub struct GroupedAnswers<K, V> {
    groups: HashMap<K, Answers<V>>,
}
impl<K: Eq + Hash, V: Eq + Hash> GroupedAnswers<K, V> {
    pub fn collect_from(rows: impl IntoIterator<Item = (K, V)>) -> Self {
        let mut groups: HashMap<K, Answers<V>> = HashMap::new();
        for (key, value) in rows {
            groups
                .entry(key)
                .or_insert_with(|| Answers {
                    rows: HashSet::new(),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collection_deduplicates_complete_rows_and_lookup_selects_the_group() {
        let answers = Answers::collect_from([(1, 2), (1, 2), (1, 3), (2, 4)]);
        assert_eq!(answers.len(), 3);
        let grouped = GroupedAnswers::collect_from(answers.iter().map(|(k, v)| ((k,), (v,))));
        let values: HashSet<_> = (&grouped).lookup(&(&1,)).copied().collect();
        assert_eq!(values, [(&2,), (&3,)].into());
        assert_eq!((&grouped).lookup(&(&9,)).count(), 0);
        assert!(grouped.contains(&(&2,)));
        assert_eq!(grouped.iter().map(|(_, v)| v.len()).sum::<usize>(), 3);
        let duplicates = GroupedAnswers::collect_from([(1, 2), (1, 2), (1, 3)]);
        assert_eq!((&duplicates).lookup(&1).count(), 2);
    }
}
