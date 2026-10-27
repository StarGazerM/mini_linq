//! Payload-exact clustered runs indexed by a radix directory and bounded-error spline.
//! The representation shares grouping/cursors with other run providers, not their
//! search algorithms. Payload lookup checks exact equality in the spline window;
//! dense-domain membership can use a bounded, lazily initialized presence bitmap.

use crate::radix_spline::{
    RadixSpline, RadixSplineMetadata, RadixSplinePoint, RadixSplineView,
};
use crate::rows::Rows;
use crate::sorted_run::RunRows;
use crate::{Build, ExactMembership, IndexRead, RunCursor};
use std::{mem::size_of, sync::OnceLock};

/// A bounded exact membership projection, initialized only if contains is used.
/// Payload lookup continues to use the spline; sparse domains allocate no bitmap.
struct DenseMembership {
    min: u64,
    span: usize,
    // Initialization needs the owning run's runtime keys, not a static initializer.
    bits: OnceLock<Box<[u64]>>,
}

impl DenseMembership {
    fn candidate<K: RadixKey>(keys: &[K]) -> Option<Self> {
        const MAX_BITS: u64 = 1 << 20;
        let min = keys.first()?.radix_key();
        let span = keys.last()?.radix_key().checked_sub(min)?.checked_add(1)?;
        if span > MAX_BITS {
            return None;
        }
        // Include word rounding: at most one extra byte per distinct key,
        // and no more than 128 KiB even for large relations.
        if span.div_ceil(64) as usize * size_of::<u64>() > keys.len() {
            return None;
        }
        Some(Self {
            min,
            span: span as usize,
            bits: OnceLock::new(),
        })
    }

    #[inline]
    fn contains<K: RadixKey>(&self, key: u64, keys: &[K]) -> bool {
        let Some(offset) = key
            .checked_sub(self.min)
            .filter(|offset| *offset < self.span as u64)
        else {
            return false;
        };
        let bits = self.bits.get_or_init(|| {
            let mut bits = vec![0_u64; self.span.div_ceil(64)];
            for key in keys {
                let offset = (key.radix_key() - self.min) as usize;
                bits[offset / 64] |= 1_u64 << (offset % 64);
            }
            bits.into_boxed_slice()
        });
        bits[offset as usize / 64] & (1_u64 << (offset % 64)) != 0
    }
}

/// An injective, order-preserving encoding of the entire key domain into `u64`.
///
/// Implementations must satisfy `a.cmp(b) == a.radix_key().cmp(&b.radix_key())`.
/// Unlike a partial dense codec, this contract must cover absent query keys too.
/// Borrowed and singleton-tuple keys preserve their underlying key's encoding.
/// Wider/composite domains require a different provider; they are not truncated.
pub trait RadixKey: Ord + Copy {
    fn radix_key(&self) -> u64;
}

impl RadixKey for u64 {
    #[inline]
    fn radix_key(&self) -> u64 {
        *self
    }
}
impl RadixKey for u32 {
    #[inline]
    fn radix_key(&self) -> u64 {
        u64::from(*self)
    }
}
impl RadixKey for i64 {
    #[inline]
    fn radix_key(&self) -> u64 {
        (*self as u64) ^ (1 << 63)
    }
}
impl RadixKey for i32 {
    #[inline]
    fn radix_key(&self) -> u64 {
        u64::from((*self as u32) ^ (1 << 31))
    }
}
impl<T: RadixKey> RadixKey for &T {
    #[inline]
    fn radix_key(&self) -> u64 {
        T::radix_key(*self)
    }
}
impl<T: RadixKey> RadixKey for (T,) {
    #[inline]
    fn radix_key(&self) -> u64 {
        self.0.radix_key()
    }
}

/// Reusable CPU read state for one immutable, strictly increasing key array.
///
/// Construction examines only the endpoints and key count. Dense membership
/// uses the same bounded policy as [`RadixRun`], and allocates its bitmap only
/// on the first in-domain membership probe. Payload probes never initialize it.
pub struct RadixRunReadState {
    membership: Option<DenseMembership>,
}

impl RadixRunReadState {
    pub fn new<K: RadixKey>(keys: &[K]) -> Self {
        Self {
            membership: DenseMembership::candidate(keys),
        }
    }
}

