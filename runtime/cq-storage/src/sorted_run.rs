//! Payload-exact clustered runs. Binary search is the default directory;
//! a total, injective, order-preserving `KeyCodec` enables bitmap rank when
//! the checked inclusive span is at most 1,048,576 bits. On 64-bit hosts the
//! bitmap and per-word usize prefixes together use at most 256 KiB.
//! No occupancy assumption is made: every rank lookup first tests presence.

use crate::{Build, ExactMembership, IndexRead, KeyCodec, NoDense, rows::Rows};

const MAX_RANK_BITS: usize = 1 << 20;

struct RankDirectory {
    min: u128,
    span: usize,
    words: Vec<u64>,
    prefixes: Vec<usize>,
}

/// Shared clustered payload storage; each provider owns its own lookup directory.
pub(crate) struct RunRows<K, V> {
    pub(crate) keys: Vec<K>,
    pub(crate) offsets: Vec<usize>,
    pub(crate) items: Vec<V>,
}

impl<K: Ord + Copy, V> RunRows<K, V> {
    pub(crate) fn from_pairs(source: impl IntoIterator<Item = (K, V)>) -> Self {
        crate::count!(index_builds);
        let mut pairs: Vec<_> = source.into_iter().collect();
        pairs.sort_by_key(|pair| pair.0);
        Self::from_sorted_pairs(pairs)
    }

    /// Cache an order-preserving numeric key when sorting indirect keys.
    /// The stable cached-key sort keeps equal-key payloads in input order.
    /// Small, ordered, and strictly reversed inputs avoid the auxiliary key array.
    pub(crate) fn from_pairs_cached_key(
        source: impl IntoIterator<Item = (K, V)>,
        mut encode: impl FnMut(&K) -> u64,
    ) -> Self {
        crate::count!(index_builds);
        let mut pairs: Vec<_> = source.into_iter().collect();
        if pairs.len() <= 32 {
            pairs.sort_by_key(|pair| pair.0);
        } else if !pairs.is_sorted_by(|a, b| a.0 <= b.0) {
            if pairs.windows(2).all(|pair| pair[0].0 > pair[1].0) {
                // Strictness is required: reversing equal keys would reverse
                // the occurrence order promised by IndexRead.
                pairs.reverse();
            } else {
                pairs.sort_by_cached_key(|pair| encode(&pair.0));
            }
        }
        Self::from_sorted_pairs(pairs)
    }

    fn from_sorted_pairs(pairs: Vec<(K, V)>) -> Self {
        let distinct =
            usize::from(!pairs.is_empty()) + pairs.windows(2).filter(|p| p[0].0 != p[1].0).count();
        let mut keys = Vec::with_capacity(distinct);
        let mut offsets = Vec::with_capacity(distinct + 1);
        let mut items = Vec::with_capacity(pairs.len());
        for (key, item) in pairs {
            if keys.last() != Some(&key) {
                keys.push(key);
                offsets.push(items.len());
            }
            items.push(item);
            crate::count!(rows_copied);
        }
        offsets.push(items.len());
        #[cfg(feature = "counters")]
        crate::counters::build_cardinality(keys.len());
        Self {
            keys,
            offsets,
            items,
        }
    }
}

impl<K, V> RunRows<K, V> {
    #[inline]
    pub(crate) fn range(&self, slot: Option<usize>) -> (usize, usize) {
        slot.map_or((self.items.len(), self.items.len()), |slot| {
            (self.offsets[slot], self.offsets[slot + 1])
        })
    }

    #[inline]
    pub(crate) fn cursor(&self, slot: Option<usize>) -> RunCursor<'_, V> {
        let (begin, end) = self.range(slot);
        RunCursor {
            items: &self.items[begin..end],
            position: 0,
        }
    }
}

impl<K: Copy, V: Copy> RunRows<K, V> {
    pub(crate) fn rows(&self) -> impl Iterator<Item = (K, V)> {
        self.keys.iter().enumerate().flat_map(move |(slot, &key)| {
            self.items[self.offsets[slot]..self.offsets[slot + 1]]
                .iter()
                .map(move |&item| (key, item))
        })
    }
}

