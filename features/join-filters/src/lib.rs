//! Optional conservative structures, selected explicitly with
//! `cq_rewrites::retype(providers { bloom_filter = cq_join_filters::BloomFilter<16384>
//! { reads [membership], build typed }; } rN join_filter => @bloom_filter as
//! cq_join_filters::BloomFilter<16384>)`.
//! Construction supports both standalone `Build` and construction-attached
//! `BuildSidecar` observers. A hit is never an exact membership answer: the
//! recorded covering probe remains mandatory.
//!
//! A row predicate alone cannot stand in for conservative membership, even when
//! it supports `BuildSidecar`. Generated Rust checks the recorded capability:
//!
//! ```compile_fail
//! struct PredicateOnly;
//! impl cq_storage::BuildSidecar<i64> for PredicateOnly {
//!     type Builder = Self;
//!     fn sidecar_builder() -> Self { Self }
//! }
//! impl cq_storage::SidecarBuilder<i64> for PredicateOnly {
//!     type Output = Self;
//!     fn observe(&mut self, _: &i64) {}
//!     fn finish(self) -> Self { self }
//! }
//! impl cq_storage::ReadPredicate<i64> for PredicateOnly {
//!     fn test(&self, _: &i64) -> bool { false }
//! }
//! let left = [(1_i64, 2_i64)];
//! let right = [(1_i64, 3_i64)];
//! let _ = mini_linq::passes::select! {
//!     pipeline { cq_rewrites::join_filter(l),
//!         cq_rewrites::retype(
//!             providers { predicate_only = PredicateOnly { reads [membership], build inferred }; }
//!             l join_filter => @predicate_only as PredicateOnly
//!         ),
//!         mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
//!     physical {
//!         relation L(a:i64,b:i64) = (left); relation R(c:i64,d:i64) = (right);
//!         l = pipe rename L@0 {a -> x,b -> y};
//!         r = pipe rename R@1 {c -> x,d -> z};
//!         s = pipe range select r if (z > 1);
//!         b = let project s keep {x,z};
//!         j = pipe probe first [x] l,b;
//!         out = let canon project j keep {x,y,z}; return out as answer(x,y,z);
//!     }
//! }.collect::<Vec<_>>();
//! ```
#![forbid(unsafe_code)]
use cq_storage::{
    Build, BuildSidecar, ConservativeMembership, MergeSidecarBuilder, ReadPredicate, SidecarBuilder,
};

/// Fixed-budget Bloom filter with four independently mixed bit positions.
/// The budget is static; construction never samples or adapts to cardinality.
pub struct BloomFilter<const WORDS: usize> {
    bits: Box<[u64]>,
    bounds: Option<(i64, i64)>,
    _allocation: cq_storage::Retention,
}
impl<const WORDS: usize> BloomFilter<WORDS> {
    pub fn from_keys(keys: impl IntoIterator<Item = i64>) -> Self {
        let mut builder = Self::sidecar_builder();
        for key in keys {
            builder.observe(&key);
        }
        builder.finish()
    }
    fn positions(key: i64) -> [usize; 4] {
        let mix = |mut x: u64| {
            x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
            x ^ (x >> 31)
        };
        let a = mix(key as u64);
        let b = mix((key as u64).wrapping_add(0x9e3779b97f4a7c15)) | 1;
        std::array::from_fn(|i| {
            (a.wrapping_add((i as u64).wrapping_mul(b)) % (WORDS as u64 * 64)) as usize
        })
    }
}

/// Fixed-budget observer that writes Bloom bits and extrema directly.
pub struct BloomFilterBuilder<const WORDS: usize> {
    filter: BloomFilter<WORDS>,
}

impl<const WORDS: usize> BuildSidecar<i64> for BloomFilter<WORDS> {
    type Builder = BloomFilterBuilder<WORDS>;

    fn sidecar_builder() -> Self::Builder {
        assert!(WORDS > 0, "Bloom word budget must be positive");
        BloomFilterBuilder {
            filter: Self {
                bits: vec![0; WORDS].into_boxed_slice(),
                bounds: None,
                _allocation: cq_storage::Retention::new(WORDS * 8),
            },
        }
    }
}

impl<const WORDS: usize> SidecarBuilder<i64> for BloomFilterBuilder<WORDS> {
    type Output = BloomFilter<WORDS>;

    fn observe(&mut self, key: &i64) {
        self.filter.bounds = Some(
            self.filter
                .bounds
                .map_or((*key, *key), |(lo, hi)| (lo.min(*key), hi.max(*key))),
        );
        for bit in BloomFilter::<WORDS>::positions(*key) {
            self.filter.bits[bit / 64] |= 1 << (bit % 64);
        }
    }

    fn finish(self) -> Self::Output {
        self.filter
    }
}

impl<const WORDS: usize> MergeSidecarBuilder<i64> for BloomFilterBuilder<WORDS> {
    fn merge(&mut self, other: Self) {
        for (word, other) in self.filter.bits.iter_mut().zip(other.filter.bits.iter()) {
            *word |= *other;
        }
        if let Some((min, max)) = other.filter.bounds {
            self.filter.bounds = Some(self.filter.bounds.map_or((min, max), |(lo, hi)| {
                (lo.min(min), hi.max(max))
            }));
        }
    }
}
impl<const WORDS: usize> ConservativeMembership<i64> for BloomFilter<WORDS> {
    fn test(&self, key: &i64) -> bool {
        self.bounds.is_some_and(|(lo, hi)| lo <= *key && *key <= hi)
            && Self::positions(*key)
                .into_iter()
                .all(|bit| self.bits[bit / 64] & (1 << (bit % 64)) != 0)
    }
    fn may_contain_in(&self, lo: &i64, hi: &i64) -> bool {
        lo <= hi
            && self
                .bounds
                .is_some_and(|(min, max)| *lo <= max && min <= *hi)
    }
}

