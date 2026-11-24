//! Whole-stream canonicalization by ordering, never by hashing.
//!
//! Comparators must be pure total orders whose `Equal` denotes relational
//! equality. Generated callers compare borrowed fields (grouping wide tuples),
//! so neither a row-level `Ord` implementation nor cloning a row is required.
//! Rust checks the compared fields' ordering bounds; the compiler does not
//! inspect their types. See `doc/STORAGE.md` §13 and planning contract 7.
//! [`Packed`] orders by packed `OrderKey` words instead of the comparator,
//! under the authored premise that every column has an `OrderKey`.
//!
//! These operations do not increment hash or distinct-insertion counters:
//! those counters measure actual hash computations and set insertion attempts,
//! not abstract canonicalization work. Sorting performs neither operation.

use std::cmp::Ordering;
use std::fmt;
use std::slice;
use std::vec;

use crate::OrderKey;

/// Sorted-array provider: the default type of a canonical rows record.
///
/// Its build sorts the whole stream and removes adjacent duplicates
/// (PL-Distinct). A `host` canonical reference uses checked prefix runs
/// instead; the marker itself is not a sortedness or uniqueness witness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sort;

impl Sort {
    /// Host build of a canonical rows record typed `Sort` (`r canon = rows`), forwarding to [`sort`].
    pub fn sort<I, C>(rows: I, compare: C) -> Sorted<I::IntoIter, C>
    where
        I: IntoIterator,
        C: FnMut(&I::Item, &I::Item) -> Ordering,
    {
        sort(rows, compare)
    }

    /// Task build of a canonical rows record typed `Sort` (`r canon = rows`), forwarding to [`par_sort`].
    pub fn par_sort<I, C>(rows: I, compare: C) -> vec::IntoIter<I::Item>
    where
        I: IntoIterator,
        I::Item: Send,
        C: Fn(&I::Item, &I::Item) -> Ordering + Sync,
    {
        par_sort(rows, compare)
    }
}

/// Collect, sort and adjacent-unique one complete stream on first demand.
///
/// The same comparator determines ordering and duplicate identity. Rows move
/// in place; no `Clone`, `Hash`, or row-level `Ord` bound is imposed. Nothing
/// is read before the first `next`: the barrier is the first answer, not
/// construction, so an undemanded result never evaluates its input.
pub fn sort<I, C>(rows: I, compare: C) -> Sorted<I::IntoIter, C>
where
    I: IntoIterator,
    C: FnMut(&I::Item, &I::Item) -> Ordering,
{
    Sorted::Pending(rows.into_iter(), compare)
}

/// Deferred whole-stream canonicalization returned by [`sort`].
pub enum Sorted<I: Iterator, C> {
    Pending(I, C),
    Ready(vec::IntoIter<I::Item>),
}

impl<I, C> Iterator for Sorted<I, C>
where
    I: Iterator,
    C: FnMut(&I::Item, &I::Item) -> Ordering,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<I::Item> {
        if let Sorted::Pending(rows, compare) = self {
            let mut rows: Vec<_> = rows.collect();
            rows.sort_unstable_by(&mut *compare);
            rows.dedup_by(|a, b| compare(a, b) == Ordering::Equal);
            *self = Sorted::Ready(rows.into_iter());
        }
        match self {
            Sorted::Ready(rows) => rows.next(),
            Sorted::Pending(..) => unreachable!(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            // Deduplication keeps at least one row of a nonempty input.
            Sorted::Pending(rows, _) => {
                let (lower, upper) = rows.size_hint();
                (lower.min(1), upper)
            }
            Sorted::Ready(rows) => rows.size_hint(),
        }
    }
}

/// Parallel comparison sort followed by an in-place adjacent-unique pass.
///
/// The caller supplies an ordinary stream or an already collected task result.
/// Sorting uses the current Rayon pool; no second row collection is needed
/// when `rows` is a `Vec`. Duplicate removal is sequential.
pub fn par_sort<I, C>(rows: I, compare: C) -> vec::IntoIter<I::Item>
where
    I: IntoIterator,
    I::Item: Send,
    C: Fn(&I::Item, &I::Item) -> Ordering + Sync,
{
    use rayon::prelude::*;

    let mut rows: Vec<_> = rows.into_iter().collect();
    rows.par_sort_unstable_by(&compare);
    rows.dedup_by(|a, b| compare(a, b) == Ordering::Equal);
    rows.into_iter()
}

