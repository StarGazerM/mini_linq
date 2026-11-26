//! CPU reads of word runs (CUDA `radix_run::device`) over the shared arrays.
//!
//! A [`WordRunView`] borrows one run's keys, offsets, order and directory in
//! place; nothing is copied. Keys passed in are `lanes` words in the run's
//! lane layout. Slots index distinct keys; item positions index `order`.
//! [`WordRunsView`] visits several runs oldest first, as CUDA reads do.

use std::cmp::Ordering;

use crate::{
    Error, Result,
    words::{MAX_LANES, MAX_RADIX_BITS, OffsetWidth, Offsets, WordRun, WordRunParams},
};

/// A key widened to [`MAX_LANES`] lanes, least-significant lane first.
pub type Wide = [u64; MAX_LANES];

/// Lane `lane` (0 least significant) of a key in either memory layout.
#[inline(always)]
pub fn lane(key: &[u64], ms_first: bool, lane: usize) -> u64 {
    if ms_first {
        key[key.len() - 1 - lane]
    } else {
        key[lane]
    }
}

/// Unsigned order of two equal-length keys in the same layout.
#[inline(always)]
pub fn compare(a: &[u64], b: &[u64], ms_first: bool) -> Ordering {
    debug_assert_eq!(a.len(), b.len());
    if ms_first {
        a.cmp(b)
    } else {
        a.iter().rev().cmp(b.iter().rev())
    }
}

/// `a - b` with borrow (wrapping when `a < b`), widened least-significant first.
#[inline(always)]
pub fn difference(a: &[u64], b: &[u64], ms_first: bool) -> Wide {
    debug_assert!(a.len() == b.len() && a.len() <= MAX_LANES);
    let mut output = [0; MAX_LANES];
    let mut borrow = false;
    for (index, slot) in output.iter_mut().enumerate().take(a.len()) {
        let (partial, first) = lane(a, ms_first, index).overflowing_sub(lane(b, ms_first, index));
        let (value, second) = partial.overflowing_sub(borrow as u64);
        *slot = value;
        borrow = first | second;
    }
    output
}

/// Number of significant bits, zero for the zero word.
#[inline(always)]
pub fn significant_bits(word: &Wide) -> u32 {
    word.iter()
        .rposition(|&value| value != 0)
        .map_or(0, |index| {
            index as u32 * 64 + 64 - word[index].leading_zeros()
        })
}

/// Bits `[shift, shift + bits)` of `word`; requires `1 <= bits <= 64` and
/// `shift + bits <= 64 * MAX_LANES`.
#[inline(always)]
pub fn extract(word: &Wide, shift: u32, bits: u32) -> u64 {
    debug_assert!((1..=64).contains(&bits) && (shift + bits) as usize <= 64 * MAX_LANES);
    let index = (shift / 64) as usize;
    let offset = shift % 64;
    let mut value = word[index] >> offset;
    if offset != 0 && bits > 64 - offset {
        value |= word[index + 1] << (64 - offset);
    }
    value & (u64::MAX >> (64 - bits))
}

/// Directory bucket of `key` in `[min, max]`: the top `radix_bits` (>= 1)
/// bits of `key - min` at the span's resolution. Monotone in `key`.
#[inline(always)]
pub fn prefix(key: &[u64], min: &[u64], max: &[u64], ms_first: bool, radix_bits: u32) -> u64 {
    let shift = significant_bits(&difference(max, min, ms_first)).saturating_sub(radix_bits);
    extract(&difference(key, min, ms_first), shift, radix_bits)
}

/// One borrowed run. Invariants (checked by [`WordRunView::from_parts`]):
/// keys strictly increasing, `offsets[0] = 0`, `offsets[key_count] = len`,
/// directory empty or `2^radix_bits + 1` exact bucket lower bounds.
#[derive(Clone, Copy, Debug)]
pub struct WordRunView<'a> {
    keys: &'a [u64],
    offsets: Offsets<'a>,
    order: &'a [u32],
    directory: Offsets<'a>,
    params: WordRunParams,
}

