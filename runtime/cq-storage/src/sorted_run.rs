//! Payload-exact clustered runs. Binary search is the default directory;
//! a total, injective, order-preserving `KeyCodec` enables bitmap rank when
//! the checked inclusive span is at most 1,048,576 bits. On 64-bit hosts the
//! bitmap and per-word usize prefixes together use at most 256 KiB.
//! No occupancy assumption is made: every rank lookup first tests presence.
//! Direct builds publish maximum group size; selected builds may omit it
//! without changing the stored occurrences, directories, or length metadata.

use crate::{Build, ExactMembership, IndexRead, KeyCodec, NoDense, rows::Rows};

const MAX_RANK_BITS: usize = 1 << 20;

struct RankDirectory {
    min: u128,
    span: usize,
    words: Vec<u64>,
    prefixes: Vec<usize>,
}

/// Offsets use the narrowest supported carrier that represents the terminal
/// item position. Width is fixed when an immutable run is built.
pub(crate) enum RunOffsets {
    Narrow(Vec<u32>),
    Wide(Vec<usize>),
}

impl RunOffsets {
    fn with_capacity(capacity: usize, items: usize) -> Self {
        if u32::try_from(items).is_ok() {
            Self::Narrow(Vec::with_capacity(capacity))
        } else {
            Self::Wide(Vec::with_capacity(capacity))
        }
    }

    fn push(&mut self, offset: usize) {
        match self {
            Self::Narrow(offsets) => offsets.push(u32::try_from(offset).expect("run width")),
            Self::Wide(offsets) => offsets.push(offset),
        }
    }

    pub(crate) fn get(&self, slot: usize) -> usize {
        match self {
            Self::Narrow(offsets) => offsets[slot] as usize,
            Self::Wide(offsets) => offsets[slot],
        }
    }
}

/// Shared clustered payload storage; each provider owns its own lookup directory.
pub(crate) struct RunRows<K, V, C = Vec<V>> {
    pub(crate) keys: Vec<K>,
    pub(crate) offsets: RunOffsets,
    pub(crate) items: C,
    pub(crate) max_group: Option<usize>,
    row: std::marker::PhantomData<fn() -> V>,
}

impl<K: Ord + Copy, V: Copy, C: crate::ItemColumns<Row = V>> RunRows<K, V, C> {
    pub(crate) fn from_pairs(
        source: impl IntoIterator<Item = (K, V)>,
        max_group: bool,
    ) -> Self {
        crate::count!(index_builds);
        let mut pairs: Vec<_> = source.into_iter().collect();
        pairs.sort_by_key(|pair| pair.0);
        if max_group {
            Self::from_sorted_pairs::<true>(pairs)
        } else {
            Self::from_sorted_pairs::<false>(pairs)
        }
    }

    /// Cache an order-preserving numeric key when sorting indirect keys.
    /// The stable cached-key sort keeps equal-key payloads in input order.
    /// Small, ordered, and strictly reversed inputs avoid the auxiliary key array.
    pub(crate) fn from_pairs_cached_key(
        source: impl IntoIterator<Item = (K, V)>,
        mut encode: impl FnMut(&K) -> u64,
        max_group: bool,
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
        if max_group {
            Self::from_sorted_pairs::<true>(pairs)
        } else {
            Self::from_sorted_pairs::<false>(pairs)
        }
    }

    fn from_sorted_pairs<const MAX_GROUP: bool>(pairs: Vec<(K, V)>) -> Self {
        let distinct =
            usize::from(!pairs.is_empty()) + pairs.windows(2).filter(|p| p[0].0 != p[1].0).count();
        let mut keys = Vec::with_capacity(distinct);
        let mut offsets = RunOffsets::with_capacity(distinct + 1, pairs.len());
        let mut items = C::with_capacity(pairs.len());
        let mut max_group = 0;
        let mut group_start = 0;
        for (key, item) in pairs {
            if keys.last() != Some(&key) {
                if MAX_GROUP {
                    max_group = max_group.max(items.len() - group_start);
                    group_start = items.len();
                }
                keys.push(key);
                offsets.push(items.len());
            }
            items.push(item);
            crate::count!(rows_copied);
        }
        offsets.push(items.len());
        if MAX_GROUP {
            max_group = max_group.max(items.len() - group_start);
        }
        #[cfg(feature = "counters")]
        crate::counters::build_cardinality(keys.len());
        Self {
            keys,
            offsets,
            items,
            max_group: MAX_GROUP.then_some(max_group),
            row: std::marker::PhantomData,
        }
    }
}

impl<K, V: Copy, C: crate::ItemColumns<Row = V>> RunRows<K, V, C> {
    #[inline]
    pub(crate) fn range(&self, slot: Option<usize>) -> (usize, usize) {
        slot.map_or((self.items.len(), self.items.len()), |slot| {
            (self.offsets.get(slot), self.offsets.get(slot + 1))
        })
    }
}