/// A stable sorted representation of `(key, item)` occurrences.
/// Keys require a total order consistent with Leibniz equality. Codecs obey
/// `KeyCodec`'s injectivity/order premises; `NoDense` uses binary search only.
/// Lookup handles borrow this value for their host lifetime, not the lifetime
/// of a temporary reference passed to `IndexRead::lookup`.
pub struct SortedRun<K, V, C = NoDense> {
    rows: RunRows<K, V>,
    rank: Option<RankDirectory>,
    codec: C,
}

impl<K: Ord + Copy, V: Copy, C: KeyCodec<K>> SortedRun<K, V, C> {
    /// Formal rule STR-SortedRun:
    /// Stable sorting preserves each key's input occurrence order and its
    /// multiplicity. Unique keys and adjacent offsets partition all items;
    /// presence plus rank selects precisely the same range as binary search.
    pub fn from_pairs(source: impl IntoIterator<Item = (K, V)>, codec: C) -> Self {
        let rows = RunRows::from_pairs(source);
        let rank = Self::rank_directory(&rows.keys, codec);
        Self { rows, rank, codec }
    }

    /// Number of distinct stored keys, also used by recorded join filters.
    pub fn len(&self) -> usize {
        self.rows.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.keys.is_empty()
    }

    /// Borrow the unique keys in increasing order without enumerating payloads.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.rows.keys.iter()
    }

    /// Every stored item in key order, bucket by bucket.
    pub fn items(&self) -> &[V] {
        &self.rows.items
    }

    /// Half-open position range of `key`'s bucket in [`Self::items`]; empty
    /// (at the end) when the key is absent. The same range `lookup` enumerates.
    pub fn range(&self, key: &K) -> (usize, usize) {
        self.rows.range(self.slot(key))
    }

    fn rank_directory(keys: &[K], codec: C) -> Option<RankDirectory> {
        if !codec.total() {
            return None;
        }
        let min = codec.ordinal(keys.first()?)?;
        let max = codec.ordinal(keys.last()?)?;
        let span = max.checked_sub(min)?.checked_add(1)?;
        if span > MAX_RANK_BITS as u128 {
            return None;
        }
        let span = span as usize;
        let mut words = vec![0u64; span.div_ceil(64)];
        for key in keys {
            let offset = usize::try_from(codec.ordinal(key)?.checked_sub(min)?).ok()?;
            if offset >= span {
                return None;
            }
            words[offset / 64] |= 1 << (offset % 64);
        }
        let mut count = 0;
        let prefixes = words
            .iter()
            .map(|word| {
                let prefix = count;
                count += word.count_ones() as usize;
                prefix
            })
            .collect();
        Some(RankDirectory {
            min,
            span,
            words,
            prefixes,
        })
    }

    fn slot(&self, key: &K) -> Option<usize> {
        if let Some(rank) = &self.rank {
            let offset = usize::try_from(self.codec.ordinal(key)?.checked_sub(rank.min)?).ok()?;
            if offset >= rank.span {
                return None;
            }
            let word = rank.words[offset / 64];
            let bit = 1u64 << (offset % 64);
            if word & bit == 0 {
                return None;
            }
            Some(rank.prefixes[offset / 64] + (word & (bit - 1)).count_ones() as usize)
        } else {
            self.rows.keys.binary_search(key).ok()
        }
    }
}

/// The directory stores each key once, independently of payload multiplicity.
impl<K, V, C> crate::Keys for SortedRun<K, V, C> {
    type Key = K;
    type Iter<'a>
        = crate::instrument::Cursor<std::slice::Iter<'a, K>>
    where
        Self: 'a;

    fn key_count(&self) -> usize {
        self.rows.keys.len()
    }

    fn distinct_keys(&self) -> Self::Iter<'_> {
        crate::instrument::cursor!(self.rows.keys.iter())
    }
}

impl<K: Ord + Copy, V: Copy, C: KeyCodec<K> + Default, I: Iterator<Item = (K, V)>> Build<I>
    for SortedRun<K, V, C>
{
    fn build(source: I) -> Self {
        Self::from_pairs(source, C::default())
    }
}