/// Sorted-run provider keyed by the packed [`OrderKey`] of every column.
///
/// Formal rule PL-PackedCanon: an ordinary `Distinct` provider for a canonical
/// rows record, selected only under authored advice whose premise is that
/// every row column has an `OrderKey` (rustc checks [`PackedRow`]). Its build
/// collects the stream while measuring each column's ordinal span, packs the
/// span-relative ordinals most-significant-column first into one word and
/// sorts `(key, position)` words; the build's rows are then read in word order,
/// skipping equal keys, without moving or copying the collected rows. Packed
/// rows are `Copy`, like index-run payloads. Packing is injective and
/// lexicographic, so the order and duplicate identity are exactly those of the
/// tuple order that the printed comparator denotes (IX-OrderKey). When the
/// measured spans need more than 128 bits, or positions exceed `u32`, the
/// build falls back to the comparator, exactly as [`Sort`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Packed;

impl Packed {
    /// Host build of a canonical rows record typed `Packed`; lazy like [`sort`].
    pub fn sort<I, C>(rows: I, compare: C) -> PackedSorted<I::IntoIter, C>
    where
        I: IntoIterator,
        I::Item: PackedRow,
        C: FnMut(&I::Item, &I::Item) -> Ordering,
    {
        let rows = rows.into_iter();
        PackedSorted::Pending(rows, compare)
    }

    /// Task build of a canonical rows record typed `Packed`. Key words are
    /// sorted in parallel on the current Rayon pool; measurement and the
    /// ordered read are sequential.
    pub fn par_sort<I, C>(rows: I, compare: C) -> PackedRows<I::Item>
    where
        I: IntoIterator,
        I::Item: PackedRow + Send,
        C: Fn(&I::Item, &I::Item) -> Ordering + Sync,
    {
        let (rows, spans) = collect_measured(rows);
        packed_rows(rows, &spans, true, |rows| {
            use rayon::prelude::*;
            rows.par_sort_unstable_by(&compare);
            rows.dedup_by(|a, b| compare(a, b) == Ordering::Equal);
        })
    }
}

/// A row whose every column has an [`OrderKey`]: the premise of [`Packed`].
///
/// `ordinals` visits each column's ordinal, widened to `u128`, in column
/// order. Implemented for tuples of `Copy` columns of arity 1 through 24;
/// rustc checks the bound at the generated build, the compiler never inspects
/// column types.
pub trait PackedRow: Copy {
    const WIDTH: usize;
    fn ordinals(&self, visit: impl FnMut(usize, u128));
}

macro_rules! packed_row {
    ($($T:ident:$field:tt),+) => {
        impl<$($T: OrderKey + Copy),+> PackedRow for ($($T,)+) {
            const WIDTH: usize = [$($field),+].len();
            #[inline]
            fn ordinals(&self, mut visit: impl FnMut(usize, u128)) {
                $(visit($field, self.$field.ordinal().into());)+
            }
        }
    };
}

packed_row!(A:0);
packed_row!(A:0, B:1);
packed_row!(A:0, B:1, C:2);
packed_row!(A:0, B:1, C:2, D:3);
packed_row!(A:0, B:1, C:2, D:3, E:4);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13, O:14);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13, O:14, P:15);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13, O:14, P:15, Q:16);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13, O:14, P:15, Q:16, R:17);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13, O:14, P:15, Q:16, R:17, S:18);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13, O:14, P:15, Q:16, R:17, S:18, T:19);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13, O:14, P:15, Q:16, R:17, S:18, T:19, U:20);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13, O:14, P:15, Q:16, R:17, S:18, T:19, U:20, V:21);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13, O:14, P:15, Q:16, R:17, S:18, T:19, U:20, V:21, W:22);
packed_row!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11, M:12, N:13, O:14, P:15, Q:16, R:17, S:18, T:19, U:20, V:21, W:22, X:23);

const MAX_PACKED_WIDTH: usize = 24;

/// Per-column ordinal spans measured while the stream is collected.
struct Spans {
    min: [u128; MAX_PACKED_WIDTH],
    max: [u128; MAX_PACKED_WIDTH],
}

impl Spans {
    fn new() -> Self {
        Self {
            min: [u128::MAX; MAX_PACKED_WIDTH],
            max: [0; MAX_PACKED_WIDTH],
        }
    }

