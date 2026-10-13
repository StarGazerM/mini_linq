//! Conservative scalar join filters. A Bloom hit is never an exact membership
//! answer: generated readers must retain their recorded exact index probe.
use crate::ReadPredicate;

/// Fixed-budget Bloom filter with four independently mixed bit positions.
/// The budget is static; construction never samples or adapts to cardinality.
pub struct BloomFilter<const WORDS: usize> {
    bits: Box<[u64]>,
    bounds: Option<(i64, i64)>,
    _allocation: crate::counters_memory::Allocation,
}
impl<const WORDS: usize> BloomFilter<WORDS> {
    pub fn from_keys(keys: impl IntoIterator<Item = i64>) -> Self {
        assert!(WORDS > 0, "Bloom word budget must be positive");
        let mut result = Self {
            bits: vec![0; WORDS].into_boxed_slice(),
            bounds: None,
            _allocation: crate::counters_memory::Allocation::new(WORDS * 8),
        };
        for key in keys {
            result.bounds = Some(result.bounds.map_or((key, key), |(lo, hi)| (lo.min(key), hi.max(key))));
            for bit in Self::positions(key) {
                result.bits[bit / 64] |= 1 << (bit % 64);
            }
        }
        result
    }
    fn positions(key: i64) -> [usize; 4] {
        let mix = |mut x: u64| {
            x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
            x ^ (x >> 31)
        };
        let a = mix(key as u64);
        let b = mix((key as u64).wrapping_add(0x9e3779b97f4a7c15)) | 1;
        std::array::from_fn(|i| (a.wrapping_add((i as u64).wrapping_mul(b)) % (WORDS as u64 * 64)) as usize)
    }
}
impl<const WORDS: usize> ReadPredicate<i64> for BloomFilter<WORDS> {
    fn test(&self, key: &i64) -> bool {
        self.bounds.is_some_and(|(lo, hi)| lo <= *key && *key <= hi)
            && Self::positions(*key).into_iter().all(|bit| self.bits[bit / 64] & (1 << (bit % 64)) != 0)
    }
    fn any_in(&self, lo: &i64, hi: &i64) -> bool {
        lo <= hi && self.bounds.is_some_and(|(min, max)| *lo <= max && min <= *hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bloom_has_no_false_negatives_including_collisions_and_extremes() {
        let keys: Vec<_> = (0..10000).map(|x| x * 7919 - 40000000).chain([i64::MIN, i64::MAX, 0, 0]).collect();
        // Tiny budget deliberately saturates and exercises collisions.
        let filter = BloomFilter::<2>::from_keys(keys.iter().copied());
        for key in keys {
            assert!(filter.test(&key));
            assert!(filter.any_in(&key, &key));
        }
        let empty = BloomFilter::<2>::from_keys([]);
        assert!(!empty.test(&0));
        assert!(!empty.any_in(&i64::MIN, &i64::MAX));
        let filter = BloomFilter::<16>::from_keys([-7, 4, 99]);
        assert!(!filter.any_in(&100, &200));
        assert!(!filter.any_in(&-20, &-8));
        assert!(!filter.any_in(&9, &8));
    }
}

/// Instrumentation boundary: passing rows continue to the recorded exact probe.
pub struct JoinFilterPredicate<P>(pub P);
impl<K, P: ReadPredicate<K>> ReadPredicate<K> for JoinFilterPredicate<P> {
    fn test(&self, key: &K) -> bool {
        crate::count!(join_filter_tested);
        let keep = self.0.test(key);
        if keep {
            crate::count!(join_filter_passed);
        }
        keep
    }
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        self.0.any_in(lo, hi)
    }
}