/// Copying a cursor snapshots its position without allocating or copying rows.
#[derive(Clone, Copy)]
pub struct RunCursor<'a, V> {
    items: &'a [V],
    position: usize,
}

impl<'a, V> RunCursor<'a, V> {
    #[inline]
    pub(crate) fn from_slice(items: &'a [V]) -> Self {
        Self { items, position: 0 }
    }
}

impl<V: Copy> Iterator for RunCursor<'_, V> {
    type Item = V;
    fn next(&mut self) -> Option<V> {
        crate::count!(cursor_steps);
        let item = self.items.get(self.position).copied();
        if item.is_some() {
            self.position += 1;
            crate::count!(rows_enumerated);
        }
        item
    }
    fn nth(&mut self, n: usize) -> Option<V> {
        self.position = self.position.saturating_add(n).min(self.items.len());
        self.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.items.len() - self.position;
        (remaining, Some(remaining))
    }
}
impl<V: Copy> ExactSizeIterator for RunCursor<'_, V> {}
impl<V: Copy> std::iter::FusedIterator for RunCursor<'_, V> {}

impl<'a, K: Ord + Copy, V: Copy, C: KeyCodec<K>> IndexRead for &'a SortedRun<K, V, C> {
    type Key = K;
    type Item = V;
    type Cursor = RunCursor<'a, V>;
    fn lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        self.rows.cursor(self.slot(key))
    }
}

impl<K: Ord + Copy, V: Copy, C: KeyCodec<K>> ExactMembership<K> for SortedRun<K, V, C> {
    fn contains(&self, key: &K) -> bool {
        crate::count!(lookups);
        self.slot(key).is_some()
    }
}

impl<K: Copy, V: Copy, C> Rows for &SortedRun<K, V, C> {
    type Row = (K, V);
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.rows.rows()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::I64Codec;

    #[test]
    fn distinct_keys_count_directory_entries_not_payload_occurrences() {
        fn check<C: KeyCodec<i64>>(codec: C) {
            for pairs in [
                vec![],
                vec![(2, 'a'), (1, 'z'), (2, 'b'), (1, 'z'), (2, 'a')],
            ] {
                let run = SortedRun::from_pairs(pairs.iter().copied(), codec);
                let expected: std::collections::BTreeSet<_> =
                    pairs.iter().map(|pair| pair.0).collect();
                assert_eq!(crate::Keys::key_count(&run), expected.len());
                assert_eq!(
                    crate::Keys::distinct_keys(&run)
                        .copied()
                        .collect::<Vec<_>>(),
                    expected.into_iter().collect::<Vec<_>>()
                );
                assert_eq!(run.items().len(), pairs.len());
            }
        }
        check(NoDense);
        check(I64Codec);
    }

    #[cfg(feature = "counters")]
    #[test]
    fn payload_build_counts_occurrences_not_set_insertions() {
        crate::counters::reset();
        let run = SortedRun::<_, _>::build([(1, 10), (1, 10), (2, 20)].into_iter());
        let counts = crate::counters::snapshot();
        assert_eq!(run.len(), 2);
        assert_eq!(counts.index_builds, 1);
        assert_eq!(counts.rows_copied, 3);
        // Like HashIndex, grouping payloads creates no set/dedup insertion
        // attempts. Distinct key cardinality belongs to build_cardinality.
        assert_eq!(counts.distinct_insertions, 0);
    }

