//! Load-recorded integer identity, independent of each column's FOR frames.
//!
//! `KeyDomain` is the scalar Offset descriptor. `Domain` owns the frozen GPU
//! membership index used by both Offset and sparse, order-preserving Rank.
use crate::{Bounds, Result};

/// Minimum-offset codes share this record across all joined member columns.
/// `bound` is exclusive; `u32::MAX` is never a stored code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyDomain {
    pub id: u32,
    pub minimum: i64,
    pub bound: u32,
    pub bits: u32,
}

impl KeyDomain {
    pub fn from_bounds(id: u32, bounds: Bounds) -> Result<Self> {
        let Bounds::Inclusive { min, max } = bounds else {
            return Ok(Self { id, minimum: 0, bound: 0, bits: 0 });
        };
        let bound = i128::from(max) - i128::from(min) + 1;
        if !(1..i128::from(u32::MAX)).contains(&bound) {
            return Err(format!("key domain {id}: range [{min}, {max}] exceeds the u32 code domain (u32::MAX is reserved)").into());
        }
        let bound = bound as u32;
        Ok(Self { id, minimum: min, bound, bits: (32 - (bound - 1).leading_zeros()).max(1) })
    }

    pub fn bounds(self) -> Bounds {
        if self.bound == 0 { Bounds::Empty } else {
            Bounds::Inclusive { min: 0, max: i64::from(self.bound - 1) }
        }
    }

    pub fn exact(self, value: i64) -> u32 {
        let code = i128::from(value) - i128::from(self.minimum);
        if (0..i128::from(self.bound)).contains(&code) { code as u32 } else { u32::MAX }
    }

    pub fn lower(self, value: i64) -> u32 {
        (i128::from(value) - i128::from(self.minimum)).clamp(0, i128::from(self.bound)) as u32
    }

    pub fn upper(self, value: i64) -> u32 {
        (i128::from(value) - i128::from(self.minimum) + 1).clamp(0, i128::from(self.bound)) as u32
    }

    /// Logical inverse, used only at Rust reads or answer publication.
    pub fn value(self, code: u32) -> i64 {
        assert!(code < self.bound, "key domain {}: invalid code {code}", self.id);
        self.minimum.checked_add(i64::from(code)).expect("validated domain range")
    }
}

impl cq_storage::CodeDomain for KeyDomain {
    type Value = i64;
    type Code = u32;
    fn exact(&self, value: &i64) -> u32 { (*self).exact(*value) }
}
impl cq_storage::OrderedCodeDomain for KeyDomain {
    fn lower(&self, value: &i64) -> u32 { (*self).lower(*value) }
    fn upper(&self, value: &i64) -> u32 { (*self).upper(*value) }
}

/// Rank wins at and above this inclusive span/cardinality ratio.
pub const RANK_SPARSITY_THRESHOLD: u64 = 2;

pub fn use_rank(span: u32, distinct: u32) -> bool {
    distinct != 0 && u64::from(span) >= RANK_SPARSITY_THRESHOLD * u64::from(distinct)
}

/// Owning runtime record. Clones retain the membership and inverse allocations
/// independently of the loader. Only `view()` may be copied into device code.
#[derive(Clone)]
pub struct Domain {
    pub id: u32,
    pub minimum: i64,
    pub bound: u32,
    pub bits: u32,
    pub(crate) offset: KeyDomain,
    pub(crate) index: Option<std::sync::Arc<crate::rank::Index>>,
    pub(crate) distinct: u32,
}

/// Non-owning device descriptor; retain its `Domain` until stream completion.
/// Null bitmap/directory pointers mean Offset, which owns no device metadata.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DomainView {
    pub minimum: i64,
    pub bound: u32,
    pub span: u32,
    pub bitmap: *const u32,
    pub directory: *const u32,
}
unsafe impl cuda_core::DeviceCopy for DomainView {}

impl DomainView {
    /// # Safety
    /// Retain the owning domain and pass a valid code on its CUDA stream.
    #[inline(always)]
    pub unsafe fn value(self, code: u32) -> i64 {
        unsafe { domain_value(self, code) }
    }
}

