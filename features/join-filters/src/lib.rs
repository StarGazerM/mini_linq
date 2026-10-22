//! Optional conservative structures, selected explicitly with
//! `cq_rewrites::retype(rN join_filter => cq_join_filters::BloomFilter<16384>)`.
//! Construction uses the public `Build` protocol. A hit is never an exact
//! membership answer: the recorded covering probe remains mandatory.
//!
//! A row predicate alone cannot stand in for conservative membership, even when
//! it supports `Build`. Generated Rust checks the recorded capability:
//!
//! ```compile_fail
//! struct PredicateOnly;
//! impl<I: Iterator<Item = (i64, ())>> cq_storage::Build<I> for PredicateOnly {
//!     fn build(_: I) -> Self { Self }
//! }
//! impl cq_storage::ReadPredicate<i64> for PredicateOnly {
//!     fn test(&self, _: &i64) -> bool { false }
//! }
//! let left = [(1_i64, 2_i64)];
//! let right = [(1_i64, 3_i64)];
//! let _ = mini_linq::passes::select! {
//!     pipeline { cq_rewrites::join_filter(l),
//!         cq_rewrites::retype(l join_filter => PredicateOnly), mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
//!     physical {
//!         relation L(a:i64,b:i64) = (left); relation R(c:i64,d:i64) = (right);
//!         l = pipe rename L@0 {a -> x,b -> y};
//!         r = pipe rename R@1 {c -> x,d -> z};
//!         s = pipe range select r if (z > 1);
//!         b = let project s keep {x,z};
//!         j = pipe probe first [x] l,b;
//!         out = pipe canon project j keep {x,y,z}; return out as answer(x,y,z);
//!     }
//! }.collect::<Vec<_>>();
//! ```
#![forbid(unsafe_code)]
use cq_storage::{Build, ConservativeMembership, ReadPredicate};

/// Fixed-budget Bloom filter with four independently mixed bit positions.
/// The budget is static; construction never samples or adapts to cardinality.
pub struct BloomFilter<const WORDS: usize> {
    bits: Box<[u64]>,
    bounds: Option<(i64, i64)>,
    _allocation: cq_storage::Retention,
}
impl<const WORDS: usize> BloomFilter<WORDS> {
    pub fn from_keys(keys: impl IntoIterator<Item = i64>) -> Self {
        assert!(WORDS > 0, "Bloom word budget must be positive");
        let mut result = Self {
            bits: vec![0; WORDS].into_boxed_slice(),
            bounds: None,
            _allocation: cq_storage::Retention::new(WORDS * 8),
        };
        for key in keys {
            result.bounds = Some(
                result
                    .bounds
                    .map_or((key, key), |(lo, hi)| (lo.min(key), hi.max(key))),
            );
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
        std::array::from_fn(|i| {
            (a.wrapping_add((i as u64).wrapping_mul(b)) % (WORDS as u64 * 64)) as usize
        })
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