impl<K, V: Copy> RunRows<K, V> {
    #[inline]
    pub(crate) fn cursor(&self, slot: Option<usize>) -> RunCursor<'_, V> {
        let (begin, end) = self.range(slot);
        RunCursor {
            items: &self.items[begin..end],
            position: 0,
        }
    }
}

impl<K: Copy, V: Copy, C: crate::ItemColumns<Row = V>> RunRows<K, V, C> {
    pub(crate) fn rows(&self) -> impl Iterator<Item = (K, V)> {
        self.keys.iter().enumerate().flat_map(move |(slot, &key)| {
            (self.offsets.get(slot)..self.offsets.get(slot + 1))
                .map(move |position| (key, self.items.item(position)))
        })
    }

    /// Merge-path partitions the two key directories, never materializing row
    /// tuples. Equal keys choose the older (left) run first; payload columns
    /// are copied by whole group, preserving occurrence order.
    pub(crate) fn merge(left: &Self, right: &Self, max_group: bool) -> Self
    where
        K: Ord,
    {
        if max_group {
            Self::merge_with_statistics::<true>(left, right)
        } else {
            Self::merge_with_statistics::<false>(left, right)
        }
    }

    fn merge_with_statistics<const MAX_GROUP: bool>(left: &Self, right: &Self) -> Self
    where
        K: Ord,
    {
        let total = left.keys.len() + right.keys.len();
        let item_count = left.items.len() + right.items.len();
        let mut result = Self {
            keys: Vec::with_capacity(total),
            offsets: RunOffsets::with_capacity(total + 1, item_count),
            items: C::with_capacity(item_count),
            max_group: None,
            row: std::marker::PhantomData,
        };
        const TILE: usize = 1024;
        let mut max_group = 0;
        let mut group_start = 0;
        for diagonal in (0..total).step_by(TILE) {
            let (mut a, mut b) = merge_path(&left.keys, &right.keys, diagonal);
            for _ in diagonal..(diagonal + TILE).min(total) {
                let (source, slot) = if b == right.keys.len()
                    || (a < left.keys.len() && left.keys[a] <= right.keys[b])
                {
                    let slot = a;
                    a += 1;
                    (left, slot)
                } else {
                    let slot = b;
                    b += 1;
                    (right, slot)
                };
                let key = source.keys[slot];
                if result.keys.last() != Some(&key) {
                    if MAX_GROUP {
                        max_group = max_group.max(result.items.len() - group_start);
                        group_start = result.items.len();
                    }
                    result.keys.push(key);
                    result.offsets.push(result.items.len());
                }
                let (begin, end) = source.range(Some(slot));
                result.items.extend_range(&source.items, begin..end);
                #[cfg(feature = "counters")]
                crate::counters::record(|counts| counts.rows_copied += (end - begin) as u64);
            }
        }
        result.offsets.push(result.items.len());
        if MAX_GROUP {
            max_group = max_group.max(result.items.len() - group_start);
            result.max_group = Some(max_group);
        }
        result
    }
}

