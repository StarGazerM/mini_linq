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

/// Type-directed supplementary-filter policy. This is deliberately narrower
/// than membership support: adding a scalar codec must not silently introduce
/// another scan filter and its key-enumeration/build work.
pub struct JoinFilterKey<K>(std::marker::PhantomData<fn() -> K>);

impl<K> JoinFilterKey<K> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

/// Capability dispatch for supplementary join filters.
///
/// Borrowing a [`JoinFilterKey`] once supplies the conservative fallback;
/// borrowing twice selects the supported scalar policy when available.
/// Generated readers use this policy without inspecting Rust type spellings.
pub trait JoinFilterEligibility {
    fn eligible(self) -> bool;
}

impl<K> JoinFilterEligibility for &JoinFilterKey<K> {
    #[inline]
    fn eligible(self) -> bool {
        false
    }
}

impl JoinFilterEligibility for &&JoinFilterKey<i64> {
    #[inline]
    fn eligible(self) -> bool {
        true
    }
}

/// Whether a build keeping `kept` of its source occurrence's `population` rows
/// is selective enough to construct its filters: at most 35% survive, the
/// creation threshold of dynamic predicate transfer (RPT+). A build keeping
/// more rarely rejects probe-side rows, so its filters are abandoned before
/// construction. Unknown populations never abandon.
pub fn selective_build(kept: usize, population: Option<usize>) -> bool {
    population.is_none_or(|population| kept * 20 <= population * 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplementary_policy_resolves_aliases_without_enabling_other_keys() {
        type KeyAlias = i64;
        assert!((&&JoinFilterKey::<KeyAlias>::new()).eligible());
        assert!(!(&&JoinFilterKey::<i32>::new()).eligible());
        assert!(!(&&JoinFilterKey::<u32>::new()).eligible());
        assert!(!(&&JoinFilterKey::<String>::new()).eligible());
        assert!(!(&&JoinFilterKey::<(i64, i64)>::new()).eligible());
    }

    #[test]
    fn ineligible_key_skips_counts_and_filter_construction() {
        let policy = JoinFilterKey::<i32>::new();
        let built: Option<()> = ((&&policy).eligible() && selective_build(panic_count(), None))
            .then(|| panic!("ineligible filter must not be built"));
        assert!(built.is_none());

        fn panic_count() -> usize {
            panic!("ineligible filter must not enumerate or count keys")
        }
    }
}