    #[inline]
    fn observe<T: PackedRow>(&mut self, row: &T) {
        row.ordinals(|column, ordinal| {
            self.min[column] = self.min[column].min(ordinal);
            self.max[column] = self.max[column].max(ordinal);
        });
    }

    /// Bits per column and their total, for a nonempty measured stream.
    fn bits<T: PackedRow>(&self) -> ([u32; MAX_PACKED_WIDTH], u32) {
        let mut bits = [0; MAX_PACKED_WIDTH];
        let mut total = 0;
        for column in 0..T::WIDTH {
            bits[column] = u128::BITS - (self.max[column] - self.min[column]).leading_zeros();
            total += bits[column];
        }
        (bits, total)
    }
}

fn collect_measured<I>(rows: I) -> (Vec<I::Item>, Spans)
where
    I: IntoIterator,
    I::Item: PackedRow,
{
    let rows = rows.into_iter();
    let mut spans = Spans::new();
    let mut collected = Vec::with_capacity(rows.size_hint().0);
    for row in rows {
        spans.observe(&row);
        collected.push(row);
    }
    (collected, spans)
}

/// One sortable word per row: the packed key above the row position.
trait KeyWord: Copy + Ord + Send {
    fn word(key: u128, position: u32, position_bits: u32) -> Self;
    fn position(self, position_bits: u32) -> usize;
    fn same_key(self, other: Self, position_bits: u32) -> bool;
}

impl KeyWord for u64 {
    #[inline]
    fn word(key: u128, position: u32, position_bits: u32) -> Self {
        ((key as u64) << position_bits) | u64::from(position)
    }
    #[inline]
    fn position(self, position_bits: u32) -> usize {
        (self & ((1 << position_bits) - 1)) as usize
    }
    #[inline]
    fn same_key(self, other: Self, position_bits: u32) -> bool {
        self >> position_bits == other >> position_bits
    }
}

impl KeyWord for u128 {
    #[inline]
    fn word(key: u128, position: u32, position_bits: u32) -> Self {
        (key << position_bits) | u128::from(position)
    }
    #[inline]
    fn position(self, position_bits: u32) -> usize {
        (self & ((1 << position_bits) - 1)) as usize
    }
    #[inline]
    fn same_key(self, other: Self, position_bits: u32) -> bool {
        self >> position_bits == other >> position_bits
    }
}

/// A 128-bit key as two words, followed by the position: 24 bytes per row.
impl KeyWord for (u64, u64, u32) {
    #[inline]
    fn word(key: u128, position: u32, _: u32) -> Self {
        ((key >> 64) as u64, key as u64, position)
    }
    #[inline]
    fn position(self, _: u32) -> usize {
        self.2 as usize
    }
    #[inline]
    fn same_key(self, other: Self, _: u32) -> bool {
        (self.0, self.1) == (other.0, other.1)
    }
}

/// Order measured rows by their packed keys, or by `fallback` when the keys
/// or positions do not fit the supported words.
fn packed_rows<T, F>(mut rows: Vec<T>, spans: &Spans, parallel: bool, fallback: F) -> PackedRows<T>
where
    T: PackedRow,
    F: FnOnce(&mut Vec<T>),
{
    let sorted = |mut rows: Vec<T>, fallback: F| {
        fallback(&mut rows);
        PackedRows(Order::Sorted(rows.into_iter()))
    };
    if rows.len() < 2 {
        return PackedRows(Order::Sorted(rows.into_iter()));
    }
    let (bits, total) = spans.bits::<T>();
    let Ok(last) = u32::try_from(rows.len() - 1) else {
        return sorted(rows, fallback);
    };
    let position_bits = u32::BITS - last.leading_zeros();
    PackedRows(if total + position_bits <= u64::BITS {
        Order::Narrow(Gather::new(rows, spans, &bits, position_bits, parallel))
    } else if total + position_bits <= u128::BITS {
        Order::Wide(Gather::new(rows, spans, &bits, position_bits, parallel))
    } else if total <= u128::BITS {
        Order::Pair(Gather::new(rows, spans, &bits, position_bits, parallel))
    } else {
        fallback(&mut rows);
        Order::Sorted(rows.into_iter())
    })
}

