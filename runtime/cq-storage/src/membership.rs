//! Exact and conservative membership in a represented build key set.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::{BuildHasher, Hash};

/// One-sided membership in the keys supplied through `Build`.
///
/// For every supplied key, `test` must be true. False positives are allowed;
/// users must retain an exact residual probe. A provider may weaken its tests
/// (even to always true) at runtime, but must never reject a supplied key.
/// `Build` consumes `(key, payload)` pairs; join filters receive `(key, ())`.
/// Rust bounds enforce capabilities, not these semantic laws.
pub trait ConservativeMembership<K: ?Sized> {
    fn test(&self, key: &K) -> bool;
    /// False proves no build key lies in the closed interval. Without a
    /// summary, retaining every block is lawful. Bounds do not prove membership.
    fn may_contain_in(&self, _lo: &K, _hi: &K) -> bool {
        true
    }
}

/// Exact membership is always a lawful conservative filter. Interval pruning
/// is optional; this blanket implementation conservatively retains blocks.
impl<K: ?Sized, T: ExactMembership<K> + ?Sized> ConservativeMembership<K> for T {
    fn test(&self, key: &K) -> bool {
        self.contains(key)
    }
}

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
        crate::count!(lookups);
        HashSet::contains(self, key)
    }
}

impl<K, Q: ?Sized> ExactMembership<Q> for BTreeSet<K>
where
    K: Borrow<Q> + Ord,
    Q: Ord,
{
    fn contains(&self, key: &Q) -> bool {
        crate::count!(lookups);
        BTreeSet::contains(self, key)
    }
}

/// A grouped hash map represents the key set of its groups.
impl<K, Q: ?Sized, V, S> ExactMembership<Q> for std::collections::HashMap<K, Vec<V>, S>
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

/// A grouped ordered map represents the key set of its groups.
impl<K, Q: ?Sized, V> ExactMembership<Q> for BTreeMap<K, Vec<V>>
where
    K: Borrow<Q> + Ord,
    Q: Ord,
{
    fn contains(&self, key: &Q) -> bool {
        crate::count!(lookups);
        self.get(key).is_some_and(|values| !values.is_empty())
    }
}

impl<K: ?Sized, T: ExactMembership<K> + ?Sized> ExactMembership<K> for &T {
    fn contains(&self, key: &K) -> bool {
        T::contains(self, key)
    }
}