/// Allocation-free native reads over borrowed grouped payloads and spline data.
///
/// This view does not own or build its arrays. In particular, completed GPU
/// arrays can use the exact same reader as a CPU-built [`RadixRun`]. Copies of
/// the view and its cursors retain the arrays' lifetime, not the view's lifetime.
pub struct RadixRunView<'a, K, V> {
    keys: &'a [K],
    offsets: &'a [usize],
    items: &'a [V],
    directory: RadixSplineView<'a>,
    state: &'a RadixRunReadState,
}

impl<K, V> Copy for RadixRunView<'_, K, V> {}

impl<K, V> Clone for RadixRunView<'_, K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, K: RadixKey, V: Copy> RadixRunView<'a, K, V> {
    /// Borrow an already completed grouped run without validating or copying it.
    ///
    /// # Safety
    ///
    /// All arrays must be fully initialized and remain immutable for `'a`,
    /// including with respect to device writes. Keys must be strictly increasing
    /// under both `Ord` and their radix encoding. Offsets must have
    /// `keys.len() + 1` entries, start at zero, increase for each key's nonempty
    /// payload group, and end at `items.len()`. Payload order within each group
    /// must preserve the represented occurrence order.
    ///
    /// Metadata must describe these keys (`len` is the distinct-key count),
    /// with `shift < 64` and `search_error + 2` representable as `usize`.
    /// For nonempty keys, points must include the first and last key/rank,
    /// be strictly increasing in key and rank, and bound interpolation error
    /// (including floating-point rounding) by `search_error`. The radix directory
    /// must contain the first point at or after each min-relative shifted prefix,
    /// plus its terminal entry; all entries must be valid point boundaries.
    /// Empty runs need no points or directory entries.
    ///
    /// `state` must have been created from the same keys and must never have
    /// been used with a different key array. No structural checks are performed
    /// here or on a probe.
    pub unsafe fn from_parts(
        keys: &'a [K],
        offsets: &'a [usize],
        items: &'a [V],
        points: &'a [RadixSplinePoint],
        radix: &'a [usize],
        metadata: RadixSplineMetadata,
        state: &'a RadixRunReadState,
    ) -> Self {
        Self {
            keys,
            offsets,
            items,
            directory: RadixSplineView { points, radix, metadata },
            state,
        }
    }

    pub fn keys(&self) -> &'a [K] {
        self.keys
    }

    pub fn items(&self) -> &'a [V] {
        self.items
    }

    /// Exact half-open payload range; missing keys return the empty terminal range.
    #[inline]
    pub fn range(&self, key: &K) -> (usize, usize) {
        self.slot(key).map_or((self.items.len(), self.items.len()), |slot| {
            (self.offsets[slot], self.offsets[slot + 1])
        })
    }

    #[inline]
    pub fn lookup(&self, key: &K) -> RunCursor<'a, V> {
        crate::count!(lookups);
        let (begin, end) = self.range(key);
        RunCursor::from_slice(&self.items[begin..end])
    }

    #[inline]
    pub fn contains(&self, key: &K) -> bool {
        crate::count!(lookups);
        match &self.state.membership {
            Some(membership) => membership.contains(key.radix_key(), self.keys),
            None => self.slot(key).is_some(),
        }
    }

    #[inline]
    fn slot(&self, key: &K) -> Option<usize> {
        let bound = self.directory.search_bound(key.radix_key());
        self.keys[bound.clone()]
            .binary_search(key)
            .ok()
            .map(|i| bound.start + i)
    }
}

/// Stable clustered payload groups with a RadixSpline lookup directory.
///
/// Uses 12 radix bits and a 32-position spline error bound. The spline routes to
/// an exact search window; it is neither membership evidence nor a lossy codec.
/// Equal-key payloads retain input order and multiplicity. Copy range cursors
/// allow later expansion without another directory lookup or row materialization.
/// Dense membership reads lazily initialize an exact bounded bitmap; payload
/// reads keep using the spline. Payload-only and sparse uses allocate no bitmap.
/// Construction and reads are CPU operations; this type makes no GPU capability claim.
pub struct RadixRun<K, V> {
    rows: RunRows<K, V>,
    directory: RadixSpline,
    state: RadixRunReadState,
}

impl<K, V> crate::Keys for RadixRun<K, V> {
    type Key = K;
    type Iter<'a> = std::slice::Iter<'a, K> where Self: 'a;