/// Rows read in sorted key-word order, skipping each later equal key.
struct Gather<T, W> {
    rows: Vec<T>,
    words: vec::IntoIter<W>,
    previous: Option<W>,
    position_bits: u32,
}

impl<T: PackedRow, W: KeyWord> Gather<T, W> {
    fn new(
        rows: Vec<T>,
        spans: &Spans,
        bits: &[u32; MAX_PACKED_WIDTH],
        position_bits: u32,
        parallel: bool,
    ) -> Self {
        let mut words: Vec<W> = rows
            .iter()
            .enumerate()
            .map(|(position, row)| {
                let mut key = 0_u128;
                row.ordinals(|column, ordinal| {
                    // A column of all 128 bits is the whole key, so the
                    // shifted-out prefix is zero.
                    key =
                        key.checked_shl(bits[column]).unwrap_or(0) | (ordinal - spans.min[column]);
                });
                W::word(key, position as u32, position_bits)
            })
            .collect();
        if parallel {
            use rayon::prelude::*;
            words.par_sort_unstable();
        } else {
            words.sort_unstable();
        }
        Self {
            rows,
            words: words.into_iter(),
            previous: None,
            position_bits,
        }
    }

    #[inline]
    fn next(&mut self) -> Option<T> {
        loop {
            if let Some(ahead) = self.words.as_slice().get(PREFETCH_DISTANCE) {
                prefetch(&self.rows[ahead.position(self.position_bits)]);
            }
            let word = self.words.next()?;
            let previous = self.previous.replace(word);
            if !previous.is_some_and(|previous| previous.same_key(word, self.position_bits)) {
                return Some(self.rows[word.position(self.position_bits)]);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.words.len();
        (remaining.min(1), Some(remaining))
    }
}

/// Rows ahead in key order whose reads are issued early: the ordered read is
/// a gather, and independent loads hide its memory latency.
const PREFETCH_DISTANCE: usize = 16;

#[inline(always)]
fn prefetch<T>(row: &T) {
    // SAFETY: SSE is part of the x86_64 baseline, and a prefetch hint never
    // faults or accesses memory observably.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
            (row as *const T).cast(),
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = row;
}

enum Order<T> {
    Narrow(Gather<T, u64>),
    Wide(Gather<T, u128>),
    Pair(Gather<T, (u64, u64, u32)>),
    Sorted(vec::IntoIter<T>),
}

/// The distinct rows of a [`Packed`] build, in ascending tuple order.
pub struct PackedRows<T>(Order<T>);

impl<T: PackedRow> Iterator for PackedRows<T> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        match &mut self.0 {
            Order::Narrow(rows) => rows.next(),
            Order::Wide(rows) => rows.next(),
            Order::Pair(rows) => rows.next(),
            Order::Sorted(rows) => rows.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            Order::Narrow(rows) => rows.size_hint(),
            Order::Wide(rows) => rows.size_hint(),
            Order::Pair(rows) => rows.size_hint(),
            Order::Sorted(rows) => rows.size_hint(),
        }
    }
}

impl<T: PackedRow> std::iter::FusedIterator for PackedRows<T> {}

/// Deferred whole-stream build returned by [`Packed::sort`].
pub enum PackedSorted<I: Iterator, C> {
    Pending(I, C),
    Ready(PackedRows<I::Item>),
}

impl<I, C> Iterator for PackedSorted<I, C>
where
    I: Iterator,
    I::Item: PackedRow,
    C: FnMut(&I::Item, &I::Item) -> Ordering,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<I::Item> {
        if let PackedSorted::Pending(..) = self {
            let PackedSorted::Pending(rows, mut compare) = std::mem::replace(
                self,
                PackedSorted::Ready(PackedRows(Order::Sorted(Vec::new().into_iter()))),
            ) else {
                unreachable!()
            };
            let (rows, spans) = collect_measured(rows);
            *self = PackedSorted::Ready(packed_rows(rows, &spans, false, |rows| {
                rows.sort_unstable_by(&mut compare);
                rows.dedup_by(|a, b| compare(a, b) == Ordering::Equal);
            }));
        }
        match self {
            PackedSorted::Ready(rows) => rows.next(),
            PackedSorted::Pending(..) => unreachable!(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            PackedSorted::Pending(rows, _) => {
                let (lower, upper) = rows.size_hint();
                (lower.min(1), upper)
            }
            PackedSorted::Ready(rows) => rows.size_hint(),
        }
    }
}

