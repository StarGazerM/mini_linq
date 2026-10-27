// Native adaptation of RadixSpline's builder.h and radix_spline.h, revision
// ab96aa59d429e7423beba2350bdcdf88952df282 (radix12/error32).
//
// The MIT License
//
// Copyright (c) 2020 Andreas Kipf, Alexander van Renen, Mihail Stoian
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

use std::cmp::Ordering;
use std::ops::Range;

const RADIX_BITS: u32 = 12;
const MAX_ERROR: usize = 32;

/// One interpolation endpoint, with its rank in the distinct-key array.
///
/// The C layout permits borrowing initialized GPU-produced endpoint arrays.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RadixSplinePoint {
    pub key: u64,
    pub rank: usize,
}

/// Scalar parameters for the native radix-spline reader.
///
/// `len` counts distinct keys, not payload occurrences. `search_error` includes
/// the interpolation rounding guard as well as the builder's corridor error.
#[derive(Clone, Copy, Debug)]
pub struct RadixSplineMetadata {
    pub min_key: u64,
    pub max_key: u64,
    pub len: usize,
    pub shift: u32,
    pub search_error: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct RadixSplineView<'a> {
    pub(crate) points: &'a [RadixSplinePoint],
    pub(crate) radix: &'a [usize],
    pub(crate) metadata: RadixSplineMetadata,
}

// Keeping differences integral until conversion is essential near u64::MAX and
// above 2^53. In particular, subtracting converted absolute keys loses spacing.
#[derive(Clone, Copy)]
struct Vector {
    x: u64,
    y: i128,
}

impl Vector {
    fn between(origin: RadixSplinePoint, point: RadixSplinePoint, error: i128) -> Self {
        Self {
            x: point.key - origin.key,
            y: point.rank as i128 - origin.rank as i128 + error,
        }
    }
}

