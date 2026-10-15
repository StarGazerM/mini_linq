//! Instrumentation for a recorded conservative membership structure.
use crate::{ConservativeMembership, ReadPredicate};

/// Instrumentation boundary: passing rows continue to the recorded exact probe.
/// An abandoned filter (`None`) passes every key, which the one-sided
/// conservative law permits.
pub struct JoinFilterPredicate<'a, P>(pub Option<&'a P>);
impl<K, P: ConservativeMembership<K>> ReadPredicate<K> for JoinFilterPredicate<'_, P> {
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

/// Whether a build keeping `kept` of its source occurrence's `population` rows
/// is selective enough to construct its filters: at most 35% survive, the
/// creation threshold of dynamic predicate transfer (RPT+). A build keeping
/// more rarely rejects probe-side rows, so its filters are abandoned before
/// construction. Unknown populations never abandon.
pub fn selective_build(kept: usize, population: Option<usize>) -> bool {
    population.is_none_or(|population| kept * 20 <= population * 7)
}