impl<const WORDS: usize, V, I: Iterator<Item = (i64, V)>> Build<I> for BloomFilter<WORDS> {
    fn build(source: I) -> Self {
        Self::from_keys(source.map(|(key, _)| key))
    }
}
impl<const WORDS: usize> ReadPredicate<i64> for BloomFilter<WORDS> {
    fn test(&self, key: &i64) -> bool {
        ConservativeMembership::test(self, key)
    }
    fn any_in(&self, lo: &i64, hi: &i64) -> bool {
        ConservativeMembership::may_contain_in(self, lo, hi)
    }
}

/// A lawful worst-case filter. Useful for checking that the exact covering probe
/// alone enforces the join constraint, including with arbitrary false positives.
pub struct AlwaysTrue;

/// Zero-state observer: retaining every probe is lawful even for empty input.
pub struct AlwaysTrueBuilder<K>(std::marker::PhantomData<fn(&K)>);

impl<K> BuildSidecar<K> for AlwaysTrue {
    type Builder = AlwaysTrueBuilder<K>;

    fn sidecar_builder() -> Self::Builder {
        AlwaysTrueBuilder(std::marker::PhantomData)
    }
}

impl<K> SidecarBuilder<K> for AlwaysTrueBuilder<K> {
    type Output = AlwaysTrue;

    fn observe(&mut self, _: &K) {}

    fn finish(self) -> Self::Output {
        AlwaysTrue
    }
}

impl<K> MergeSidecarBuilder<K> for AlwaysTrueBuilder<K> {
    fn merge(&mut self, _: Self) {}
}
impl<K, V, I: Iterator<Item = (K, V)>> Build<I> for AlwaysTrue {
    fn build(source: I) -> Self {
        source.for_each(drop);
        Self
    }
}
impl ConservativeMembership<i64> for AlwaysTrue {
    fn test(&self, _: &i64) -> bool {
        true
    }
}
impl ReadPredicate<i64> for AlwaysTrue {
    fn test(&self, key: &i64) -> bool {
        ConservativeMembership::test(self, key)
    }
}

#[cfg(test)]
mod sidecar_tests {
    use super::{AlwaysTrue, BloomFilter};
    use cq_storage::{BuildSidecar, ConservativeMembership, SidecarBuilder};

    #[test]
    fn bloom_sidecar_preserves_extremes_and_duplicates() {
        let keys = [i64::MIN, -1, 0, 1, i64::MAX, i64::MIN];
        let mut builder = BloomFilter::<1>::sidecar_builder();
        for key in &keys {
            builder.observe(key);
        }
        let filter = builder.finish();
        assert_eq!(filter.bounds, Some((i64::MIN, i64::MAX)));
        for key in &keys {
            assert!(filter.test(key));
            assert!(filter.may_contain_in(key, key));
        }
        assert!(!filter.may_contain_in(&1, &0));
    }

    #[test]
    fn empty_bloom_and_disjoint_bounds_reject() {
        let empty = BloomFilter::<1>::sidecar_builder().finish();
        assert!(!empty.test(&0));
        assert!(!empty.may_contain_in(&i64::MIN, &i64::MAX));
        let mut builder = BloomFilter::<1>::sidecar_builder();
        builder.observe(&-5);
        builder.observe(&5);
        let filter = builder.finish();
        assert!(!filter.test(&6));
        assert!(!filter.may_contain_in(&i64::MIN, &-6));
        assert!(!filter.may_contain_in(&6, &i64::MAX));
    }

    #[test]
    fn saturated_bloom_collisions_do_not_replace_exact_cover() {
        let mut builder = BloomFilter::<1>::sidecar_builder();
        // Many witnesses share just 64 bits. Every odd interior key is absent
        // from the exact build, regardless of its conservative filter answer.
        for key in (-4096..=4096).step_by(2) {
            builder.observe(&key);
        }
        let filter = builder.finish();
        let mut false_positives = 0;
        for key in -4096..=4096 {
            let exact = key % 2 == 0;
            let admitted = filter.test(&key);
            if exact {
                assert!(admitted);
            } else if admitted {
                false_positives += 1;
            }
            assert_eq!(admitted && exact, exact);
        }
        assert!(false_positives > 0);
    }

    #[test]
    fn merged_bloom_observers_publish_union_and_bounds() {
        let mut left = BloomFilter::<2>::sidecar_builder();
        let mut right = BloomFilter::<2>::sidecar_builder();
        for key in [i64::MIN, -7, 0] {
            left.observe(&key);
        }
        for key in [0, 9, i64::MAX] {
            right.observe(&key);
        }
        cq_storage::MergeSidecarBuilder::merge(&mut left, right);
        cq_storage::MergeSidecarBuilder::merge(&mut left, BloomFilter::<2>::sidecar_builder());
        let filter = left.finish();
        assert_eq!(filter.bounds, Some((i64::MIN, i64::MAX)));
        for key in [i64::MIN, -7, 0, 9, i64::MAX] {
            assert!(filter.test(&key));
            assert!(filter.may_contain_in(&key, &key));
        }
    }

    #[test]
    fn always_true_sidecar_is_lawful_for_empty_and_nonclone_keys() {
        struct Key;
        let mut builder = <AlwaysTrue as BuildSidecar<Key>>::sidecar_builder();
        builder.observe(&Key);
        let filter = builder.finish();
        let empty = <AlwaysTrue as BuildSidecar<Key>>::sidecar_builder().finish();
        for key in [i64::MIN, -1, 0, 1, i64::MAX] {
            assert!(filter.test(&key));
            assert!(empty.test(&key));
            assert!(filter.may_contain_in(&key, &key));
        }
    }
}