// Positive means clockwise, just as in the upstream GreedySplineCorridor.
// The common case uses upstream's nested FMA. Near a cancellation, use the
// exact integral determinant instead of accepting a rounded corridor violation.
// Each unsigned product needs at most 129 bits (u64 key difference times a
// usize rank difference plus 32); (carry, low) represents it without allocation.
fn orientation(a: Vector, b: Vector) -> Ordering {
    let ax = a.x as f64;
    let ay = a.y as f64;
    let bx = b.x as f64;
    let by = b.y as f64;
    let product = by.mul_add(ax, 0.0);
    let determinant = ay.mul_add(bx, -product);
    let uncertainty = 4.0 * f64::EPSILON * ((ay * bx).abs() + product.abs());
    if determinant > uncertainty {
        return Ordering::Greater;
    }
    if determinant < -uncertainty {
        return Ordering::Less;
    }

    let a_negative = a.y < 0 && b.x != 0;
    let b_negative = b.y < 0 && a.x != 0;
    if a_negative != b_negative {
        return if a_negative {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let (a_low, a_carry) = a.y.unsigned_abs().overflowing_mul(u128::from(b.x));
    let (b_low, b_carry) = b.y.unsigned_abs().overflowing_mul(u128::from(a.x));
    let order = (a_carry, a_low).cmp(&(b_carry, b_low));
    if a_negative { order.reverse() } else { order }
}

/// Radix directory over an error-bounded piecewise-linear rank spline.
///
/// Input keys must be strictly increasing. Ranks remain `usize`; no copy of
/// the input key array is retained. The exact corridor bounds mathematical
/// interpolation by 32 ranks. IEEE-754 interpolation has at most four epsilon
/// times the rank span of absolute rounding error; `search_error` includes its
/// ceiling (one additional rank for ordinary-sized arrays). Thus correctness
/// does not assume absolute keys or ranks fit exactly in an f64 significand.
#[derive(Debug)]
pub(crate) struct RadixSpline {
    min_key: u64,
    max_key: u64,
    len: usize,
    shift: u32,
    search_error: usize,
    radix: Vec<usize>,
    points: Vec<RadixSplinePoint>,
}

impl RadixSpline {
    pub(crate) fn build(n: usize, key_at: impl Fn(usize) -> u64) -> Self {
        if n == 0 {
            return Self {
                min_key: 0,
                max_key: 0,
                len: 0,
                shift: 0,
                search_error: MAX_ERROR,
                radix: Vec::new(),
                points: Vec::new(),
            };
        }
        let min_key = key_at(0);
        let max_key = if n == 1 { min_key } else { key_at(n - 1) };
        // leading_zeros is defined for zero, unlike the upstream clz builtin.
        let shift = (u64::BITS - (max_key - min_key).leading_zeros()).saturating_sub(RADIX_BITS);
        let mut points = vec![RadixSplinePoint {
            key: min_key,
            rank: 0,
        }];
        if n > 1 {
            let mut previous = RadixSplinePoint {
                key: key_at(1),
                rank: 1,
            };
            debug_assert!(previous.key > min_key);
            let mut upper = previous;
            let mut lower = previous;
            for rank in 2..n {
                let current = RadixSplinePoint {
                    key: key_at(rank),
                    rank,
                };
                debug_assert!(current.key > previous.key);
                let last = points[points.len() - 1];
                let upper_vector = Vector::between(last, upper, MAX_ERROR as i128);
                let lower_vector =
                    Vector::between(last, lower, -(lower.rank.min(MAX_ERROR) as i128));
                let current_vector = Vector::between(last, current, 0);
                if orientation(upper_vector, current_vector) != Ordering::Greater
                    || orientation(lower_vector, current_vector) != Ordering::Less
                {
                    points.push(previous);
                    upper = current;
                    lower = current;
                } else {
                    if orientation(
                        upper_vector,
                        Vector::between(last, current, MAX_ERROR as i128),
                    ) == Ordering::Greater
                    {
                        upper = current;
                    }
                    if orientation(
                        lower_vector,
                        Vector::between(last, current, -(rank.min(MAX_ERROR) as i128)),
                    ) == Ordering::Less
                    {
                        lower = current;
                    }
                }
                previous = current;
            }
            // The previous point is never already a spline endpoint here.
            points.push(previous);
        }

        // A bucket contains the first spline point at/after its prefix. Its
        // successor also serves as the endpoint when a bucket has no points.
        let max_prefix = ((max_key - min_key) >> shift) as usize;
        let mut radix = vec![0; max_prefix + 2];
        let mut previous_prefix = 0;
        for (index, point) in points.iter().enumerate() {
            let prefix = ((point.key - min_key) >> shift) as usize;
            if prefix != previous_prefix {
                radix[previous_prefix + 1..=prefix].fill(index);
                previous_prefix = prefix;
            }
        }
        radix[previous_prefix + 1..].fill(points.len());
        let rounding_guard = (n as f64 * (4.0 * f64::EPSILON)).ceil() as usize;
        Self {
            min_key,
            max_key,
            len: n,
            shift,
            search_error: MAX_ERROR + rounding_guard,
            radix,
            points,
        }
    }

    #[inline]
    pub(crate) fn as_view(&self) -> RadixSplineView<'_> {
        RadixSplineView {
            points: &self.points,
            radix: &self.radix,
            metadata: RadixSplineMetadata {
                min_key: self.min_key,
                max_key: self.max_key,
                len: self.len,
                shift: self.shift,
                search_error: self.search_error,
            },
        }
    }

    #[cfg(test)]
    fn search_bound(&self, key: u64) -> Range<usize> {
        self.as_view().search_bound(key)
    }
}

impl RadixSplineView<'_> {
    /// The exact lower bound is in `start..=end`; an existing key is in
    /// `start..end`. Searching this slice therefore also handles absent keys.
    #[inline]
    pub(crate) fn search_bound(&self, key: u64) -> Range<usize> {
        let RadixSplineMetadata { min_key, max_key, len, search_error, .. } = self.metadata;
        if len == 0 {
            return 0..0;
        }
        if key <= min_key {
            return 0..1;
        }
        if key >= max_key {
            return if key == max_key {
                len - 1..len
            } else {
                len..len
            };
        }
        let index = self.segment(key);
        let down = self.points[index - 1];
        let up = self.points[index];
        let rank_span = up.rank - down.rank;
        let slope = rank_span as f64 / (up.key - down.key) as f64;
        let offset = ((key - down.key) as f64).mul_add(slope, 0.0) as usize;
        // Add the integral anchor after interpolation, retaining exact ranks
        // even when an absolute rank would no longer fit in the significand.
        let estimate = down.rank + offset.min(rank_span);
        estimate.saturating_sub(search_error)
            ..estimate.saturating_add(search_error + 2).min(len)
    }

    #[inline]
    fn segment(&self, key: u64) -> usize {
        let prefix = ((key - self.metadata.min_key) >> self.metadata.shift) as usize;
        let begin = self.radix[prefix];
        let end = self.radix[prefix + 1];
        if end - begin < 32 {
            let mut current = begin;
            while current < end && self.points[current].key < key {
                current += 1;
            }
            current
        } else {
            begin + self.points[begin..end].partition_point(|point| point.key < key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_ERROR, RadixSpline, Vector, orientation};
    use std::cmp::Ordering;

    fn check_query(keys: &[u64], spline: &RadixSpline, key: u64) {
        let range = spline.search_bound(key);
        let expected = keys.partition_point(|candidate| *candidate < key);
        assert!(range.start <= range.end && range.end <= keys.len());
        assert!(
            range.start <= expected && expected <= range.end,
            "query {key}: {range:?} excludes insertion rank {expected}"
        );
        let found = range.start + keys[range.clone()].partition_point(|candidate| *candidate < key);
        assert_eq!(found, expected, "query {key}: {range:?}");
        if keys.get(expected) == Some(&key) {
            assert!(
                expected < range.end,
                "query {key}: {range:?} excludes existing key"
            );
        }
        assert!(range.len() <= 2 * (MAX_ERROR + 1) + 2);
    }

    fn check_keys(keys: &[u64]) -> RadixSpline {
        let spline = RadixSpline::build(keys.len(), |i| keys[i]);
        check_query(keys, &spline, 0);
        check_query(keys, &spline, u64::MAX);
        for &key in keys {
            check_query(keys, &spline, key);
            check_query(keys, &spline, key.saturating_sub(1));
            check_query(keys, &spline, key.saturating_add(1));
        }
        for pair in keys.windows(2) {
            check_query(keys, &spline, pair[0] + (pair[1] - pair[0]) / 2);
        }
        spline
    }

    #[test]
    fn empty_singleton_and_all_small_subsets() {
        let empty = RadixSpline::build(0, |_| panic!("empty input must not be read"));
        assert_eq!(empty.search_bound(0), 0..0);
        assert_eq!(empty.search_bound(u64::MAX), 0..0);
        check_keys(&[0]);
        check_keys(&[u64::MAX]);
        check_keys(&[0, u64::MAX]);
        for mask in 0u16..(1 << 10) {
            let keys: Vec<_> = (0u64..10).filter(|i| mask & (1 << i) != 0).collect();
            let spline = check_keys(&keys);
            for key in 0..=11 {
                check_query(&keys, &spline, key);
            }
        }
    }

    #[test]
    fn regular_spacing_large_offsets_and_skew() {
        for step in [1, 3, 1 << 32, (u64::MAX / 8192)] {
            let keys: Vec<_> = (0..8192).map(|i| i * step).collect();
            check_keys(&keys);
            let shifted: Vec<_> = keys.iter().map(|key| u64::MAX - keys[8191] + key).collect();
            check_keys(&shifted);
        }
        let squares: Vec<_> = (0u64..16384).map(|i| i * i * i).collect();
        check_keys(&squares);
        let mut powers = vec![0];
        for bit in 0..64 {
            let key = 1u64 << bit;
            powers.push(key);
            if bit > 1 {
                powers.push(key + 1);
            }
        }
        powers.push(u64::MAX);
        check_keys(&powers);
    }

    #[test]
    fn deterministic_random_full_domain() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut keys: Vec<_> = (0..16384).map(|_| next()).collect();
        keys.extend([
            0,
            1,
            (1 << 53) - 1,
            1 << 53,
            (1 << 53) + 1,
            u64::MAX - 1,
            u64::MAX,
        ]);
        keys.sort_unstable();
        keys.dedup();
        let spline = check_keys(&keys);
        for _ in 0..32768 {
            check_query(&keys, &spline, next());
        }
    }

    #[test]
    fn crowded_radix_bucket_and_large_empty_gaps() {
        let mut keys = Vec::new();
        for block in 0u64..512 {
            for offset in 0..96 {
                keys.push(block * 1_000_000 + offset);
            }
        }
        keys.push(u64::MAX);
        let spline = check_keys(&keys);
        // Exercise binary search over spline points, not the underlying keys.
        assert!(spline.radix[1] - spline.radix[0] >= 32);
        for prefix in 0..4096u64 {
            check_query(&keys, &spline, prefix << 52);
        }
    }

    #[test]
    fn determinant_cancellation_uses_exact_sign() {
        let big = u64::MAX;
        let a = Vector {
            x: big,
            y: big as i128 - 1,
        };
        let b = Vector {
            x: big - 1,
            y: big as i128 - 2,
        };
        assert_eq!(orientation(a, b), Ordering::Greater);
        assert_eq!(orientation(b, a), Ordering::Less);
        assert_eq!(orientation(a, a), Ordering::Equal);
        let a = Vector {
            x: big,
            y: big as i128 + 32,
        };
        let b = Vector {
            x: big - 1,
            y: big as i128 + 31,
        };
        assert_eq!(orientation(a, b), Ordering::Less);
        assert_eq!(orientation(b, a), Ordering::Greater);
        assert_eq!(orientation(a, Vector { x: 1, y: -1 }), Ordering::Greater);
    }
}
