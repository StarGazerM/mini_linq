//! Payload-exact clustered runs indexed by a radix directory and bounded-error spline.
//! The representation shares grouping/cursors with other run providers, not their
//! search algorithms. Payload lookup checks exact equality in the spline window;
//! dense-domain membership can use a bounded, lazily initialized presence bitmap.
//! Maximum group size is maintained only when requested at build time. An
//! unrequested maximum stays unavailable through maintenance and lazy reads.

use crate::radix_spline::{RadixSpline, RadixSplineMetadata, RadixSplinePoint, RadixSplineView};
use crate::rows::Rows;
use crate::sorted_run::RunRows;
use crate::{Build, ExactMembership, IndexRead, ItemColumns, OrderKey, RunCursor};
use std::{cmp::Reverse, collections::BinaryHeap, mem::size_of, sync::OnceLock};

// DESIGN ordered-index-key
// STORAGE §8: one encoding serves every index role; absent queries obey the
// same total order law. Never truncate a wider ordinal into a radix key.

#[cfg(test)]
thread_local! {
    static DIRECTORY_SEARCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A bounded exact membership projection, initialized only if contains is used.
/// Payload lookup continues to use the spline; sparse domains allocate no bitmap.
struct DenseMembership {
    min: u64,
    span: usize,
    // Initialization needs the owning run's runtime keys, not a static initializer.
    bits: OnceLock<Box<[u64]>>,
}

impl DenseMembership {
    fn candidate<K: OrderKey<Ordinal = u64> + Copy>(keys: &[K]) -> Option<Self> {
        const MAX_BITS: u64 = 1 << 20;
        let min = keys.first()?.ordinal();
        let span = keys.last()?.ordinal().checked_sub(min)?.checked_add(1)?;
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
    fn contains<K: OrderKey<Ordinal = u64> + Copy>(&self, key: u64, keys: &[K]) -> bool {
        let Some(offset) = key
            .checked_sub(self.min)
            .filter(|offset| *offset < self.span as u64)
        else {
            return false;
        };
        let bits = self.bits.get_or_init(|| {
            let mut bits = vec![0_u64; self.span.div_ceil(64)];
            for key in keys {
                let offset = (key.ordinal() - self.min) as usize;
                bits[offset / 64] |= 1_u64 << (offset % 64);
            }
            bits.into_boxed_slice()
        });
        bits[offset as usize / 64] & (1_u64 << (offset % 64)) != 0
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
    pub fn new<K: OrderKey<Ordinal = u64> + Copy>(keys: &[K]) -> Self {
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

impl<'a, K: OrderKey<Ordinal = u64> + Copy, V: Copy> RadixRunView<'a, K, V> {
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
            directory: RadixSplineView {
                points,
                radix,
                metadata,
            },
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
        self.slot(key)
            .map_or((self.items.len(), self.items.len()), |slot| {
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
            Some(membership) => membership.contains(key.ordinal(), self.keys),
            None => self.slot(key).is_some(),
        }
    }

    #[inline]
    fn slot(&self, key: &K) -> Option<usize> {
        let bound = self.directory.search_bound(key.ordinal());
        self.keys[bound.clone()]
            .binary_search(key)
            .ok()
            .map(|i| bound.start + i)
    }
}

/// Immutable grouped runs with ratio-bounded merging. Membership, counts
/// and lookup cursors read the runs directly. Legacy contiguous `items()`
/// materializes a stable merged view only when requested.
pub struct RadixRun<K, V, C = Vec<V>> {
    runs: Vec<Segment<K, V, C>>,
    ratio: usize,
    key_count: OnceLock<usize>,
    max_group: Option<OnceLock<usize>>,
    row_count: usize,
    bounds: Option<(usize, usize)>,
    compact: OnceLock<Vec<V>>,
}

struct Segment<K, V, C> {
    rows: RunRows<K, V, C>,
    directory: RadixSpline,
    state: RadixRunReadState,
}

impl<K, V, C> RadixRun<K, V, C> {
    pub fn stat_rows(&self) -> Option<u64> {
        Some(self.row_count as u64)
    }

    /// Global distinct keys are unavailable until an existing read finalizes them.
    pub fn stat_keys(&self) -> Option<u64> {
        match self.runs.as_slice() {
            [] => Some(0),
            [run] => Some(run.rows.keys.len() as u64),
            _ => self.key_count.get().map(|&count| count as u64),
        }
    }

    /// Local maxima cannot be combined when a key occurs in several runs.
    /// Unrequested maxima remain unavailable, even after contiguous reads.
    pub fn stat_max_group(&self) -> Option<u64> {
        let max_group = self.max_group.as_ref()?;
        match self.runs.as_slice() {
            [] => Some(0),
            [run] => run.rows.max_group.map(|count| count as u64),
            _ => max_group.get().map(|&count| count as u64),
        }
    }

    pub fn stat_runs(&self) -> Option<u64> {
        Some(self.runs.len() as u64)
    }

    /// Borrow the full-key extrema; `None` denotes a proven empty publication.
    pub fn key_bounds(&self) -> Option<(&K, &K)> {
        let (first, last) = self.bounds?;
        self.runs[first].rows.keys.first().zip(self.runs[last].rows.keys.last())
    }
}

impl<K, V, C> crate::run_filter::RunFilterSource for RadixRun<K, V, C> {
    type Key = K;

    fn key_runs(&self) -> impl Iterator<Item = &[K]> {
        self.runs.iter().map(|run| run.rows.keys.as_slice())
    }
}

impl<K, V> crate::run_filter::RunFilterSource for RadixRunView<'_, K, V> {
    type Key = K;

    fn key_runs(&self) -> impl Iterator<Item = &[K]> {
        std::iter::once(self.keys)
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> Segment<K, V, C> {
    fn new(rows: RunRows<K, V, C>) -> Self {
        let directory = RadixSpline::build(rows.keys.len(), |i| rows.keys[i].ordinal());
        let state = RadixRunReadState::new(&rows.keys);
        Self {
            rows,
            directory,
            state,
        }
    }

    fn slot(&self, key: &K) -> Option<usize> {
        self.slot_ordinal(key.ordinal())
    }

    fn slot_ordinal(&self, key: u64) -> Option<usize> {
        self.search_ordinal(key).ok()
    }

    fn search_ordinal(&self, key: u64) -> Result<usize, usize> {
        #[cfg(test)]
        DIRECTORY_SEARCHES.with(|searches| searches.set(searches.get() + 1));
        let bound = self.directory.as_view().search_bound(key);
        self.rows.keys[bound.clone()]
            .binary_search_by_key(&key, OrderKey::ordinal)
            .map(|i| bound.start + i)
            .map_err(|i| bound.start + i)
    }

    fn contains(&self, key: &K) -> bool {
        match &self.state.membership {
            Some(membership) => membership.contains(key.ordinal(), &self.rows.keys),
            None => self.slot(key).is_some(),
        }
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> crate::Keys
    for RadixRun<K, V, C>
{
    type Key = K;
    type Iter<'a>
        = RadixKeys<'a, K, V, C>
    where
        Self: 'a;

    fn key_count(&self) -> usize {
        self.len()
    }

    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.keys()
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> Default
    for RadixRun<K, V, C>
{
    fn default() -> Self {
        Self::from_column_rows([], 4)
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy> RadixRun<K, V> {
    /// Formal rule STR-RadixRun:
    /// Stable grouping preserves occurrences; the single OrderKey law and an
    /// error-bounded spline followed by exact search select exact groups.
    pub fn from_pairs(source: impl IntoIterator<Item = (K, V)>) -> Self {
        Self::with_ratio(source, 4)
    }

    /// Select the geometric size ratio for lazy merging. Adjacent runs
    /// from oldest to newest satisfy `older.len >= ratio * newer.len`.
    /// Ratios below two cannot bound the number of live runs.
    pub fn with_ratio(source: impl IntoIterator<Item = (K, V)>, ratio: usize) -> Self {
        Self::from_column_rows(source, ratio)
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> RadixRun<K, V, C> {
    /// Build the explicitly selected payload-column layout with a run ratio.
    /// For example, `C = (Vec<A>, Vec<B>)` stores `(A,B)` as independent columns.
    pub fn from_column_rows(source: impl IntoIterator<Item = (K, V)>, ratio: usize) -> Self {
        Self::from_column_rows_with_statistics(source, ratio, true)
    }

    fn from_column_rows_with_statistics(
        source: impl IntoIterator<Item = (K, V)>,
        ratio: usize,
        max_group: bool,
    ) -> Self {
        assert!(ratio >= 2, "run ratio must be at least two");
        let rows = RunRows::<K, V, C>::from_pairs_cached_key(source, OrderKey::ordinal, max_group);
        let row_count = rows.items.len();
        let runs = if rows.items.is_empty() {
            Vec::new()
        } else {
            vec![Segment::new(rows)]
        };
        Self {
            runs,
            ratio,
            key_count: OnceLock::new(),
            max_group: max_group.then(OnceLock::new),
            row_count,
            bounds: (row_count != 0).then_some((0, 0)),
            compact: OnceLock::new(),
        }
    }

    /// Borrow each payload group as columns and a position interval.
    /// Callers can read only the required tuple-vector component; no complete
    /// row is reconstructed and no contiguous row cache is initialized.
    pub fn column_groups(&self, key: &K) -> impl Iterator<Item = (&C, std::ops::Range<usize>)> {
        let key = key.ordinal();
        self.runs.iter().filter_map(move |run| {
            let (begin, end) = run.rows.range(run.slot_ordinal(key));
            (begin != end).then_some((&run.rows.items, begin..end))
        })
    }

    pub fn len(&self) -> usize {
        if self.runs.len() == 1 {
            self.runs[0].rows.keys.len()
        } else {
            *self.key_count.get_or_init(|| self.keys().count())
        }
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// Unique keys in increasing order; never reads or materializes payloads.
    pub fn keys(&self) -> RadixKeys<'_, K, V, C> {
        let single = (self.runs.len() == 1).then(|| self.runs[0].rows.keys.iter());
        let heads = if single.is_some() {
            BinaryHeap::new()
        } else {
            BinaryHeap::from(
                self.runs
                    .iter()
                    .enumerate()
                    .filter_map(|(run, segment)| {
                        segment
                            .rows
                            .keys
                            .first()
                            .map(|key| Reverse((key.ordinal(), run, 0)))
                    })
                    .collect::<Vec<_>>(),
            )
        };
        RadixKeys {
            runs: &self.runs,
            single,
            heads,
            last: None,
        }
    }

    /// Number of immutable runs currently read, useful for maintenance evidence.
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }

    /// Number of payload occurrences at this key, using directories/offsets only.
    pub fn count(&self, key: &K) -> usize {
        self.runs
            .iter()
            .map(|run| {
                let (begin, end) = run.rows.range(run.slot(key));
                end - begin
            })
            .sum()
    }

    /// All payloads in stable key order. Multi-run storage is merged lazily for
    /// this contiguous compatibility read, not for membership/count/lookup.
    pub fn items(&self) -> &[V] {
        self.compacted()
    }

    /// Exact range in `items()`, computed without reading any item column.
    pub fn range(&self, key: &K) -> (usize, usize) {
        if self.runs.len() == 1 {
            let run = &self.runs[0];
            return run.rows.range(run.slot(key));
        }
        let ordinal = key.ordinal();
        let mut begin = 0;
        let mut count = 0;
        let mut total = 0;
        for run in &self.runs {
            match run.search_ordinal(ordinal) {
                Ok(slot) => {
                    begin += run.rows.offsets.get(slot);
                    count += run.rows.offsets.get(slot + 1) - run.rows.offsets.get(slot);
                }
                Err(slot) => begin += run.rows.offsets.get(slot),
            }
            total += run.rows.items.len();
        }
        if count == 0 {
            (total, total)
        } else {
            (begin, begin + count)
        }
    }

    fn compacted(&self) -> &[V] {
        if self.runs.len() == 1
            && let Some(items) = self.runs[0].rows.items.row_slice()
        {
            return items;
        }
        self.compact.get_or_init(|| {
            let mut runs = self.runs.iter();
            match (runs.next(), runs.next()) {
                (Some(left), Some(right)) => {
                    let requested = self.max_group.is_some();
                    let mut rows = RunRows::merge(&left.rows, &right.rows, requested);
                    for run in runs {
                        rows = RunRows::merge(&rows, &run.rows, requested);
                    }
                    let _ = self.key_count.set(rows.keys.len());
                    if let Some(max_group) = &self.max_group {
                        let _ = max_group.set(rows.max_group.expect("requested maximum group size"));
                    }
                    rows.items.into_rows()
                }
                (Some(run), None) => (0..run.rows.items.len())
                    .map(|position| run.rows.items.item(position))
                    .collect(),
                _ => Vec::new(),
            }
        })
    }
}

/// Borrowed unique keys across all runs, without touching payloads.
pub struct RadixKeys<'a, K, V, C = Vec<V>> {
    runs: &'a [Segment<K, V, C>],
    single: Option<std::slice::Iter<'a, K>>,
    heads: BinaryHeap<Reverse<(u64, usize, usize)>>,
    last: Option<u64>,
}

impl<'a, K: OrderKey<Ordinal = u64>, V, C> Iterator for RadixKeys<'a, K, V, C> {
    type Item = &'a K;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(single) = &mut self.single {
            return single.next();
        }
        while let Some(Reverse((ordinal, run, slot))) = self.heads.pop() {
            let keys = &self.runs[run].rows.keys;
            if let Some(next) = keys.get(slot + 1) {
                self.heads.push(Reverse((next.ordinal(), run, slot + 1)));
            }
            if self.last != Some(ordinal) {
                self.last = Some(ordinal);
                return Some(&keys[slot]);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.single.as_ref().map_or_else(
            || {
                (
                    0,
                    Some(
                        self.heads
                            .iter()
                            .map(|Reverse((_, run, slot))| self.runs[*run].rows.keys.len() - slot)
                            .sum(),
                    ),
                )
            },
            Iterator::size_hint,
        )
    }
}

impl<K: OrderKey<Ordinal = u64>, V, C> std::iter::FusedIterator for RadixKeys<'_, K, V, C> {}

/// Copyable, allocation-free cursor across immutable payload groups.
/// The ordinal is captured, not a borrow of the temporary query key.
pub struct RadixCursor<'a, K, V, C = Vec<V>> {
    runs: &'a [Segment<K, V, C>],
    key: u64,
    current: Option<(&'a C, usize, usize)>,
}

impl<K, V, C> Copy for RadixCursor<'_, K, V, C> {}
impl<K, V, C> Clone for RadixCursor<'_, K, V, C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> Iterator
    for RadixCursor<'_, K, V, C>
{
    type Item = V;

    fn next(&mut self) -> Option<V> {
        crate::count!(cursor_steps);
        loop {
            if let Some((columns, position, end)) = &mut self.current
                && *position < *end
            {
                let item = columns.item(*position);
                *position += 1;
                crate::count!(rows_enumerated);
                return Some(item);
            }
            let (run, remaining) = self.runs.split_first()?;
            self.runs = remaining;
            let (begin, end) = run.rows.range(run.slot_ordinal(self.key));
            self.current = Some((&run.rows.items, begin, end));
        }
    }

    fn nth(&mut self, mut n: usize) -> Option<V> {
        loop {
            if let Some((_, position, end)) = &mut self.current {
                let remaining = *end - *position;
                if n < remaining {
                    *position += n;
                    return self.next();
                }
                n -= remaining;
                *position = *end;
            }
            let (run, remaining) = self.runs.split_first()?;
            self.runs = remaining;
            let (begin, end) = run.rows.range(run.slot_ordinal(self.key));
            self.current = Some((&run.rows.items, begin, end));
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.current.map_or(0, |(_, begin, end)| end - begin)
            + self
                .runs
                .iter()
                .map(|run| {
                    let (begin, end) = run.rows.range(run.slot_ordinal(self.key));
                    end - begin
                })
                .sum::<usize>();
        (remaining, Some(remaining))
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> std::iter::FusedIterator
    for RadixCursor<'_, K, V, C>
{
}
impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> ExactSizeIterator
    for RadixCursor<'_, K, V, C>
{
}

impl<
    K: OrderKey<Ordinal = u64> + Copy,
    V: Copy,
    C: ItemColumns<Row = V>,
    I: Iterator<Item = (K, V)>,
> Build<I> for RadixRun<K, V, C>
{
    fn build(source: I) -> Self {
        Self::from_column_rows_with_statistics(source, 4, false)
    }
}

impl<K, V, C, I> crate::BuildWithStatistics<I> for RadixRun<K, V, C>
where
    K: OrderKey<Ordinal = u64> + Copy,
    V: Copy,
    C: ItemColumns<Row = V>,
    I: Iterator<Item = (K, V)>,
{
    fn build_with_statistics<const ROWS: bool, const MAX_GROUP: bool>(source: I) -> Self {
        Self::from_column_rows_with_statistics(source, 4, MAX_GROUP)
    }
}

impl<K, V, C, I, S> crate::BuildWithSidecar<I, S> for RadixRun<K, V, C>
where
    K: OrderKey<Ordinal = u64> + Copy,
    V: Copy,
    C: ItemColumns<Row = V>,
    I: Iterator<Item = (K, V)>,
    S: crate::SidecarBuilder<K>,
{
    fn build_with_sidecar<const ROWS: bool, const MAX_GROUP: bool>(source: I, mut sidecar: S) -> (Self, S::Output) {
        let run = Self::from_column_rows_with_statistics(
            source.inspect(|(key, _)| sidecar.observe(key)),
            4,
            MAX_GROUP,
        );
        (run, sidecar.finish())
    }
}

impl<'a, K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> IndexRead
    for &'a RadixRun<K, V, C>
{
    type Key = K;
    type Item = V;
    type Cursor = RadixCursor<'a, K, V, C>;

    fn lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        let ordinal = key.ordinal();
        let (current, runs) = match self.runs.split_first() {
            Some((run, remaining)) => {
                let (begin, end) = run.rows.range(run.slot_ordinal(ordinal));
                (Some((&run.rows.items, begin, end)), remaining)
            }
            None => (None, &[][..]),
        };
        RadixCursor {
            runs,
            key: ordinal,
            current,
        }
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> ExactMembership<K>
    for RadixRun<K, V, C>
{
    fn contains(&self, key: &K) -> bool {
        crate::count!(lookups);
        self.runs.iter().any(|run| run.contains(key))
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> Rows
    for &RadixRun<K, V, C>
{
    type Row = (K, V);
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.keys()
            .flat_map(move |&key| self.lookup(&key).map(move |item| (key, item)))
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: ItemColumns<Row = V>> RadixRun<K, V, C> {
    /// Formal rule IX-RunMerge:
    /// The provider's maintenance merge: afterwards reads observe the old
    /// occurrences plus every delta occurrence (a multiset union; no
    /// disjointness is required). The grouped delta is appended as a run and
    /// adjacent runs are stably merged by merge-path while the geometric ratio
    /// is violated; direct reads include all unmerged runs.
    /// The build's maximum-group request is preserved for deltas and merges.
    pub fn merge(&mut self, delta: impl IntoIterator<Item = (K, V)>) {
        let requested = self.max_group.is_some();
        let rows = RunRows::<K, V, C>::from_pairs_cached_key(delta, OrderKey::ordinal, requested);
        if rows.items.is_empty() {
            return;
        }
        self.key_count.take();
        if let Some(max_group) = &mut self.max_group {
            max_group.take();
        }
        self.row_count += rows.items.len();
        self.compact.take();
        self.runs.push(Segment::new(rows));
        while self.runs.len() >= 2 {
            let n = self.runs.len();
            if self.runs[n - 2].rows.items.len() / self.ratio >= self.runs[n - 1].rows.items.len() {
                break;
            }
            let newer = self.runs.pop().expect("two runs");
            let older = self.runs.pop().expect("two runs");
            self.runs.push(Segment::new(RunRows::merge(
                &older.rows,
                &newer.rows,
                requested,
            )));
        }
        // Only endpoint metadata is reduced at publication; no key/payload scan.
        let first = (0..self.runs.len())
            .min_by_key(|&i| self.runs[i].rows.keys.first());
        let last = (0..self.runs.len())
            .max_by_key(|&i| self.runs[i].rows.keys.last());
        self.bounds = first.zip(last);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_statistics_preserve_lazy_boundaries_and_empty_delta() {
        let mut run = RadixRun::from_pairs((0_u64..256).map(|key| (key, key)));
        run.merge([(7, 1000), (u64::MAX, 2000)]);
        assert_eq!(run.stat_rows(), Some(258));
        assert_eq!(run.stat_runs(), Some(2));
        assert_eq!(run.stat_keys(), None);
        assert_eq!(run.stat_max_group(), None);
        assert_eq!(run.key_bounds(), Some((&0, &u64::MAX)));
        let view = crate::RunFilterView::new(&run, |key| key);
        assert!(view.test(&7));
        assert!(view.any_in(&u64::MAX, &u64::MAX));
        assert!(!view.any_in(&300, &(u64::MAX - 1)));
        assert!(run.key_count.get().is_none());
        assert!(run.compact.get().is_none());
        assert!(run.runs.iter().all(|segment| segment.state.membership.as_ref()
            .is_none_or(|membership| membership.bits.get().is_none())));
        assert_eq!(run.len(), 257);
        assert_eq!(run.stat_keys(), Some(257));
        assert_eq!(run.stat_max_group(), None);
        assert_eq!(run.items().len(), 258);
        assert_eq!(run.stat_max_group(), Some(2));
        run.merge([]);
        assert_eq!(run.stat_keys(), Some(257));
        assert_eq!(run.stat_max_group(), Some(2));
        assert!(run.compact.get().is_some());
        run.merge([(7, 3000)]);
        assert_eq!(run.stat_rows(), Some(259));
        assert_eq!(run.stat_keys(), None);
        assert_eq!(run.stat_max_group(), None);
        assert!(run.compact.get().is_none());
        assert_eq!(run.items().len(), 259);
        assert_eq!(run.stat_keys(), Some(257));
        assert_eq!(run.stat_max_group(), Some(3));
    }

    #[test]
    fn selected_statistics_preserve_empty_single_run_and_columnar_occurrences() {
        use crate::BuildWithStatistics;

        fn check<C: ItemColumns<Row = (u64, u64)>>(input: &[(u64, (u64, u64))]) {
            let mut expected = input.to_vec();
            expected.sort_by_key(|pair| pair.0);
            let mut oracle = std::collections::BTreeMap::<u64, Vec<(u64, u64)>>::new();
            for &(key, item) in input {
                oracle.entry(key).or_default().push(item);
            }
            for requested in [false, true] {
                let run = if requested {
                    RadixRun::<_, _, C>::build_with_statistics::<true, true>(input.iter().copied())
                } else {
                    RadixRun::<_, _, C>::build_with_statistics::<true, false>(input.iter().copied())
                };
                let maximum = requested.then(|| {
                    oracle.values().map(|items| items.len() as u64).max().unwrap_or(0)
                });
                assert_eq!(run.stat_rows(), Some(input.len() as u64));
                assert_eq!(run.stat_keys(), Some(oracle.len() as u64));
                assert_eq!(run.stat_runs(), Some(u64::from(!input.is_empty())));
                assert_eq!(run.stat_max_group(), maximum);
                assert_eq!(run.key_bounds(), oracle.first_key_value().zip(oracle.last_key_value())
                    .map(|((lo, _), (hi, _))| (lo, hi)));
                assert_eq!(run.max_group.is_some(), requested);
                assert!(run.runs.iter().all(|segment| segment.rows.max_group.is_some() == requested));
                assert_eq!((&run).rows().collect::<Vec<_>>(), expected);
                for (&key, items) in &oracle {
                    assert_eq!(run.count(&key), items.len());
                    assert_eq!((&run).lookup(&key).collect::<Vec<_>>(), *items);
                }
                assert!(run.compact.get().is_none());
                assert_eq!(run.items(), expected.iter().map(|pair| pair.1).collect::<Vec<_>>());
                assert_eq!(run.stat_max_group(), maximum);
            }
        }

        for input in [
            vec![],
            vec![(u64::MAX, (7, 8))],
            vec![(2, (7, 8)), (1, (9, 10)), (2, (7, 8)), (1, (9, 10)), (2, (3, 4))],
            (0..2049).map(|i| ((i * 73) % 31, (i, i + 1))).collect(),
        ] {
            check::<Vec<(u64, u64)>>(&input);
            check::<(Vec<u64>, Vec<u64>)>(&input);
        }
    }

    #[test]
    fn disabled_statistics_stay_absent_through_lazy_and_maintenance_merges() {
        use crate::BuildWithStatistics;

        let mut input: Vec<_> = (0_u64..256).map(|key| (key, key)).collect();
        let mut run = RadixRun::<_, _>::build_with_statistics::<true, false>(input.iter().copied());
        let delta = [(7, 1000), (u64::MAX, 2000)];
        run.merge(delta);
        input.extend(delta);
        assert_eq!(run.stat_rows(), Some(258));
        assert_eq!(run.stat_runs(), Some(2));
        assert_eq!(run.stat_keys(), None);
        assert_eq!(run.stat_max_group(), None);
        assert_eq!(run.key_bounds(), Some((&0, &u64::MAX)));
        assert!(run.max_group.is_none());
        assert!(run.runs.iter().all(|segment| segment.rows.max_group.is_none()));
        let view = crate::RunFilterView::new(&run, |key| key);
        assert!(view.test(&7));
        assert!(view.any_in(&u64::MAX, &u64::MAX));
        assert!(!view.any_in(&300, &(u64::MAX - 1)));
        assert_eq!((&run).lookup(&7).collect::<Vec<_>>(), [7, 1000]);
        assert_eq!(run.count(&7), 2);
        assert_eq!(run.range(&7), (7, 9));
        assert!(run.key_count.get().is_none());
        assert!(run.compact.get().is_none());
        assert!(run.runs.iter().all(|segment| segment.state.membership.as_ref()
            .is_none_or(|membership| membership.bits.get().is_none())));
        assert_eq!(run.len(), 257);
        assert_eq!(run.stat_keys(), Some(257));
        assert_eq!(run.stat_max_group(), None);
        let mut expected = input.clone();
        expected.sort_by_key(|pair| pair.0);
        assert_eq!(run.items(), expected.iter().map(|pair| pair.1).collect::<Vec<_>>());
        assert_eq!(run.stat_max_group(), None);
        run.merge([]);
        assert_eq!(run.stat_keys(), Some(257));
        assert_eq!(run.stat_max_group(), None);
        assert!(run.compact.get().is_some());
        run.merge([(7, 3000)]);
        input.push((7, 3000));
        assert_eq!(run.stat_keys(), None);
        assert_eq!(run.stat_max_group(), None);
        assert!(run.compact.get().is_none());
        assert_eq!((&run).lookup(&7).collect::<Vec<_>>(), [7, 1000, 3000]);
        assert_eq!(run.items().len(), input.len());
        assert_eq!(run.stat_max_group(), None);
        let delta: Vec<_> = (0..2049).map(|i| ((i * 73) % 31, i + 4000)).collect();
        run.merge(delta.iter().copied());
        input.extend(delta);
        expected = input;
        expected.sort_by_key(|pair| pair.0);
        assert_eq!(run.run_count(), 1);
        assert_eq!(run.stat_rows(), Some(expected.len() as u64));
        assert_eq!(run.stat_keys(), Some(257));
        assert_eq!(run.stat_max_group(), None);
        assert!(run.max_group.is_none());
        assert!(run.runs[0].rows.max_group.is_none());
        assert!(run.compact.get().is_none());
        assert_eq!((&run).rows().collect::<Vec<_>>(), expected);
        assert_eq!(run.items(), expected.iter().map(|pair| pair.1).collect::<Vec<_>>());
        assert_eq!(run.stat_max_group(), None);
    }

    #[test]
    fn run_statistics_match_occurrence_oracle_across_merge_publications() {
        for ratio in [2, 4, 8] {
            for batch_size in [1, 17, 128, 1024] {
                let mut run = RadixRun::<i64, usize>::with_ratio([], ratio);
                assert_eq!(run.stat_rows(), Some(0));
                assert_eq!(run.stat_keys(), Some(0));
                assert_eq!(run.stat_max_group(), Some(0));
                assert_eq!(run.stat_runs(), Some(0));
                assert_eq!(run.key_bounds(), None);
                let input: Vec<_> = (0..1024).map(|i| {
                    let key = match i {
                        0 => i64::MIN,
                        1 => i64::MAX,
                        _ if i % 3 == 0 => 7,
                        _ => ((i * 73) % 127) as i64 - 63,
                    };
                    (key, i)
                }).collect();
                let mut oracle = std::collections::BTreeMap::<i64, u64>::new();
                let mut occurrences = 0_u64;
                for batch in input.chunks(batch_size) {
                    run.merge(batch.iter().copied());
                    for &(key, _) in batch {
                        *oracle.entry(key).or_default() += 1;
                        occurrences += 1;
                    }
                    assert_eq!(run.stat_rows(), Some(occurrences));
                    assert_eq!(run.stat_runs(), Some(run.runs.len() as u64));
                    assert_eq!(run.key_bounds(), oracle.first_key_value().zip(oracle.last_key_value())
                        .map(|((lo, _), (hi, _))| (lo, hi)));
                    let single = run.runs.len() == 1;
                    assert_eq!(run.stat_keys(), single.then_some(oracle.len() as u64));
                    assert_eq!(run.stat_max_group(), single.then(|| *oracle.values().max().unwrap()));
                    assert!(run.compact.get().is_none());
                }
                run.items();
                assert_eq!(run.stat_keys(), Some(oracle.len() as u64));
                assert_eq!(run.stat_max_group(), oracle.values().copied().max());
            }
        }
    }

    #[test]
    fn disabled_statistics_preserve_columnar_occurrences_across_merge_publications() {
        use crate::BuildWithStatistics;

        let input: Vec<_> = (0_u64..4097)
            .map(|i| (((i * 73) % 2053), (i, i + 7)))
            .collect();
        for batch_size in [1, 97, 1024, input.len()] {
            let mut run = RadixRun::<_, _, (Vec<u64>, Vec<u64>)>::build_with_statistics::<true, false>(
                std::iter::empty(),
            );
            assert_eq!(run.stat_max_group(), None);
            run.merge([]);
            assert_eq!(run.stat_max_group(), None);
            let mut published = 0;
            for batch in input.chunks(batch_size) {
                run.merge(batch.iter().copied());
                published += batch.len();
                assert_eq!(run.stat_rows(), Some(published as u64));
                assert_eq!(run.stat_max_group(), None);
                assert!(run.runs.iter().all(|segment| segment.rows.max_group.is_none()));
                assert!(run.key_count.get().is_none());
                assert!(run.compact.get().is_none());
                let mut expected = input[..published].to_vec();
                expected.sort_by_key(|pair| pair.0);
                assert_eq!((&run).rows().collect::<Vec<_>>(), expected);
            }
            let mut expected = input.clone();
            expected.sort_by_key(|pair| pair.0);
            assert_eq!(run.items(), expected.iter().map(|pair| pair.1).collect::<Vec<_>>());
            assert_eq!(run.stat_keys(), Some(2053));
            assert_eq!(run.stat_max_group(), None);
            assert!(run.max_group.is_none());
        }
    }

    #[test]
    fn metadata_and_borrowed_filters_never_read_unselected_payload_columns() {
        let mut run = RadixRun::<u64, (u64, u64), ObservedColumns>::from_column_rows(
            (0..128).map(|key| (key, (key, key + 1000))), 4,
        );
        run.merge([(7, (7, 2000)), (u64::MAX, (0, 3000))]);
        let view = crate::RunFilterView::new(&run, |key| key);
        for key in 0..130 {
            assert_eq!(view.test(&key), key < 128);
            assert_eq!(view.any_in(&key, &key), key < 128);
        }
        assert_eq!(run.stat_rows(), Some(130));
        assert_eq!(run.stat_keys(), None);
        assert_eq!(run.stat_max_group(), None);
        assert_eq!(run.key_bounds(), Some((&0, &u64::MAX)));
        let selected: Vec<_> = run.column_groups(&7)
            .flat_map(|(columns, range)| columns.columns.0[range].iter().copied())
            .collect();
        assert_eq!(selected, [7, 7]);
        assert!(run.runs.iter().all(|segment| segment.rows.items.reads.get() == 0));
        assert!(run.compact.get().is_none());
        assert!(run.key_count.get().is_none());
    }

    #[test]
    fn retained_single_run_expansion_searches_the_directory_once() {
        let run =
            RadixRun::from_pairs((0_u64..200).flat_map(|key| (0..16).map(move |item| (key, item))));
        DIRECTORY_SEARCHES.with(|searches| searches.set(0));
        let saved = (&run).lookup(&64);
        assert_eq!(DIRECTORY_SEARCHES.with(|searches| searches.get()), 1);
        assert_eq!(saved.len(), 16);
        for position in 0..16 {
            let mut cursor = saved;
            assert_eq!(cursor.nth(position), Some(position));
            assert_eq!(cursor.len(), 15 - position);
        }
        let mut exhausted = saved;
        assert_eq!(exhausted.nth(usize::MAX), None);
        assert_eq!(DIRECTORY_SEARCHES.with(|searches| searches.get()), 1);
        DIRECTORY_SEARCHES.with(|searches| searches.set(0));
        assert_eq!(run.range(&64), (64 * 16, 65 * 16));
        assert_eq!(DIRECTORY_SEARCHES.with(|searches| searches.get()), 1);
    }

    #[test]
    fn merged_key_scans_merge_lazily_and_cache_only_the_count() {
        thread_local! {
            static ENCODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        }
        #[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Debug)]
        struct Key(u64);
        impl OrderKey for Key {
            type Ordinal = u64;
            fn ordinal(&self) -> u64 {
                ENCODINGS.with(|count| count.set(count.get() + 1));
                self.0
            }
        }
        let mut run = RadixRun::from_pairs((0..256).map(|key| (Key(key), key)));
        run.merge([(Key(1024), 1024)]);
        ENCODINGS.with(|count| count.set(0));
        assert_eq!(run.keys().next(), Some(&Key(0)));
        assert!(
            ENCODINGS.with(|count| count.get()) <= run.run_count() + 1,
            "the first key must not force or sort the full key relation"
        );
        ENCODINGS.with(|count| count.set(0));
        assert_eq!(run.len(), 257);
        let counted = ENCODINGS.with(|count| count.get());
        assert!(counted > 0);
        assert_eq!(crate::Keys::key_count(&run), 257);
        assert_eq!(ENCODINGS.with(|count| count.get()), counted);
        run.merge([(Key(1025), 1025)]);
        assert_eq!(run.len(), 258);
        assert_eq!(
            run.keys().map(|key| key.0).collect::<Vec<_>>(),
            (0..256).chain([1024, 1025]).collect::<Vec<_>>()
        );
    }

    struct ObservedColumns {
        columns: (Vec<u64>, Vec<u64>),
        reads: std::cell::Cell<usize>,
    }

    impl ItemColumns for ObservedColumns {
        type Row = (u64, u64);

        fn with_capacity(rows: usize) -> Self {
            Self {
                columns: ItemColumns::with_capacity(rows),
                reads: std::cell::Cell::new(0),
            }
        }
        fn len(&self) -> usize {
            self.columns.len()
        }
        fn push(&mut self, row: Self::Row) {
            self.columns.push(row);
        }
        fn item(&self, position: usize) -> Self::Row {
            self.reads.set(self.reads.get() + 1);
            self.columns.item(position)
        }
        fn extend_range(&mut self, source: &Self, range: std::ops::Range<usize>) {
            assert_eq!(
                source.reads.get(),
                0,
                "merge must not materialize source rows"
            );
            self.columns.extend_range(&source.columns, range);
        }
    }

    #[test]
    fn columnar_merges_and_key_reads_do_not_materialize_items() {
        let mut run = RadixRun::<u64, (u64, u64), ObservedColumns>::from_column_rows([], 4);
        let input: Vec<_> = (0..6001).map(|i| (i % 2053, (i, i + 7))).collect();
        for batch in input.chunks(97) {
            run.merge(batch.iter().copied());
        }
        assert_eq!(run.len(), 2053);
        for key in [0, 1023, 1024, 2052, 2053, u64::MAX] {
            let expected: Vec<_> = input
                .iter()
                .filter_map(|&(k, row)| (k == key).then_some(row))
                .collect();
            assert_eq!(run.count(&key), expected.len());
            assert_eq!(run.contains(&key), !expected.is_empty());
            let (begin, end) = run.range(&key);
            assert_eq!(end - begin, expected.len());
            let first: Vec<_> = run
                .column_groups(&key)
                .flat_map(|(columns, range)| columns.columns.0[range].iter().copied())
                .collect();
            assert_eq!(first, expected.iter().map(|row| row.0).collect::<Vec<_>>());
        }
        assert!(
            run.runs
                .iter()
                .all(|segment| segment.rows.items.reads.get() == 0)
        );
        assert!(run.compact.get().is_none());
        let mut expected = input;
        expected.sort_by_key(|row| row.0);
        assert_eq!(
            run.items(),
            expected.iter().map(|row| row.1).collect::<Vec<_>>()
        );
    }

    #[test]
    fn merge_reads_unmerged_runs_without_materializing_items() {
        let mut run = RadixRun::with_ratio((0_u64..128).map(|key| (key, key)), 4);
        run.merge([(64, 1000), (200, 2000)]);
        assert!(run.run_count() > 1);
        let cursor = (&run).lookup(&64);
        assert_eq!(cursor.collect::<Vec<_>>(), [64, 1000]);
        assert_eq!(cursor.collect::<Vec<_>>(), [64, 1000]);
        assert_eq!(run.count(&64), 2);
        assert_eq!(run.count(&129), 0);
        assert!(run.contains(&200));
        assert!(!run.contains(&199));
        assert_eq!(run.range(&64), (64, 66));
        assert_eq!(run.keys().count(), 129);
        assert!(
            run.compact.get().is_none(),
            "key-only and cursor reads must not compact items"
        );
        assert_eq!(&run.items()[64..66], [64, 1000]);
        run.merge([(64, 3000)]);
        assert!(
            run.compact.get().is_none(),
            "a merge invalidates the contiguous snapshot"
        );
        assert_eq!((&run).lookup(&64).collect::<Vec<_>>(), [64, 1000, 3000]);
    }

    #[test]
    fn merge_path_merges_preserve_stable_groups_across_partitions() {
        let input: Vec<_> = (0_u64..9001).map(|i| ((i * 7919) % 4099, i)).collect();
        for ratio in [2, 4, 8] {
            for batch_size in [1, 127, 2048, input.len()] {
                let mut run = RadixRun::with_ratio([], ratio);
                for batch in input.chunks(batch_size) {
                    run.merge(batch.iter().copied());
                    for pair in run.runs.windows(2) {
                        assert!(pair[0].rows.items.len() / ratio >= pair[1].rows.items.len());
                    }
                }
                let mut expected = input.clone();
                expected.sort_by_key(|row| row.0);
                assert_eq!((&run).rows().collect::<Vec<_>>(), expected);
                for key in [0, 1, 1023, 1024, 2048, 4098, 4099, u64::MAX] {
                    let values: Vec<_> = input
                        .iter()
                        .filter_map(|&(k, v)| (k == key).then_some(v))
                        .collect();
                    assert_eq!((&run).lookup(&key).collect::<Vec<_>>(), values);
                    assert_eq!(run.count(&key), values.len());
                    assert_eq!(run.contains(&key), !values.is_empty());
                    let (begin, end) = run.range(&key);
                    assert_eq!(&run.items()[begin..end], values);
                }
            }
        }
    }

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
                RadixRunView::from_parts(&keys, &offsets, &items, &points, &radix, metadata, &state)
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
        for keys in [Vec::new(), vec![u64::MAX], vec![0, 1 << 63, u64::MAX]] {
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
        let membership = run.runs[0]
            .state
            .membership
            .as_ref()
            .expect("bounded dense domain");
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
                run.runs[0].state.membership.is_none(),
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
