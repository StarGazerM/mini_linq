//! Whole-stream canonicalization by comparison, never by hashing.
//!
//! Comparators must be pure total orders whose `Equal` denotes relational
//! equality. Generated callers compare borrowed fields (grouping wide tuples),
//! so neither a row-level `Ord` implementation nor cloning a row is required.
//! Rust checks the compared fields' ordering bounds; the compiler does not
//! inspect their types. See `doc/STORAGE.md` §13 and planning contract 7.
//!
//! These operations do not increment hash or distinct-insertion counters:
//! those counters measure actual hash computations and set insertion attempts,
//! not abstract canonicalization work. Sorting performs neither operation.

use std::cmp::Ordering;
use std::fmt;
use std::slice;
use std::vec;

/// Host catalog capability for comparison-based canonicalization.
///
/// This zero-sized marker records implementation availability at a machine
/// level. The recorded strategy still decides between whole-stream sort and
/// checked prefix runs; the marker is not a sortedness or uniqueness witness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sort;

impl Sort {
    /// Host provider protocol for explicit `sort using Sort`, forwarding to [`sort`].
    pub fn sort<I, C>(rows: I, compare: C) -> Sorted<I::IntoIter, C>
    where
        I: IntoIterator,
        C: FnMut(&I::Item, &I::Item) -> Ordering,
    {
        sort(rows, compare)
    }

    /// Task provider protocol for explicit `sort using Sort`, forwarding to [`par_sort`].
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
