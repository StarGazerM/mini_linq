//! Point lookup returns full borrowed rows from a host-owned key and payload.
//! Their shared references remain valid for the host's lifetime. Leibniz
//! equality makes any equal key the same value; provenance adds no meaning.
use crate::rows::{Columns, Concat};
use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hash};

/// Native point access to full borrowed rows in declaration order.
/// Payload-only IndexRead does not supply a host-lifetime key reference.
pub trait PointLookup: Clone {
    type Key: Eq;
    type Row: Copy;
    type Cursor: Iterator<Item = Self::Row>;
    fn point_lookup(&self, key: &Self::Key) -> Self::Cursor;
}
impl<T: PointLookup> PointLookup for &T {
    type Key = T::Key;
    type Row = T::Row;
    type Cursor = T::Cursor;
    fn point_lookup(&self, key: &Self::Key) -> Self::Cursor {
        T::point_lookup(*self, key)
    }
}

/// A host-owned key reference paired with each stored payload reference.
pub struct PointRows<K, I> {
    pub(crate) key: Option<K>,
    pub(crate) values: I,
}
impl<K, I> PointRows<K, I> {
    /// Rows of one found (or absent) key paired with its payload cursor, for
    /// providers outside this crate that store grouped buckets.
    pub fn new(key: Option<K>, values: I) -> Self {
        Self { key, values }
    }
}
impl<K: Copy + Columns, I: Iterator> Iterator for PointRows<K, I>
where
    I::Item: Columns,
    K::Row: Concat<<I::Item as Columns>::Row>,
{
    type Item = <K::Row as Concat<<I::Item as Columns>::Row>>::Row;
    fn next(&mut self) -> Option<Self::Item> {
        crate::count!(cursor_steps);
        let row = self.key?.columns().concat(self.values.next()?.columns());
        crate::count!(rows_enumerated);
        Some(row)
    }
}
impl<'a, K: Eq + Hash, V, S: BuildHasher> PointLookup for &'a HashMap<K, Vec<V>, S>
where
    &'a K: Columns,
    &'a V: Columns,
    <&'a K as Columns>::Row: Concat<<&'a V as Columns>::Row>,
{
    type Key = K;
    type Row = <<&'a K as Columns>::Row as Concat<<&'a V as Columns>::Row>>::Row;
    type Cursor = PointRows<&'a K, std::slice::Iter<'a, V>>;
    fn point_lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        let found = self.get_key_value(key);
        PointRows {
            key: found.map(|(k, _)| k),
            values: found.map_or([].iter(), |(_, v)| v.iter()),
        }
    }
}
impl<'a, K: Ord, V> PointLookup for &'a BTreeMap<K, Vec<V>>
where
    &'a K: Columns,
    &'a V: Columns,
    <&'a K as Columns>::Row: Concat<<&'a V as Columns>::Row>,
{
    type Key = K;
    type Row = <<&'a K as Columns>::Row as Concat<<&'a V as Columns>::Row>>::Row;
    type Cursor = PointRows<&'a K, std::slice::Iter<'a, V>>;
    fn point_lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        let found = self.get_key_value(key);
        PointRows {
            key: found.map(|(k, _)| k),
            values: found.map_or([].iter(), |(_, v)| v.iter()),
        }
    }
}

impl<'a, K: Eq + Hash, S: BuildHasher> PointLookup for &'a std::collections::HashSet<K, S>
where
    &'a K: Columns,
{
    type Key = K;
    type Row = <&'a K as Columns>::Row;
    type Cursor = std::iter::Map<std::option::IntoIter<&'a K>, fn(&'a K) -> Self::Row>;
    fn point_lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        self.get(key).into_iter().map(Columns::columns)
    }
}
impl<'a, K: Ord> PointLookup for &'a std::collections::BTreeSet<K>
where
    &'a K: Columns,
{
    type Key = K;
    type Row = <&'a K as Columns>::Row;
    type Cursor = std::iter::Map<std::option::IntoIter<&'a K>, fn(&'a K) -> Self::Row>;
    fn point_lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        self.get(key).into_iter().map(Columns::columns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn map_points_return_the_key_value_and_every_payload() {
        let hash = HashMap::from([((5,), vec![(1,), (2,)]), ((7,), vec![])]);
        let rows: Vec<_> = (&hash).point_lookup(&(5,)).collect();
        assert_eq!(rows, [(&5, &1), (&5, &2)]);
        assert_eq!((&hash).point_lookup(&(7,)).count(), 0);
        assert_eq!((&hash).point_lookup(&(8,)).count(), 0);
        let ordered: BTreeMap<_, _> = hash.into_iter().collect();
        assert_eq!(
            (&ordered).point_lookup(&(5,)).collect::<Vec<_>>(),
            [(&5, &1), (&5, &2)]
        );
    }
    #[test]
    fn set_points_return_the_tuple_values() {
        let hash = std::collections::HashSet::from([(5, 6)]);
        assert_eq!(
            (&hash).point_lookup(&(5, 6)).collect::<Vec<_>>(),
            [(&5, &6)]
        );
        let ordered = std::collections::BTreeSet::from([(5, 6)]);
        assert_eq!(
            (&ordered).point_lookup(&(5, 6)).collect::<Vec<_>>(),
            [(&5, &6)]
        );
    }
}