/// First descending adjacent prefix pair found at load time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefixOrderError {
    /// Zero-based position of the second row in the descending pair.
    pub row: usize,
}

impl fmt::Display for PrefixOrderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "prefix order decreases at row {}", self.row)
    }
}

impl std::error::Error for PrefixOrderError {}

/// Immutable rows paired with the exact prefix comparison checked at load.
///
/// Only [`verify_prefix_order`] constructs this witness. There is no unchecked
/// constructor, mutable row access, or detached proof to apply to other rows.
/// The comparator must remain a pure total order, and keys must not change
/// through interior mutability, just as for other ordered representations.
/// Equal prefixes are allowed: this witness says nothing about uniqueness.
/// The logical row type and layout are unchanged.
///
/// ```compile_fail
/// use cq_storage::dedup::PrefixOrdered;
/// let forged = PrefixOrdered {
///     rows: vec![2, 1],
///     prefix_compare: i32::cmp,
/// };
/// ```
pub struct PrefixOrdered<T, P> {
    rows: Vec<T>,
    prefix_compare: P,
}

impl<T, P> PrefixOrdered<T, P> {
    /// Borrow the checked rows without allowing reordering or replacement.
    pub fn as_slice(&self) -> &[T] {
        &self.rows
    }

    /// An ordinary host scan in the verified prefix order.
    pub fn iter(&self) -> slice::Iter<'_, T> {
        self.rows.iter()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Recover mutable ownership, consuming (and thus discarding) the proof.
    pub fn into_rows(self) -> Vec<T> {
        self.rows
    }
}

/// Verify nondecreasing prefixes in one pass over loaded rows.
///
/// Every adjacent pair is checked exactly once, stopping at the first descent.
/// No sorting, uniqueness assumption, row copying or metadata allocation is
/// involved. Empty and singleton inputs are ordered. The comparator is retained
/// with the immutable rows, so consumers cannot substitute a different prefix.
pub fn verify_prefix_order<T, P>(
    rows: Vec<T>,
    prefix_compare: P,
) -> Result<PrefixOrdered<T, P>, PrefixOrderError>
where
    P: Fn(&T, &T) -> Ordering,
{
    for (index, pair) in rows.windows(2).enumerate() {
        if prefix_compare(&pair[0], &pair[1]) == Ordering::Greater {
            return Err(PrefixOrderError { row: index + 1 });
        }
    }
    Ok(PrefixOrdered {
        rows,
        prefix_compare,
    })
}

/// Canonicalize checked rows by sorting and adjacent-uniquing equal-prefix runs.
///
/// `compare` is the full-row comparator: equality must imply equal prefixes.
/// This never treats prefix order as a uniqueness proof. Only rows within a
/// verified equal-prefix run are sorted, using the existing row allocation.
/// The prefix order is retained; order within each run is given by `compare`.
pub fn ordered<T, P, C>(rows: PrefixOrdered<T, P>, mut compare: C) -> vec::IntoIter<T>
where
    P: Fn(&T, &T) -> Ordering,
    C: FnMut(&T, &T) -> Ordering,
{
    let PrefixOrdered {
        rows,
        prefix_compare,
    } = rows;
    ordered_rows(rows, &prefix_compare, &mut compare)
}

/// Borrowed counterpart of [`ordered`], retaining the checked input for reuse.
///
/// Only references are collected and moved; owned rows are never cloned. The
/// returned references keep the immutable input borrowed for their lifetime.
pub fn ordered_ref<'a, T, P, C>(
    rows: &'a PrefixOrdered<T, P>,
    mut compare: C,
) -> vec::IntoIter<&'a T>
where
    P: Fn(&T, &T) -> Ordering,
    C: FnMut(&T, &T) -> Ordering,
{
    ordered_rows(
        rows.iter().collect(),
        &|a: &&T, b: &&T| (rows.prefix_compare)(a, b),
        &mut |a: &&T, b: &&T| compare(a, b),
    )
}

