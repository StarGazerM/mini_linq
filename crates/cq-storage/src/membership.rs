//! Exact immutable key lookup, distinct from conservative summaries.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::{BuildHasher, Hash};

/// Exact membership in the collection's represented key set.
///
/// `contains` is true if and only if the key belongs to that set. Repeated calls
/// during an immutable borrow must agree. When a caller constructs the set with
/// `FromIterator`, its representation must contain every supplied distinct key
/// and no others; the Rust construction trait alone cannot prove this law.
/// Conservative filters cannot implement this interface by returning possible
/// membership: they require a separate exact residual lookup.
pub trait ExactMembership<K: ?Sized> {
    fn contains(&self, key: &K) -> bool;
}

impl<K, Q: ?Sized, S> ExactMembership<Q> for HashSet<K, S>
where
    K: Borrow<Q> + Eq + Hash,
    Q: Eq + Hash,
    S: BuildHasher,
{
    fn contains(&self, key: &Q) -> bool {
        HashSet::contains(self, key)
    }
}

impl<K, Q: ?Sized> ExactMembership<Q> for BTreeSet<K>
where
    K: Borrow<Q> + Ord,
    Q: Ord,
{
    fn contains(&self, key: &Q) -> bool {
        BTreeSet::contains(self, key)
    }
}

/// A grouped ordered map represents the key set of its groups.
impl<K, Q: ?Sized, V> ExactMembership<Q> for BTreeMap<K, V>
where
    K: Borrow<Q> + Ord,
    Q: Ord,
{
    fn contains(&self, key: &Q) -> bool {
        BTreeMap::contains_key(self, key)
    }
}

impl<K: ?Sized, T: ExactMembership<K> + ?Sized> ExactMembership<K> for &T {
    fn contains(&self, key: &K) -> bool {
        T::contains(self, key)
    }
}
