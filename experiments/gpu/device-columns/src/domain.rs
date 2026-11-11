//! Load-recorded integer identity, separate from each column's FOR frames.
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

#[cfg(test)]
mod tests {
    use super::*;

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