impl<'a> From<&'a WordRun> for WordRunView<'a> {
    fn from(run: &'a WordRun) -> Self {
        Self {
            keys: run.keys(),
            offsets: run.offsets(),
            order: run.order(),
            directory: run.directory(),
            params: run.params(),
        }
    }
}

fn width(values: Offsets<'_>) -> OffsetWidth {
    match values {
        Offsets::U32(_) => OffsetWidth::U32,
        Offsets::U64(_) => OffsetWidth::U64,
    }
}

impl<'a> WordRunView<'a> {
    /// A view over raw run arrays. Shapes, carriers and the terminal offset
    /// are checked; key order and directory contents are trusted, as CUDA.
    pub fn from_parts(
        keys: &'a [u64],
        offsets: Offsets<'a>,
        order: &'a [u32],
        directory: Offsets<'a>,
        params: WordRunParams,
    ) -> Result<Self> {
        let lanes = params.lanes;
        if lanes == 0 || lanes > MAX_LANES || !keys.len().is_multiple_of(lanes) {
            return Err(Error(
                "word run view keys must be whole keys of 1..=8 lanes".into(),
            ));
        }
        if params.radix_bits > MAX_RADIX_BITS {
            return Err(Error("word run view directory bits out of range".into()));
        }
        if width(offsets) != params.offsets || width(directory) != params.offsets {
            return Err(Error("word run view has another offset carrier".into()));
        }
        let key_count = keys.len() / lanes;
        if offsets.len() != key_count + 1
            || offsets.get(0) != 0
            || offsets.get(key_count) != order.len() as u64
        {
            return Err(Error(
                "word run view offsets must be key_count + 1 bounds of order".into(),
            ));
        }
        let entries = if params.radix_bits == 0 {
            0
        } else {
            (1usize << params.radix_bits) + 1
        };
        if directory.len() != entries {
            return Err(Error(
                "word run view directory must hold 2^radix_bits + 1 entries".into(),
            ));
        }
        Ok(Self {
            keys,
            offsets,
            order,
            directory,
            params,
        })
    }

    pub fn params(&self) -> WordRunParams {
        self.params
    }

    /// Distinct keys.
    pub fn key_count(&self) -> usize {
        self.offsets.len() - 1
    }