/// Parallel counterpart of [`ordered_ref`], retaining verified prefix order.
///
/// Each equal-prefix run is sorted in parallel using the current Rayon pool;
/// adjacent duplicate removal is sequential. Only references are collected and
/// moved. The prefix comparator is used on the calling thread, so it need not
/// be `Sync`; rows and the full-row comparator must support shared task access.
pub fn par_ordered_ref<'a, T, P, C>(
    rows: &'a PrefixOrdered<T, P>,
    compare: C,
) -> vec::IntoIter<&'a T>
where
    T: Sync,
    P: Fn(&T, &T) -> Ordering,
    C: Fn(&T, &T) -> Ordering + Sync,
{
    use rayon::prelude::*;

    let mut refs: Vec<_> = rows.iter().collect();
    let mut start = 0;
    while start < refs.len() {
        let mut end = start + 1;
        while end < refs.len() && (rows.prefix_compare)(refs[start], refs[end]) == Ordering::Equal {
            end += 1;
        }
        refs[start..end].par_sort_unstable_by(|a, b| compare(a, b));
        start = end;
    }
    refs.dedup_by(|a, b| {
        (rows.prefix_compare)(a, b) == Ordering::Equal && compare(a, b) == Ordering::Equal
    });
    refs.into_iter()
}

fn ordered_rows<T, P, C>(mut rows: Vec<T>, prefix_compare: &P, compare: &mut C) -> vec::IntoIter<T>
where
    P: Fn(&T, &T) -> Ordering,
    C: FnMut(&T, &T) -> Ordering,
{
    let mut start = 0;
    while start < rows.len() {
        let mut end = start + 1;
        while end < rows.len() && prefix_compare(&rows[start], &rows[end]) == Ordering::Equal {
            end += 1;
        }
        rows[start..end].sort_unstable_by(&mut *compare);
        start = end;
    }
    // Prefix equality is checked too: adjacent-unique is confined to the same
    // verified run, rather than assuming that a prefix alone identifies a row.
    rows.dedup_by(|a, b| {
        prefix_compare(a, b) == Ordering::Equal && compare(a, b) == Ordering::Equal
    });
    rows.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic xorshift stream, so no test dependency is needed.
    fn stream(seed: u64) -> impl FnMut() -> u64 {
        let mut state = seed | 1;
        move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        }
    }

    fn check<T: PackedRow + Ord + Copy + Send + Sync + std::fmt::Debug>(rows: Vec<T>) {
        let expected: Vec<T> = sort(rows.clone(), T::cmp).collect();
        let packed: Vec<T> = Packed::sort(rows.clone(), T::cmp).collect();
        assert_eq!(packed, expected);
        let parallel: Vec<T> = Packed::par_sort(rows, T::cmp).collect();
        assert_eq!(parallel, expected);
    }

    #[test]
    fn packed_order_and_identity_equal_the_comparator() {
        let mut next = stream(7);
        // One word with positions (narrow spans), including duplicates.
        check(
            (0..2000)
                .map(|_| {
                    (
                        (next() % 5) as i64 - 2,
                        (next() % 3) as u32,
                        (next() % 7) as i32,
                    )
                })
                .collect(),
        );
        // A word of key and position above 64 bits.
        check(
            (0..2000)
                .map(|_| (next() as i64 >> 4, (next() % 3) as u32, (next() % 2) as i64))
                .collect(),
        );
        // A 128-bit key beside the position.
        check(
            (0..2000)
                .map(|_| (next() as i64, (next() % 4) as i64, next()))
                .collect(),
        );
        // More than 128 bits: the comparator fallback.
        check(
            (0..500)
                .map(|_| {
                    (
                        next(),
                        next() as i64,
                        next() % 3,
                        (next() % 2) as u128 * u128::MAX,
                    )
                })
                .collect(),
        );
        // Extreme ordinals, singletons, empty input and borrowed columns.
        check(vec![
            (i64::MIN, u64::MAX),
            (i64::MAX, 0),
            (i64::MIN, u64::MAX),
            (0, 1),
        ]);
        check(vec![(1_i64,)]);
        check(Vec::<(i64, i64)>::new());
        let owned: Vec<(i64, u32)> = (0..300)
            .map(|_| ((next() % 9) as i64, (next() % 2) as u32))
            .collect();
        check(owned.iter().map(|(a, b)| (a, b)).collect());
    }

    #[test]
    fn packed_build_is_lazy() {
        let mut built = false;
        let rows = Packed::sort(
            std::iter::once((1_i64, 2_i64)).inspect(|_| built = true),
            |a: &(i64, i64), b: &(i64, i64)| a.cmp(b),
        );
        drop(rows);
        assert!(!built);
    }
}
