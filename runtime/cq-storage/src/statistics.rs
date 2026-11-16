//! Statistics the runtime keeps of the relations it stores (PLANNING-REDESIGN,
//! PS-Admit): they are updated as rows are admitted, from the rows being
//! admitted, and keep sketches and samples, never the rows.

mod degrees;
pub use degrees::{Degrees, Reach, Request, Rows};

/// The hash of one cell: the 64-bit finalizer of MurmurHash3.
#[inline]
pub fn hash(value: i64) -> u64 {
    let mut x = value as u64;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// A distinct-count sketch: HyperLogLog over `M` one-byte registers (`M` a
/// power of two, at least 16), estimated as in Ertl, "New cardinality
/// estimation algorithms for HyperLogLog sketches" (arXiv:1702.01284,
/// Algorithm 6). Its relative error is about `1.04 / sqrt(M)`; sketches of
/// one `M` merge by the larger register.
#[derive(Clone)]
pub struct Sketch<const M: usize>([u8; M]);

impl<const M: usize> Default for Sketch<M> {
    fn default() -> Self {
        Self([0; M])
    }
}

impl<const M: usize> Sketch<M> {
    const PRECISION: u32 = M.trailing_zeros();

    /// Count a value by its hash; whether the sketch changed.
    #[inline]
    pub fn insert(&mut self, hash: u64) -> bool {
        let register = (hash & (M as u64 - 1)) as usize;
        let rest = (hash >> Self::PRECISION) | (1 << (64 - Self::PRECISION));
        let rank = rest.trailing_zeros() as u8 + 1;
        let grows = rank > self.0[register];
        if grows {
            self.0[register] = rank;
        }
        grows
    }

    pub fn merge(&mut self, other: &Self) {
        for (mine, theirs) in self.0.iter_mut().zip(&other.0) {
            *mine = (*mine).max(*theirs);
        }
    }

    /// The estimated number of distinct values counted.
    pub fn count(&self) -> u64 {
        let q = (64 - Self::PRECISION) as usize;
        let mut histogram = [0u32; 66];
        for &register in &self.0 {
            histogram[register as usize] += 1;
        }
        let m = M as f64;
        let mut z = m * tau(1.0 - f64::from(histogram[q + 1]) / m);
        for rank in (1..=q).rev() {
            z = 0.5 * (z + f64::from(histogram[rank]));
        }
        z += m * sigma(f64::from(histogram[0]) / m);
        // 1 / (2 ln 2)
        (0.721_347_520_444_481_7 * m * m / z).round() as u64
    }
}

fn sigma(mut x: f64) -> f64 {
    if x == 1.0 {
        return f64::INFINITY;
    }
    let (mut y, mut z) = (1.0, x);
    loop {
        x *= x;
        let before = z;
        z += x * y;
        y += y;
        if before == z {
            return z;
        }
    }
}

fn tau(mut x: f64) -> f64 {
    if x == 0.0 || x == 1.0 {
        return 0.0;
    }
    let (mut y, mut z) = (1.0, 1.0 - x);
    loop {
        x = x.sqrt();
        let before = z;
        y *= 0.5;
        z -= (1.0 - x) * (1.0 - x) * y;
        if before == z {
            return z / 3.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sketch_counts_within_its_error() {
        for distinct in [0u64, 1, 7, 100, 5_000, 300_000] {
            let mut sketch = Sketch::<4096>::default();
            // Every value three times: repeats do not count.
            for value in (0..distinct).chain(0..distinct).chain(0..distinct) {
                sketch.insert(hash(value as i64 * 7919 - 13));
            }
            let count = sketch.count() as f64;
            let error = if distinct == 0 { count } else { (count - distinct as f64).abs() / distinct as f64 };
            // 1.04 / sqrt(4096) is 1.6%; four standard errors.
            assert!(error <= 0.065, "{distinct} counted as {count}");
        }
    }

    #[test]
    fn merged_sketches_count_the_union() {
        let (mut left, mut right, mut both) = (Sketch::<256>::default(), Sketch::<256>::default(), Sketch::<256>::default());
        for value in 0..4_000i64 {
            let target = if value % 2 == 0 { &mut left } else { &mut right };
            target.insert(hash(value));
            both.insert(hash(value));
        }
        left.merge(&right);
        assert_eq!(left.count(), both.count());
    }
}