    /// Items (source rows).
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn keys(&self) -> &'a [u64] {
        self.keys
    }

    pub fn offsets(&self) -> Offsets<'a> {
        self.offsets
    }

    pub fn order(&self) -> &'a [u32] {
        self.order
    }

    pub fn directory(&self) -> Offsets<'a> {
        self.directory
    }

    /// The key at `slot < key_count`.
    #[inline(always)]
    pub fn key(&self, slot: usize) -> &'a [u64] {
        let lanes = self.params.lanes;
        &self.keys[slot * lanes..][..lanes]
    }

    /// Item position of `slot`'s group start; `slot <= key_count`.
    #[inline(always)]
    pub fn offset(&self, slot: usize) -> usize {
        self.offsets.get(slot) as usize
    }

    /// Item positions `[begin, end)` of `slot < key_count`.
    #[inline(always)]
    pub fn group(&self, slot: usize) -> (usize, usize) {
        (self.offset(slot), self.offset(slot + 1))
    }

    /// Source rows of `slot < key_count`, in stable order.
    #[inline(always)]
    pub fn rows(&self, slot: usize) -> &'a [u32] {
        let (begin, end) = self.group(slot);
        &self.order[begin..end]
    }

    #[inline(always)]
    fn check_key(&self, key: &[u64]) {
        assert_eq!(key.len(), self.params.lanes, "word run probe lane count");
    }

    /// Lower-bound slot of `key` and whether it is present. Keys outside
    /// `[min, max]` resolve without the directory; inside, the directory
    /// bucket bounds the binary search.
    #[inline]
    pub fn search(&self, key: &[u64]) -> (usize, bool) {
        self.check_key(key);
        let len = self.key_count();
        if len == 0 {
            return (0, false);
        }
        let ms_first = self.params.ms_first;
        let min = self.key(0);
        if compare(key, min, ms_first).is_lt() {
            return (0, false);
        }
        let max = self.key(len - 1);
        match compare(key, max, ms_first) {
            Ordering::Greater => return (len, false),
            Ordering::Equal => return (len - 1, true),
            Ordering::Less => {}
        }
        let (low, high) = if self.params.radix_bits == 0 {
            (0, len)
        } else {
            let bucket = prefix(key, min, max, ms_first, self.params.radix_bits) as usize;
            (
                self.directory.get(bucket) as usize,
                self.directory.get(bucket + 1) as usize,
            )
        };
        self.bounded(key, low, high)
    }

    /// Lower bound by plain binary search over all slots, ignoring the
    /// directory; equal to [`search`](Self::search) for every key.
    pub fn binary_search(&self, key: &[u64]) -> (usize, bool) {
        self.check_key(key);
        self.bounded(key, 0, self.key_count())
    }

    #[inline(always)]
    fn bounded(&self, key: &[u64], low: usize, high: usize) -> (usize, bool) {
        let ms_first = self.params.ms_first;
        let present =
            |slot: usize| slot < self.key_count() && compare(self.key(slot), key, ms_first).is_eq();
        if low == high {
            return (low, present(low));
        }
        // Halving lower bound (a conditional move for one lane): slots in
        // `[low, base)` are < key and the answer lies in `[base, base + size]`.
        let (mut base, mut size) = (low, high - low);
        if self.params.lanes == 1 {
            let (keys, probe) = (&self.keys[..high], key[0]);
            while size > 1 {
                let half = size / 2;
                let middle = base + half;
                base = if keys[middle] < probe { middle } else { base };
                size -= half;
            }
            let slot = base + (keys[base] < probe) as usize;
            return (slot, present(slot));
        }
        while size > 1 {
            let half = size / 2;
            let middle = base + half;
            base = if compare(self.key(middle), key, ms_first).is_lt() {
                middle
            } else {
                base
            };
            size -= half;
        }
        match compare(self.key(base), key, ms_first) {
            Ordering::Less => (base + 1, present(base + 1)),
            order => (base, order.is_eq()),
        }
    }

    pub fn contains(&self, key: &[u64]) -> bool {
        self.search(key).1
    }

    /// Occurrences of `key`.
    pub fn count(&self, key: &[u64]) -> usize {
        let (begin, end) = self.range(key);
        end - begin
    }

    /// Item positions of `key`, `(len, len)` when absent (CUDA `group`).
    #[inline]
    pub fn range(&self, key: &[u64]) -> (usize, usize) {
        match self.search(key) {
            (slot, true) => self.group(slot),
            _ => (self.len(), self.len()),
        }
    }

    /// Slots `[first, last)` of the keys in inclusive `[low, high]`; empty
    /// (`first == last`) when `low > high`.
    pub fn interval(&self, low: &[u64], high: &[u64]) -> (usize, usize) {
        let (first, _) = self.search(low);
        let (last, found) = self.search(high);
        let last = last + found as usize;
        (first, last.max(first))
    }

    /// Item positions `[begin, end)` of the keys in inclusive `[low, high]`.
    pub fn interval_items(&self, low: &[u64], high: &[u64]) -> (usize, usize) {
        let (first, last) = self.interval(low, high);
        (self.offset(first), self.offset(last))
    }
}

