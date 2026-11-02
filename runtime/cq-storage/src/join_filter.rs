//! Instrumentation for a recorded conservative membership structure.
use crate::{ConservativeMembership, ReadPredicate};

/// Identity predicate before a join filter is built. It carries no provider or
/// key type, so a later binding can infer the concrete capability-selected
/// provider without constraining this earlier read.
#[derive(Clone, Copy, Default)]
pub struct NoJoinFilter;

impl<K> ReadPredicate<K> for NoJoinFilter {
    #[inline]
    fn is_identity(&self) -> bool {
        true
    }

    #[inline]
    fn test(&self, _: &K) -> bool {
        true
    }

    #[inline]
    fn test_chunk(&self, keys: &[K], mask: &mut [bool]) {
        assert_eq!(keys.len(), mask.len(), "chunk keys and mask lengths differ");
    }
}

/// Instrumentation boundary: passing rows continue to the recorded exact probe.
/// An abandoned filter (`None`) passes every key, which the one-sided
/// conservative law permits.
pub struct JoinFilterPredicate<'a, P>(pub Option<&'a P>);
impl<K, P: ConservativeMembership<K>> ReadPredicate<K> for JoinFilterPredicate<'_, P> {
    #[inline]
    fn is_identity(&self) -> bool {
        self.0.is_none()
    }

    #[inline]
    fn test(&self, key: &K) -> bool {
        let Some(filter) = self.0 else {
            return true;
        };
        crate::count!(join_filter_tested);
        let keep = filter.test(key);
        if keep {
            crate::count!(join_filter_passed);
        }
        keep
    }
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        self.0.is_none_or(|filter| filter.may_contain_in(lo, hi))
    }
}

/// Formal rule IF-JoinFilterAbandon: whether a build keeping `kept` of its
/// source occurrence's `population` rows is selective enough to construct its
/// recorded filters. Structures records the abandon threshold `percent` (35,
/// the creation threshold of dynamic predicate transfer, RPT+) with the base
/// whose length is the population; a build keeping more abandons its filters
/// before construction. A filter recorded without a threshold is always built.
#[inline]
pub fn selective_build(kept: usize, population: usize, percent: usize) -> bool {
    kept * 100 <= population * percent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abandon_threshold_is_inclusive_percent_of_population() {
        assert!(selective_build(35, 100, 35));
        assert!(!selective_build(36, 100, 35));
        assert!(selective_build(0, 0, 35));
        assert!(!selective_build(1, 0, 35));
        assert!(selective_build(7, 20, 35));
    }
}
