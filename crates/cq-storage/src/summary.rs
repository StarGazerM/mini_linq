//! Conservative coordinate bounds; they never establish exact membership.

use crate::Range;

/// A conservative range summary of observed integer keys.
///
/// `Default` represents no observed keys and rejects every valid candidate.
/// `observe` adds one whole key without forgetting earlier keys. `may_match`
/// may return false only when none of the
/// observed keys can equal the projection, in `columns` order, of any row
/// enclosed by `bounds`. True permits false positives and requires an exact
/// residual check. Bounds must enclose the candidate payload; coordinate count
/// and indices must be valid. Implementations may panic on invalid arguments.
///
/// Zero-column observations still carry existence: an empty summary rejects,
/// while observing `[]` permits a match against any valid candidate bounds.
/// This contract describes range metadata, not a universal pruning interface.
pub trait RangeSummary<const K: usize>: Default {
    fn observe(&mut self, key: &[i64; K]);

    fn may_match(&self, bounds: &[(i64, i64)], columns: &[usize]) -> bool;
}

/// Bounds enclosing every observed whole key. Empty and nonempty zero-column
/// key collections remain distinct. False from `may_match` excludes all observed
/// keys from a block; true permits false positives and requires an exact check.
#[derive(Debug)]
pub struct ZoneSummary<const K: usize>(Option<[Range; K]>);

impl<const K: usize> Default for ZoneSummary<K> {
    fn default() -> Self {
        Self(None)
    }
}

impl<const K: usize> RangeSummary<K> for ZoneSummary<K> {
    fn observe(&mut self, key: &[i64; K]) {
        let ranges = self.0.get_or_insert_with(|| key.map(|v| (v, v)));
        for (range, &value) in ranges.iter_mut().zip(key) {
            range.0 = range.0.min(value);
            range.1 = range.1.max(value);
        }
    }

    /// `bounds` must enclose the payload and `columns` pair each key coordinate
    /// with a payload coordinate. BlockStore descriptors provide such bounds;
    /// any subset of the same immutable payload remains enclosed by them.
    fn may_match(&self, bounds: &[Range], columns: &[usize]) -> bool {
        assert_eq!(columns.len(), K, "zone summary key arity mismatch");
        assert!(columns.iter().all(|&c| c < bounds.len()));
        self.0.as_ref().is_some_and(|ranges| {
            columns.iter().zip(ranges).all(|(&c, &(lo, hi))| {
                let (block_lo, block_hi) = bounds[c];
                block_lo <= hi && lo <= block_hi
            })
        })
    }
}