/// Several runs, oldest first, sharing one key layout. Item positions are
/// those of the stable concatenation of all runs merged (older rows first
/// within a key), the order CUDA's ratio-law merges preserve.
#[derive(Clone, Copy, Debug)]
pub struct WordRunsView<'r, 'a> {
    runs: &'r [WordRunView<'a>],
    items: usize,
}

impl<'r, 'a> WordRunsView<'r, 'a> {
    pub fn new(runs: &'r [WordRunView<'a>]) -> Result<Self> {
        if let Some(first) = runs.first() {
            let (lanes, ms_first) = (first.params.lanes, first.params.ms_first);
            if runs
                .iter()
                .any(|run| run.params.lanes != lanes || run.params.ms_first != ms_first)
            {
                return Err(Error(
                    "word runs must share one lane count and layout".into(),
                ));
            }
        }
        Ok(Self {
            runs,
            items: runs.iter().map(WordRunView::len).sum(),
        })
    }

    pub fn runs(&self) -> &'r [WordRunView<'a>] {
        self.runs
    }

    /// Items across all runs.
    pub fn len(&self) -> usize {
        self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items == 0
    }

    pub fn contains(&self, key: &[u64]) -> bool {
        self.runs.iter().any(|run| run.contains(key))
    }

    /// Occurrences of `key` across all runs.
    pub fn count(&self, key: &[u64]) -> usize {
        self.runs.iter().map(|run| run.count(key)).sum()
    }

    /// Merged item positions of `key`, `(len, len)` when absent (CUDA `range`).
    pub fn range(&self, key: &[u64]) -> (usize, usize) {
        let (mut begin, mut count) = (0, 0);
        for run in self.runs {
            let (slot, found) = run.search(key);
            let start = run.offset(slot);
            begin += start;
            if found {
                count += run.offset(slot + 1) - start;
            }
        }
        if count == 0 {
            (self.items, self.items)
        } else {
            (begin, begin + count)
        }
    }

    /// Groups (per-run slots, summed) and items of the keys in inclusive
    /// `[low, high]` (CUDA `IntervalProbe`).
    pub fn interval_counts(&self, low: &[u64], high: &[u64]) -> (usize, usize) {
        self.runs.iter().fold((0, 0), |(groups, items), run| {
            let (first, last) = run.interval(low, high);
            (
                groups + last - first,
                items + run.offset(last) - run.offset(first),
            )
        })
    }

    /// Whether a run older than `before` holds `key`.
    #[inline]
    fn older(&self, before: usize, key: &[u64]) -> bool {
        self.runs[..before].iter().any(|run| run.contains(key))
    }

    /// Distinct keys in inclusive `[low, high]` over the union of runs; each
    /// key counts once, in its oldest run (CUDA `distinct`).
    pub fn distinct(&self, low: &[u64], high: &[u64]) -> usize {
        let mut total = 0;
        for (index, run) in self.runs.iter().enumerate() {
            let (first, last) = run.interval(low, high);
            if index == 0 {
                total += last - first;
            } else {
                total += (first..last)
                    .filter(|&slot| !self.older(index, run.key(slot)))
                    .count();
            }
        }
        total
    }

    /// The `ordinal`-th key of [`distinct`](Self::distinct)'s enumeration
    /// (runs oldest first, keys ascending within a run, each key in its
    /// oldest run) as `(run, slot)`; `None` past the last (CUDA `distinct_at`).
    pub fn distinct_at(
        &self,
        low: &[u64],
        high: &[u64],
        mut ordinal: usize,
    ) -> Option<(usize, usize)> {
        for (index, run) in self.runs.iter().enumerate() {
            let (first, last) = run.interval(low, high);
            if index == 0 {
                if ordinal < last - first {
                    return Some((0, first + ordinal));
                }
                ordinal -= last - first;
                continue;
            }
            for slot in first..last {
                if !self.older(index, run.key(slot)) {
                    if ordinal == 0 {
                        return Some((index, slot));
                    }
                    ordinal -= 1;
                }
            }
        }
        None
    }
}