    fn key_count(&self) -> usize {
        self.rows.keys.len()
    }

    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.rows.keys.iter()
    }
}

impl<K: RadixKey, V: Copy> RadixRun<K, V> {
    /// Formal rule STR-RadixRun:
    /// Stable grouping preserves occurrences; an injective ordered key encoding
    /// and an error-bounded spline window followed by exact search select the
    /// same payload range, including when the query key is absent.
    pub fn from_pairs(source: impl IntoIterator<Item = (K, V)>) -> Self {
        let rows = RunRows::from_pairs_cached_key(source, RadixKey::radix_key);
        let directory = RadixSpline::build(rows.keys.len(), |i| rows.keys[i].radix_key());
        let state = RadixRunReadState::new(&rows.keys);
        Self {
            rows,
            directory,
            state,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.keys.is_empty()
    }

    /// Unique keys in increasing order, without enumerating payloads.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.rows.keys.iter()
    }

    /// All payloads, grouped by key; equal-key payloads retain their source order.
    pub fn items(&self) -> &[V] {
        &self.rows.items
    }

    /// Exact half-open payload range; an absent key returns `(items.len(), items.len())`.
    #[inline]
    pub fn range(&self, key: &K) -> (usize, usize) {
        self.as_view().range(key)
    }

    #[inline]
    fn as_view(&self) -> RadixRunView<'_, K, V> {
        RadixRunView {
            keys: &self.rows.keys,
            offsets: &self.rows.offsets,
            items: &self.rows.items,
            directory: self.directory.as_view(),
            state: &self.state,
        }
    }
}

impl<K: RadixKey, V: Copy, I: Iterator<Item = (K, V)>> Build<I> for RadixRun<K, V> {
    fn build(source: I) -> Self {
        Self::from_pairs(source)
    }
}

impl<'a, K: RadixKey, V: Copy> IndexRead for &'a RadixRun<K, V> {
    type Key = K;
    type Item = V;
    type Cursor = RunCursor<'a, V>;

    #[inline]
    fn lookup(&self, key: &K) -> Self::Cursor {
        self.as_view().lookup(key)
    }
}

impl<K: RadixKey, V: Copy> ExactMembership<K> for RadixRun<K, V> {
    #[inline]
    fn contains(&self, key: &K) -> bool {
        self.as_view().contains(key)
    }
}

impl<K: Copy, V: Copy> Rows for &RadixRun<K, V> {
    type Row = (K, V);
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.rows.rows()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_grouped_view_preserves_payloads_and_lazy_membership() {
        let keys: Vec<u32> = (0..96).map(|rank| 64 + 2 * rank).collect();
        let offsets: Vec<usize> = (0..=96).map(|rank| 2 * rank).collect();
        let items: Vec<u32> = (0..96).flat_map(|rank| [rank, 1000 + rank]).collect();
        // Exact collinear endpoints with a consecutive-rank chunk bridge.
        // This need not be the segmentation the native greedy builder chooses.
        let points = [
            RadixSplinePoint { key: 64, rank: 0 },
            RadixSplinePoint { key: 128, rank: 32 },
            RadixSplinePoint { key: 130, rank: 33 },
            RadixSplinePoint { key: 254, rank: 95 },
        ];
        let radix: Vec<usize> = (0..192)
            .map(|prefix| points.partition_point(|point| point.key < 64 + prefix))
            .collect();
        let metadata = RadixSplineMetadata {
            min_key: 64,
            max_key: 254,
            len: keys.len(),
            shift: 0,
            search_error: 33,
        };
        let state = RadixRunReadState::new(&keys);
        let membership = state.membership.as_ref().expect("bounded dense domain");
        let saved = {
            // SAFETY: the arrays above are immutable, exactly grouped, and the
            // collinear spline/directory describe these keys with zero error.
            let view = unsafe {
                RadixRunView::from_parts(
                    &keys, &offsets, &items, &points, &radix, metadata, &state,
                )
            };
            assert_eq!(view.keys(), keys);
            assert_eq!(view.items(), items);
            for key in 0..300 {
                let expected: Vec<_> = keys
                    .binary_search(&key)
                    .ok()
                    .map(|rank| items[2 * rank..2 * rank + 2].to_vec())
                    .unwrap_or_default();
                let cursor = view.lookup(&key);
                assert_eq!(cursor.collect::<Vec<_>>(), expected);
                let (begin, end) = view.range(&key);
                assert_eq!(&items[begin..end], expected);
                let mut advanced = cursor;
                assert_eq!(advanced.nth(1), expected.get(1).copied());
                assert_eq!(cursor.collect::<Vec<_>>(), expected);
            }
            assert!(!view.contains(&63));
            assert!(!view.contains(&255));
            assert!(membership.bits.get().is_none());
            let saved = view.lookup(&128);
            let copy = view;
            for key in 0..300 {
                assert_eq!(copy.contains(&key), keys.binary_search(&key).is_ok());
            }
            assert!(membership.bits.get().is_some());
            saved
        };
        assert_eq!(saved.collect::<Vec<_>>(), [32, 1032]);
    }