/// Stable co-rank of a merge diagonal: left keys precede equal right keys.
fn merge_path<K: Ord>(left: &[K], right: &[K], diagonal: usize) -> (usize, usize) {
    let mut low = diagonal.saturating_sub(right.len());
    let mut high = diagonal.min(left.len());
    while low < high {
        let a = (low + high) / 2;
        let b = diagonal - a;
        if b > 0 && a < left.len() && right[b - 1] >= left[a] {
            low = a + 1;
        } else {
            high = a;
        }
    }
    (low, diagonal - low)
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

impl<K, V, C> SortedRun<K, V, C> {
    /// Formal rule RT-BuildStats:
    /// Published occurrence count; reading metadata never enumerates payloads.
    pub fn stat_rows(&self) -> Option<u64> {
        Some(self.rows.items.len() as u64)
    }

    /// Distinct full grouping keys, not distinct projected scalar keys.
    pub fn stat_keys(&self) -> Option<u64> {
        Some(self.rows.keys.len() as u64)
    }

    /// Available only when requested at build time; never scans the groups.
    pub fn stat_max_group(&self) -> Option<u64> {
        self.rows.max_group.map(|count| count as u64)
    }

    pub fn stat_runs(&self) -> Option<u64> {
        Some(u64::from(!self.rows.keys.is_empty()))
    }

    /// Full-key endpoints; `None` denotes a proven empty publication.
    /// These are not extrema of arbitrary composite-key components.
    pub fn key_bounds(&self) -> Option<(&K, &K)> {
        self.rows.keys.first().zip(self.rows.keys.last())
    }
}

impl<K, V, C> crate::run_filter::RunFilterSource for SortedRun<K, V, C> {
    type Key = K;

    fn key_runs(&self) -> impl Iterator<Item = &[K]> {
        std::iter::once(self.rows.keys.as_slice())
    }
}

impl<K: Ord + Copy, V: Copy, C: KeyCodec<K>> SortedRun<K, V, C> {
    /// Formal rule STR-SortedRun:
    /// Stable sorting preserves each key's input occurrence order and its
    /// multiplicity. Unique keys and adjacent offsets partition all items;
    /// presence plus rank selects precisely the same range as binary search.
    pub fn from_pairs(source: impl IntoIterator<Item = (K, V)>, codec: C) -> Self {
        Self::from_pairs_with_statistics(source, codec, true)
    }

    fn from_pairs_with_statistics(
        source: impl IntoIterator<Item = (K, V)>,
        codec: C,
        max_group: bool,
    ) -> Self {
        let rows = RunRows::from_pairs(source, max_group);
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
        Self::from_pairs_with_statistics(source, C::default(), false)
    }
}

impl<K: Ord + Copy, V: Copy, C: KeyCodec<K> + Default, I: Iterator<Item = (K, V)>>
    crate::BuildWithStatistics<I> for SortedRun<K, V, C>
{
    fn build_with_statistics<const ROWS: bool, const MAX_GROUP: bool>(source: I) -> Self {
        Self::from_pairs_with_statistics(source, C::default(), MAX_GROUP)
    }
}

impl<K, V, C, I, S> crate::BuildWithSidecar<I, S> for SortedRun<K, V, C>
where
    K: Ord + Copy,
    V: Copy,
    C: KeyCodec<K> + Default,
    I: Iterator<Item = (K, V)>,
    S: crate::SidecarBuilder<K>,
{
    fn build_with_sidecar<const ROWS: bool, const MAX_GROUP: bool>(source: I, mut sidecar: S) -> (Self, S::Output) {
        let run = Self::from_pairs_with_statistics(
            source.inspect(|(key, _)| sidecar.observe(key)),
            C::default(),
            MAX_GROUP,
        );
        (run, sidecar.finish())
    }
}

/// Copying a cursor snapshots its position without allocating or copying rows.
#[derive(Clone, Copy)]
pub struct RunCursor<'a, V> {
    items: &'a [V],
    position: usize,
}

