//! Instrumentation for a recorded conservative membership structure.
use crate::{ConservativeMembership, ReadPredicate};

/// Instrumentation boundary: passing rows continue to the recorded exact probe.
pub struct JoinFilterPredicate<'a, P>(pub &'a P);
impl<K, P: ConservativeMembership<K>> ReadPredicate<K> for JoinFilterPredicate<'_, P> {
    fn test(&self, key: &K) -> bool {
        crate::count!(join_filter_tested);
        let keep = self.0.test(key);
        if keep {
            crate::count!(join_filter_passed);
        }
        keep
    }
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        self.0.may_contain_in(lo, hi)
    }
}