    #[test]
    fn borrowed_views_cover_empty_singleton_and_sparse_full_domain() {
        for keys in [
            Vec::new(),
            vec![u64::MAX],
            vec![0, 1 << 63, u64::MAX],
        ] {
            let offsets: Vec<_> = (0..=keys.len()).collect();
            let items: Vec<_> = (0..keys.len()).collect();
            let spline = RadixSpline::build(keys.len(), |rank| keys[rank]);
            let directory = spline.as_view();
            let state = RadixRunReadState::new(&keys);
            assert!(state.membership.is_none());
            // SAFETY: one payload per ordered key; the native builder supplies
            // the matching valid spline and the state uses this same key array.
            let view = unsafe {
                RadixRunView::from_parts(
                    &keys,
                    &offsets,
                    &items,
                    directory.points,
                    directory.radix,
                    directory.metadata,
                    &state,
                )
            };
            for key in [0, 1, (1 << 63) - 1, 1 << 63, u64::MAX - 1, u64::MAX] {
                let slot = keys.binary_search(&key).ok();
                assert_eq!(
                    view.lookup(&key).collect::<Vec<_>>(),
                    slot.into_iter().collect::<Vec<_>>(),
                );
                assert_eq!(view.contains(&key), slot.is_some());
                assert_eq!(
                    view.range(&key),
                    slot.map_or((items.len(), items.len()), |rank| (rank, rank + 1)),
                );
            }
        }
    }

    #[test]
    fn cached_numeric_sort_preserves_borrowed_duplicate_order() {
        let ascending: Vec<_> = (0_i64..257).map(|i| (i / 3, i)).collect();
        let reversed: Vec<_> = ascending.iter().copied().rev().collect();
        let unique_reverse: Vec<_> = (0_i64..257).rev().map(|i| (i, i * 2)).collect();
        let scrambled: Vec<_> = (0_i64..257).map(|i| ((i * 73 % 257) / 3, i)).collect();
        for input in [ascending, reversed, unique_reverse, scrambled] {
            let run = RadixRun::from_pairs(input.iter().map(|(key, value)| ((key,), *value)));
            let mut expected = input.clone();
            expected.sort_by_key(|(key, _)| *key);
            assert_eq!(
                (&run)
                    .rows()
                    .map(|((key,), value)| (*key, value))
                    .collect::<Vec<_>>(),
                expected
            );
            for key in [-1, 0, 42, 85, 256, 257] {
                let expected: Vec<_> = input
                    .iter()
                    .filter_map(|(k, value)| (*k == key).then_some(*value))
                    .collect();
                assert_eq!((&run).lookup(&(&key,)).collect::<Vec<_>>(), expected);
            }
        }
    }