/// Reconstruct a logical key at an R/P boundary.
///
/// # Safety
/// The owning domain must be alive, and `code < domain.bound`.
#[cuda_device::device]
#[inline(always)]
pub unsafe fn domain_value(domain: DomainView, code: u32) -> i64 {
    // DECODE ?: recorded Offset/Rank inverse requires a caller Rust-read or publication boundary.
    let offset = if domain.directory.is_null() {
        code
    } else {
        let words = (domain.span as usize).div_ceil(32);
        let mut low = 0;
        let mut high = words;
        // Upper bound handles empty bitmap words and repeated prefixes.
        while low < high {
            let mid = low + (high - low) / 2;
            if unsafe { domain.directory.add(mid).read() } <= code {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        let word = low - 1;
        let mut remaining = code - unsafe { domain.directory.add(word).read() };
        let mut bits = unsafe { domain.bitmap.add(word).read() };
        while remaining != 0 {
            bits &= bits - 1;
            remaining -= 1;
        }
        word as u32 * 32 + bits.trailing_zeros()
    };
    domain.minimum.wrapping_add(i64::from(offset))
}

/// Exact code or the reserved missing sentinel.
///
/// # Safety
/// The owning domain and its immutable GPU buffers must remain alive.
#[cuda_device::device]
#[inline(always)]
pub unsafe fn domain_exact(domain: DomainView, value: i64) -> u32 {
    if value < domain.minimum { return u32::MAX; }
    let offset = (value as u64).wrapping_sub(domain.minimum as u64);
    if offset >= domain.span as u64 { return u32::MAX; }
    let word = offset as usize / 32;
    if !domain.bitmap.is_null()
        && unsafe { domain.bitmap.add(word).read() } & (1 << (offset % 32)) == 0
    {
        return u32::MAX;
    }
    unsafe { domain_lower(domain, value) }
}

/// Ordered lower bound, including absent and out-of-range constants.
///
/// # Safety
/// The owning domain and its immutable GPU buffers must remain alive.
#[cuda_device::device]
#[inline(always)]
pub unsafe fn domain_lower(domain: DomainView, value: i64) -> u32 {
    if value < domain.minimum { return 0; }
    let offset = (value as u64).wrapping_sub(domain.minimum as u64);
    if offset >= domain.span as u64 { return domain.bound; }
    if domain.directory.is_null() { return offset as u32; }
    let word = offset as usize / 32;
    let rank = unsafe { domain.directory.add(word).read() };
    let before = (1u32 << (offset % 32)) - 1;
    rank + (unsafe { domain.bitmap.add(word).read() } & before).count_ones()
}

impl cq_storage::CodeDomain for Domain {
    type Value = i64;
    type Code = u32;
    fn exact(&self, value: &i64) -> u32 { Domain::exact(self, *value) }
}
impl cq_storage::OrderedCodeDomain for Domain {
    fn lower(&self, value: &i64) -> u32 { Domain::lower(self, *value) }
    fn upper(&self, value: &i64) -> u32 { Domain::upper(self, *value) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_threshold_is_inclusive_and_empty_stays_offset() {
        assert!(!use_rank(0, 0));
        assert!(!use_rank(1, 1));
        assert!(!use_rank(7, 4));
        assert!(use_rank(8, 4));
        assert!(use_rank(u32::MAX - 1, 1));
    }

    #[test]
    fn empty_and_singleton_domains() {
        let empty = KeyDomain::from_bounds(1, Bounds::Empty).unwrap();
        assert_eq!((empty.minimum, empty.bound, empty.bits), (0, 0, 0));
        assert_eq!(empty.exact(0), u32::MAX);
        assert_eq!((empty.lower(i64::MIN), empty.upper(i64::MAX)), (0, 0));
        let single = KeyDomain::from_bounds(2, Bounds::singleton(-9)).unwrap();
        assert_eq!((single.minimum, single.bound, single.bits), (-9, 1, 1));
        assert_eq!(single.value(single.exact(-9)), -9);
    }

    #[test]
    fn signed_edges_and_outside_operands_do_not_wrap() {
        let domain = KeyDomain::from_bounds(3, Bounds::Inclusive { min: -5, max: 10 }).unwrap();
        assert_eq!((domain.bound, domain.bits), (16, 4));
        for value in -5..=10 { assert_eq!(domain.value(domain.exact(value)), value); }
        for value in [i64::MIN, -6, 11, i64::MAX] { assert_eq!(domain.exact(value), u32::MAX); }
        assert_eq!((domain.lower(-5), domain.upper(-5)), (0, 1));
        assert_eq!((domain.lower(10), domain.upper(10)), (15, 16));
        assert_eq!((domain.lower(i64::MIN), domain.upper(i64::MIN)), (0, 0));
        assert_eq!((domain.lower(i64::MAX), domain.upper(i64::MAX)), (16, 16));
    }

    #[test]
    fn range_limit_and_extreme_minimum_are_checked() {
        let max = i64::from(u32::MAX) - 2;
        assert_eq!(KeyDomain::from_bounds(4, Bounds::Inclusive { min: 0, max }).unwrap().bits, 32);
        for bounds in [Bounds::Inclusive { min: 0, max: max + 1 }, Bounds::Inclusive { min: i64::MIN, max: i64::MAX }] {
            let error = KeyDomain::from_bounds(4, bounds).unwrap_err().to_string();
            assert!(error.contains("key domain 4") && error.contains("u32"));
        }
        let domain = KeyDomain::from_bounds(5, Bounds::Inclusive { min: i64::MIN, max: i64::MIN + 5 }).unwrap();
        assert_eq!(domain.value(5), i64::MIN + 5);
        assert_eq!(domain.upper(i64::MAX), 6);
    }
}
