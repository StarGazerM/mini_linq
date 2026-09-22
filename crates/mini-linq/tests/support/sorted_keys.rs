//! A caller-owned immutable exact key collection used by the test and example.
use std::cell::Cell;

thread_local! {
    static CALLS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

/// Observational counters for verifying that a facade selected this collection.
pub fn take_counts() -> (usize, usize) {
    CALLS.with(|calls| calls.replace((0, 0)))
}

#[derive(Debug)]
pub struct SortedKeys<T> {
    keys: Vec<T>,
}

impl<K: Ord> FromIterator<K> for SortedKeys<K> {
    fn from_iter<T: IntoIterator<Item = K>>(iter: T) -> Self {
        CALLS.with(|calls| {
            let (builds, probes) = calls.get();
            calls.set((builds + 1, probes));
        });
        let mut keys: Vec<_> = iter.into_iter().collect();
        keys.sort_unstable();
        keys.dedup();
        Self { keys }
    }
}

impl<K: Ord> cq_storage::ExactMembership<K> for SortedKeys<K> {
    fn contains(&self, key: &K) -> bool {
        CALLS.with(|calls| {
            let (builds, probes) = calls.get();
            calls.set((builds, probes + 1));
        });
        self.keys.binary_search(key).is_ok()
    }
}