    #[test]
    fn dense_membership_is_lazy_exact_and_safe_for_concurrent_readers() {
        let run = RadixRun::from_pairs((0_u64..96).map(|i| (64 + 2 * i, i)));
        let membership = run.state.membership.as_ref().expect("bounded dense domain");
        let saved = (&run).lookup(&64);
        assert_eq!(saved.collect::<Vec<_>>(), [0]);
        assert!(!run.contains(&63));
        assert!(!run.contains(&255));
        assert!(
            membership.bits.get().is_none(),
            "payload/out-of-domain reads must not allocate the projection"
        );
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for key in 0..300 {
                        assert_eq!(
                            run.contains(&key),
                            (64..=254).contains(&key) && key % 2 == 0
                        );
                    }
                });
            }
        });
        assert!(membership.bits.get().is_some());
        assert_eq!(
            saved.collect::<Vec<_>>(),
            [0],
            "membership initialization cannot invalidate a payload cursor"
        );

        let high: Vec<_> = (0_u64..96).map(|i| u64::MAX - 190 + 2 * i).collect();
        let run = RadixRun::from_pairs(high.iter().copied().map(|key| (key, key)));
        for key in [
            0,
            u64::MAX - 191,
            u64::MAX - 190,
            u64::MAX - 189,
            u64::MAX - 64,
            u64::MAX - 1,
            u64::MAX,
        ] {
            assert_eq!(run.contains(&key), high.contains(&key));
        }
    }

    #[test]
    fn sparse_membership_keeps_bounded_storage_and_exact_spline_lookup() {
        for step in [1_000_u64, 100_000] {
            let keys: Vec<_> = (0..64).map(|i| i * step).collect();
            let run = RadixRun::from_pairs(keys.iter().copied().map(|key| (key, key)));
            assert!(
                run.state.membership.is_none(),
                "sparse domains must not reserve a large presence bitmap"
            );
            for key in keys.iter().flat_map(|key| [*key, key + 1]) {
                assert_eq!(run.contains(&key), keys.contains(&key));
                let expected: Vec<_> = keys
                    .iter()
                    .copied()
                    .filter(|candidate| *candidate == key)
                    .collect();
                assert_eq!((&run).lookup(&key).collect::<Vec<_>>(), expected);
            }
        }
    }

    #[test]
    fn stable_payload_ranges_cover_extremes_holes_and_duplicate_occurrences() {
        let input = [
            (i64::MAX, 7),
            (-9, 1),
            (0, 2),
            (-9, 3),
            (-9, 1),
            (i64::MIN, 4),
        ];
        let run = RadixRun::from_pairs(input);
        for key in [
            i64::MIN,
            i64::MIN + 1,
            -10,
            -9,
            -8,
            0,
            1,
            i64::MAX - 1,
            i64::MAX,
        ] {
            let expected: Vec<_> = input
                .iter()
                .filter(|(k, _)| *k == key)
                .map(|(_, v)| *v)
                .collect();
            let cursor = (&run).lookup(&key);
            assert_eq!(cursor.collect::<Vec<_>>(), expected);
            assert_eq!(run.contains(&key), !expected.is_empty());
            let (begin, end) = run.range(&key);
            assert_eq!(&run.items()[begin..end], expected.as_slice());
            let mut advanced = cursor;
            assert_eq!(advanced.nth(1), expected.get(1).copied());
            assert_eq!(cursor.collect::<Vec<_>>(), expected);
        }
        assert_eq!(
            run.keys().copied().collect::<Vec<_>>(),
            [i64::MIN, -9, 0, i64::MAX]
        );
        let mut expected = input.to_vec();
        expected.sort_by_key(|(key, _)| *key);
        assert_eq!((&run).rows().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn borrowed_tuple_and_unsigned_keys_preserve_full_domain() {
        let keys = [0_u64, 1, 1 << 63, u64::MAX];
        let run = RadixRun::from_pairs(keys.iter().enumerate().map(|(i, k)| ((k,), i)));
        for (i, key) in keys.iter().enumerate() {
            assert_eq!((&run).lookup(&(key,)).collect::<Vec<_>>(), [i]);
        }
        assert!(!run.contains(&(&2,)));
        let signed = [i32::MIN, -1, 0, i32::MAX];
        let run = RadixRun::from_pairs(signed.iter().map(|k| ((k,), *k)));
        for key in &signed {
            assert_eq!((&run).lookup(&(key,)).collect::<Vec<_>>(), [*key]);
        }
        let run = RadixRun::from_pairs([(u32::MAX, 1), (0_u32, 2)]);
        assert_eq!((&run).lookup(&u32::MAX).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn empty_and_singleton_groups_have_exact_missing_ranges() {
        let empty = RadixRun::<u64, u64>::from_pairs([]);
        assert!(empty.is_empty());
        assert_eq!(empty.range(&0), (0, 0));
        assert!(!empty.contains(&u64::MAX));
        let single = RadixRun::from_pairs([(u64::MAX, 1), (u64::MAX, 2)]);
        assert_eq!(single.len(), 1);
        assert_eq!(single.range(&u64::MAX), (0, 2));
        assert_eq!(single.range(&(u64::MAX - 1)), (2, 2));
    }
}