    #[test]
    fn rank_boundaries_holes_and_budget() {
        for input in [
            vec![],
            vec![i64::MIN],
            vec![i64::MAX],
            vec![i64::MIN, i64::MAX],
            vec![-65, -64, -1, 0, 1, 63, 64, 65],
            vec![-7, MAX_RANK_BITS as i64 - 8],
            vec![-7, MAX_RANK_BITS as i64 - 7],
        ] {
            let run = SortedRun::from_pairs(input.iter().map(|&k| (k, k)), I64Codec);
            let span = input
                .first()
                .zip(input.last())
                .map(|(&min, &max)| max as i128 - min as i128 + 1);
            assert_eq!(
                run.rank.is_some(),
                span.is_some_and(|n| n <= MAX_RANK_BITS as i128)
            );
            for key in input.iter().copied().chain([
                i64::MIN,
                i64::MAX,
                -66,
                -65,
                -64,
                -63,
                -8,
                -7,
                -6,
                -1,
                0,
                1,
                2,
                62,
                63,
                64,
                65,
                66,
                MAX_RANK_BITS as i64 - 9,
                MAX_RANK_BITS as i64 - 8,
                MAX_RANK_BITS as i64 - 7,
            ]) {
                assert_eq!(
                    (&run).lookup(&key).collect::<Vec<_>>(),
                    input
                        .iter()
                        .copied()
                        .filter(|&k| k == key)
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn borrowed_integer_keys_and_partial_codecs() {
        use crate::BorrowedI64TupleCodec;
        let keys = [-65i64, -1, 0, 65];
        let run = crate::build!(
            SortedRun<_, _, crate::AutoDense>,
            keys.iter().map(|k| ((k,), *k)),
        );
        let run: SortedRun<(&i64,), i64, BorrowedI64TupleCodec> = run;
        assert!(run.rank.is_some());
        assert_eq!((&run).lookup(&(&-1,)).collect::<Vec<_>>(), [-1]);
        assert!(!run.contains(&(&1,)));
        let default_run = crate::build!(SortedRun<_, _>, keys.iter().map(|k| ((k,), *k)));
        assert!(default_run.rank.is_none());
        let explicit_run = crate::build!(
            SortedRun<_, _, NoDense>,
            keys.iter().map(|k| ((k,), *k)),
        );
        assert!(explicit_run.rank.is_none());
        let run = SortedRun::from_pairs([((&-1i32,), 'a'), ((&65,), 'b')], BorrowedI64TupleCodec);
        assert!(run.rank.is_some());
        assert_eq!((&run).lookup(&(&65,)).collect::<Vec<_>>(), ['b']);
        #[derive(Clone, Copy)]
        struct Partial;
        impl KeyCodec<i64> for Partial {
            fn ordinal(self, key: &i64) -> Option<u128> {
                (*key >= 0).then_some(*key as u128)
            }
        }
        let run = SortedRun::from_pairs([(-1, 'a'), (1, 'b')], Partial);
        assert!(run.rank.is_none());
        assert_eq!((&run).lookup(&-1).collect::<Vec<_>>(), ['a']);
    }

    #[test]
    fn range_offsets_match_scalar_cursor_steps() {
        let run = SortedRun::<_, _>::build([(1, 'a'), (1, 'b'), (1, 'c')].into_iter());
        for offset in [0, 1, 2, 3, usize::MAX] {
            let mut cursor = (&run).lookup(&1);
            assert_eq!(cursor.nth(offset), ['a', 'b', 'c'].get(offset).copied());
            assert_eq!(
                cursor.len(),
                3usize.saturating_sub(offset.saturating_add(1))
            );
            if offset >= 3 {
                assert_eq!(cursor.next(), None);
            }
        }
    }

    #[test]
    fn rows_preserve_bag_and_stable_order_and_cursors_outlive_handles() {
        let pairs = [(2, 'a'), (1, 'z'), (2, 'b'), (1, 'z'), (2, 'a')];
        let run = SortedRun::<_, _>::build(pairs.into_iter());
        assert!(run.rank.is_none());
        assert_eq!(
            (&run).rows().collect::<Vec<_>>(),
            [(1, 'z'), (1, 'z'), (2, 'a'), (2, 'b'), (2, 'a')]
        );
        let mut cursor = {
            let handle = &run;
            handle.lookup(&2)
        };
        assert_eq!(cursor.next(), Some('a'));
        let snapshot = cursor;
        assert_eq!(cursor.collect::<Vec<_>>(), ['b', 'a']);
        assert_eq!(snapshot.collect::<Vec<_>>(), ['b', 'a']);
        let words = SortedRun::<_, _>::build([("b", 1), ("a", 2)].into_iter());
        assert_eq!((&words).lookup(&"a").collect::<Vec<_>>(), [2]);
    }
}