impl<'a, V> RunCursor<'a, V> {
    /// Borrow a contiguous payload group without copying or allocating.
    #[inline]
    pub fn from_slice(items: &'a [V]) -> Self {
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
    fn published_statistics_match_occurrence_oracle() {
        for seed in 0_u64..48 {
            let mut input: Vec<_> = (0..seed * 11)
                .map(|i| (((i * 73 + seed) % 29) as i64 - 14, i))
                .collect();
            if seed != 0 {
                input.extend([(i64::MIN, 0), (i64::MAX, 0)]);
                input.extend((0..seed * 5).map(|i| (7, i)));
            }
            let mut oracle = std::collections::BTreeMap::<i64, u64>::new();
            for &(key, _) in &input {
                *oracle.entry(key).or_default() += 1;
            }
            for dense in [false, true] {
                let (rows, keys, maximum, runs, bounds) = if dense {
                    let run = SortedRun::from_pairs(input.iter().copied(), I64Codec);
                    (run.stat_rows(), run.stat_keys(), run.stat_max_group(), run.stat_runs(),
                     run.key_bounds().map(|(&lo, &hi)| (lo, hi)))
                } else {
                    let run = SortedRun::from_pairs(input.iter().copied(), NoDense);
                    (run.stat_rows(), run.stat_keys(), run.stat_max_group(), run.stat_runs(),
                     run.key_bounds().map(|(&lo, &hi)| (lo, hi)))
                };
                assert_eq!(rows, Some(input.len() as u64));
                assert_eq!(keys, Some(oracle.len() as u64));
                assert_eq!(maximum, Some(oracle.values().copied().max().unwrap_or(0)));
                assert_eq!(runs, Some(u64::from(!input.is_empty())));
                assert_eq!(bounds, oracle.first_key_value().zip(oracle.last_key_value())
                    .map(|((&lo, _), (&hi, _))| (lo, hi)));
            }
        }
    }

    #[test]
    fn selected_statistics_preserve_occurrences_and_direct_lengths() {
        use crate::BuildWithStatistics;

        fn check<C: KeyCodec<i64> + Default>(input: &[(i64, u64)]) {
            let mut expected = input.to_vec();
            expected.sort_by_key(|pair| pair.0);
            let mut oracle = std::collections::BTreeMap::<i64, Vec<u64>>::new();
            for &(key, item) in input {
                oracle.entry(key).or_default().push(item);
            }
            for requested in [false, true] {
                let run = if requested {
                    SortedRun::<_, _, C>::build_with_statistics::<true, true>(input.iter().copied())
                } else {
                    SortedRun::<_, _, C>::build_with_statistics::<true, false>(input.iter().copied())
                };
                let maximum = requested.then(|| {
                    oracle.values().map(|items| items.len() as u64).max().unwrap_or(0)
                });
                assert_eq!(run.stat_rows(), Some(input.len() as u64));
                assert_eq!(run.stat_keys(), Some(oracle.len() as u64));
                assert_eq!(run.stat_max_group(), maximum);
                assert_eq!(run.stat_runs(), Some(u64::from(!input.is_empty())));
                assert_eq!(run.key_bounds(), oracle.first_key_value().zip(oracle.last_key_value())
                    .map(|((lo, _), (hi, _))| (lo, hi)));
                assert_eq!((&run).rows().collect::<Vec<_>>(), expected);
                assert_eq!(run.items(), expected.iter().map(|pair| pair.1).collect::<Vec<_>>());
                for (&key, items) in &oracle {
                    assert_eq!((&run).lookup(&key).collect::<Vec<_>>(), *items);
                    let (begin, end) = run.range(&key);
                    assert_eq!(&run.items()[begin..end], items);
                }
                assert_eq!(run.stat_max_group(), maximum);
            }
        }

        for input in [
            vec![],
            vec![(2, 7), (1, 9), (2, 7), (1, 9), (2, 3)],
            vec![(i64::MAX, 1), (i64::MIN, 2), (i64::MAX, 1)],
            (0..2049).map(|i| (((i * 73) % 31) as i64 - 15, i)).collect(),
        ] {
            check::<NoDense>(&input);
            check::<I64Codec>(&input);
        }
    }

    #[test]
    fn merged_group_maximum_includes_duplicates_across_merge_tiles() {
        for requested in [false, true] {
            let left = RunRows::<_, _>::from_pairs((0..2050).map(|key| (key, key)), requested);
            let right = RunRows::<_, _>::from_pairs(
                (0..2050).flat_map(|key| (0..if key == 1023 { 17 } else { 2 }).map(move |v| (key, v))),
                requested,
            );
            let merged = RunRows::merge(&left, &right, requested);
            assert_eq!(merged.keys.len(), 2050);
            assert_eq!(merged.max_group, requested.then_some(18));
            assert_eq!(merged.items.len(), 2050 * 3 + 15);
            let mut expected: Vec<_> = left.rows().chain(right.rows()).collect();
            expected.sort_by_key(|pair| pair.0);
            assert_eq!(merged.rows().collect::<Vec<_>>(), expected);
            let empty = RunRows::<i32, i32>::from_pairs([], requested);
            assert_eq!(RunRows::merge(&empty, &empty, requested).max_group, requested.then_some(0));
        }
    }

    #[test]
    fn offset_width_preserves_terminal_positions_at_the_u32_boundary() {
        let mut narrow = RunOffsets::with_capacity(2, u32::MAX as usize);
        narrow.push(0);
        narrow.push(u32::MAX as usize);
        assert_eq!(narrow.get(0), 0);
        assert_eq!(narrow.get(1), u32::MAX as usize);
        assert!(matches!(narrow, RunOffsets::Narrow(_)));
        if usize::BITS > 32 {
            let terminal = (u32::MAX as usize).checked_add(1).unwrap();
            let mut wide = RunOffsets::with_capacity(2, terminal);
            wide.push(0);
            wide.push(terminal);
            assert_eq!(wide.get(1), terminal);
            assert!(matches!(wide, RunOffsets::Wide(_)));
        }
    }

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
    fn borrowed_integer_keys_and_disabled_codecs() {
        use crate::{BorrowedI64TupleCodec, Build, I64Codec};
        let keys = [-65i64, -1, 0, 65];
        let run = <SortedRun<_, _, I64Codec> as Build<_>>::build(keys.iter().map(|k| ((k,), *k)));
        let run: SortedRun<(&i64,), i64, I64Codec> = run;
        assert!(run.rank.is_some());
        assert_eq!((&run).lookup(&(&-1,)).collect::<Vec<_>>(), [-1]);
        assert!(!run.contains(&(&1,)));
        let default_run = <SortedRun<_, _> as Build<_>>::build(keys.iter().map(|k| ((k,), *k)));
        assert!(default_run.rank.is_none());
        let explicit_run =
            <SortedRun<_, _, NoDense> as Build<_>>::build(keys.iter().map(|k| ((k,), *k)));
        assert!(explicit_run.rank.is_none());
        let run = SortedRun::from_pairs([((&-1i32,), 'a'), ((&65,), 'b')], BorrowedI64TupleCodec);
        assert!(run.rank.is_some());
        assert_eq!((&run).lookup(&(&65,)).collect::<Vec<_>>(), ['b']);
        let run = SortedRun::from_pairs([(-1i64, 'a'), (1, 'b')], NoDense);
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
