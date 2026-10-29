//! Stable borrowed occurrences and exact predicates. IDs are base-relative;
//! callers must use only live IDs enumerated by that host.
//!
//! DESIGN contract8-positional-reads
//! Owns stable base-relative witnesses and exact borrowed reads (contract 8, STR-Repr/STR-Lineage).
//! Sources: doc/STORAGE.md §§2,3,9,12; Data Blocks §3.4; DuckDB row_group.cpp:1035-1088.
//! DO NOT turn positions into logical columns, copy whole rows to scan, or silently change Eq/Ord.
//! Scan columns and grouped index runs have distinct roles; materialize only surviving required columns.
//! Block-backed reads remain Plan P5; index Build copying is separate P4 work, not a positional promise.
//! Summary pruning is selected from evidence, not assumed useful on TPC-H load order (§12).
use crate::rows::{Columns, Rows};
use std::ops::Bound;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RowId(pub(crate) usize);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockId(usize);
impl RowId {
    pub fn offset(self) -> usize {
        self.0
    }
}
impl BlockId {
    pub fn offset(self) -> usize {
        self.0
    }
}
/// Each occurrence has one stable offset in 0..len, including equal-valued
/// duplicates. Positions and blocks enumerate that range in base order; block
/// partitioning uses block_size and remains stable for the immutable host.
pub trait PositionalRows: Rows {
    /// Normalize a caller's possibly nested shared reference by method lookup.
    /// This selects no provider: the receiver must already implement positions.
    fn positional_handle(self) -> Self {
        self
    }
    fn len(self) -> usize;
    fn row_at(self, id: RowId) -> Option<Self::Row>;
    fn positions(self) -> impl Iterator<Item = RowId> {
        (0..self.len()).map(RowId)
    }
    /// Borrow a contiguous range of base-relative positions without buffering.
    fn position_range(self, range: std::ops::Range<usize>) -> impl Iterator<Item = RowId> {
        (range.start.min(self.len())..range.end.min(self.len())).map(RowId)
    }
    fn is_empty(self) -> bool {
        self.len() == 0
    }
    /// Positive block size. Scalar hosts expose a single block.
    fn block_size(self) -> std::num::NonZeroUsize {
        std::num::NonZeroUsize::new(self.len().max(1)).unwrap()
    }
    fn blocks(self) -> impl Iterator<Item = BlockId> {
        (0..self.len().div_ceil(self.block_size().get())).map(BlockId)
    }
    fn block_positions(self, block: BlockId) -> impl Iterator<Item = RowId> {
        let size = self.block_size().get();
        let start = block.0.saturating_mul(size).min(self.len());
        (start..start.saturating_add(size).min(self.len())).map(RowId)
    }
}
macro_rules! positional {
    ([$($g:tt)*] $ty:ty) => {
        impl<'a, $($g)*> PositionalRows for &'a $ty where &'a T: Columns {
            fn len(self) -> usize { self.as_slice_ref().len() }
            fn row_at(self, id: RowId) -> Option<Self::Row> {
                self.as_slice_ref().get(id.0).map(Columns::columns)
            }
        }
    };
}
trait SliceRef<T> {
    fn as_slice_ref(&self) -> &[T];
}
impl<T> SliceRef<T> for [T] {
    fn as_slice_ref(&self) -> &[T] {
        self
    }
}
impl<T> SliceRef<T> for Vec<T> {
    fn as_slice_ref(&self) -> &[T] {
        self
    }
}
impl<T, const N: usize> SliceRef<T> for [T; N] {
    fn as_slice_ref(&self) -> &[T] {
        self
    }
}
positional!([T][T]);
positional!([T] Vec<T>);
positional!([T, const N: usize] [T; N]);

pub trait Column<H: PositionalRows>: Copy {
    type Key: Ord;
    /// Valid only for a live host-issued ID. May borrow the host for its lifetime.
    fn key(self, host: H, id: RowId) -> Self::Key;
}
pub trait ReadPredicate<K> {
    /// True proves every key passes and permits omitting all key extraction,
    /// bounds reads, and predicate evaluation. The fact must remain stable
    /// while this predicate is borrowed by a read.
    fn is_identity(&self) -> bool {
        false
    }
    fn test(&self, key: &K) -> bool;
    /// AND each exact row test into a same-length selection mask. Existing
    /// false entries stay false, so successive predicates compose by intersection.
    fn test_chunk(&self, keys: &[K], mask: &mut [bool]) {
        assert_eq!(keys.len(), mask.len(), "chunk keys and mask lengths differ");
        for (key, selected) in keys.iter().zip(mask) {
            *selected &= self.test(key);
        }
    }
    /// False proves no satisfying key in the closed interval; true is conservative.
    fn any_in(&self, _lo: &K, _hi: &K) -> bool {
        true
    }
}
impl<K, P: ReadPredicate<K> + ?Sized> ReadPredicate<K> for &P {
    #[inline]
    fn is_identity(&self) -> bool {
        P::is_identity(self)
    }
    #[inline]
    fn test(&self, key: &K) -> bool {
        P::test(self, key)
    }
    fn test_chunk(&self, keys: &[K], mask: &mut [bool]) {
        P::test_chunk(self, keys, mask);
    }
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        P::any_in(self, lo, hi)
    }
}

/// Test a live position without extracting its key when the predicate is identity.
#[inline]
pub fn test_position<H, C, P>(host: H, column: C, predicate: &P, id: RowId) -> bool
where
    H: PositionalRows,
    C: Column<H>,
    P: ReadPredicate<C::Key> + ?Sized,
{
    predicate.is_identity() || predicate.test(&column.key(host, id))
}
pub trait BlockRead<C>: PositionalRows
where
    C: Column<Self>,
{
    /// Bounds enclose every key in the block. None retains the block.
    fn bounds(self, _column: C, _block: BlockId) -> Option<(C::Key, C::Key)> {
        None
    }
    /// The block's keys in base occurrence order, borrowing where the marker
    /// permits it. Layout-specific hosts may override this without materializing.
    fn column_chunk(self, column: C, block: BlockId) -> impl ExactSizeIterator<Item = C::Key> {
        let size = self.block_size().get();
        let start = block.0.saturating_mul(size).min(self.len());
        let end = start.saturating_add(size).min(self.len());
        (start..end).map(move |offset| column.key(self, RowId(offset)))
    }
    /// Pulled row by row, this loop keeps the predicate test inline; an
    /// adapter chain (blocks, flatten, filter) measured twice as slow.
    fn read<P: ReadPredicate<C::Key>>(
        self,
        column: C,
        predicate: &P,
    ) -> impl Iterator<Item = RowId> {
        self.read_owned(column, predicate)
    }
    /// The same exact read, owning its predicate so a pipe can escape its
    /// construction scope without collecting rows or borrowing a local value.
    /// Host overrides of `bounds` apply to both read forms.
    fn read_owned<P: ReadPredicate<C::Key>>(
        self,
        column: C,
        predicate: P,
    ) -> impl Iterator<Item = RowId> {
        let identity = predicate.is_identity();
        BlockScan {
            host: self,
            column,
            predicate,
            size: if identity { 1 } else { self.block_size().get() },
            len: self.len(),
            block: 0,
            offset: 0,
            end: 0,
            identity,
        }
    }
    /// Evaluate each admitted block into a mask before yielding its survivors.
    /// Bounds and order match `read_owned`; only predicate lookahead differs.
    /// Key and mask buffers are reused across blocks.
    fn read_chunked<P: ReadPredicate<C::Key>>(
        self,
        column: C,
        predicate: P,
    ) -> impl Iterator<Item = RowId> {
        let mut identity_positions = predicate.is_identity().then(|| self.positions());
        let mut blocks = self.blocks().fuse();
        let mut positions = None;
        let mut keys = Vec::new();
        let mut mask = Vec::new();
        let mut offset = 0;
        std::iter::from_fn(move || {
            if let Some(positions) = &mut identity_positions {
                return positions.next();
            }
            loop {
                while let Some(id) = positions.as_mut().and_then(Iterator::next) {
                    let selected = mask[offset];
                    offset += 1;
                    if selected {
                        return Some(id);
                    }
                }
                let block = blocks.next()?;
                crate::count!(blocks_considered);
                if self
                    .bounds(column, block)
                    .is_some_and(|(lo, hi)| !predicate.any_in(&lo, &hi))
                {
                    crate::count!(blocks_skipped);
                    crate::count!(blocks_skipped_metadata);
                    continue;
                }
                crate::count!(blocks_visited);
                keys.clear();
                keys.extend(self.column_chunk(column, block).inspect(|_| {
                    crate::count!(rows_tested);
                }));
                mask.clear();
                mask.resize(keys.len(), true);
                predicate.test_chunk(&keys, &mut mask);
                positions = Some(self.block_positions(block));
                offset = 0;
            }
        })
    }
}
/// The exact positional read as a named iterator. Its loop state is plain
/// offsets, so `next` keeps the position in a register and writes it back once
/// per returned row, and `fold` runs the whole read as one loop. The previous
/// `from_fn` closure over `Option<Range>` state measured 1.3–2× a plain loop on
/// Q6: it stored the position every row and re-tested the predicate's `Bound`
/// discriminants every row on the mid-block re-entry path.
pub struct BlockScan<H, C, P> {
    host: H,
    column: C,
    predicate: P,
    size: usize,
    len: usize,
    /// Next block to consider; blocks partition `0..len` by `size`.
    block: usize,
    /// Remaining positions of the current block.
    offset: usize,
    end: usize,
    identity: bool,
}
impl<H: BlockRead<C>, C: Column<H>, P: ReadPredicate<C::Key>> BlockScan<H, C, P> {
    /// Advance to the next block the bounds cannot exclude. False when exhausted.
    #[inline]
    fn open(&mut self) -> bool {
        loop {
            let start = self.block.saturating_mul(self.size);
            if start >= self.len {
                return false;
            }
            let block = BlockId(self.block);
            self.block += 1;
            crate::count!(blocks_considered);
            if self
                .host
                .bounds(self.column, block)
                .is_some_and(|(lo, hi)| !self.predicate.any_in(&lo, &hi))
            {
                crate::count!(blocks_skipped);
                crate::count!(blocks_skipped_metadata);
                continue;
            }
            crate::count!(blocks_visited);
            self.offset = start;
            self.end = start.saturating_add(self.size).min(self.len);
            return true;
        }
    }
}
impl<H: BlockRead<C>, C: Column<H>, P: ReadPredicate<C::Key>> Iterator for BlockScan<H, C, P> {
    type Item = RowId;
    // Keep residual positional filters in the same loop. An out-of-line next
    // reloads the scan and bounds state for every survivor of the first filter.
    #[inline(always)]
    fn next(&mut self) -> Option<RowId> {
        if self.identity {
            if self.offset == self.len {
                return None;
            }
            let id = RowId(self.offset);
            self.offset += 1;
            return Some(id);
        }
        loop {
            let (mut offset, end) = (self.offset, self.end);
            while offset < end {
                let id = RowId(offset);
                offset += 1;
                crate::count!(rows_tested);
                if self.predicate.test(&self.column.key(self.host, id)) {
                    self.offset = offset;
                    return Some(id);
                }
            }
            self.offset = end;
            if !self.open() {
                return None;
            }
        }
    }
    #[inline]
    fn fold<B, F: FnMut(B, RowId) -> B>(mut self, mut accumulator: B, mut f: F) -> B {
        if self.identity {
            for offset in self.offset..self.len {
                accumulator = f(accumulator, RowId(offset));
            }
            return accumulator;
        }
        loop {
            for offset in self.offset..self.end {
                let id = RowId(offset);
                crate::count!(rows_tested);
                if self.predicate.test(&self.column.key(self.host, id)) {
                    accumulator = f(accumulator, id);
                }
            }
            self.offset = self.end;
            if !self.open() {
                return accumulator;
            }
        }
    }
}
impl<H: BlockRead<C>, C: Column<H>, P: ReadPredicate<C::Key>> std::iter::FusedIterator
    for BlockScan<H, C, P>
{
}
macro_rules! scalar_read {
    ([$($g:tt)*] $ty:ty) => {
        impl<'a, $($g)* C> BlockRead<C> for &'a $ty
        where &'a T: Columns, C: Column<Self> {}
    };
}
scalar_read!([T,][T]);
scalar_read!([T,] Vec<T>);
scalar_read!([T, const N: usize,] [T; N]);

pub trait ExactIntervalMembership<K: Ord>: crate::ExactMembership<K> {
    /// True iff an actual key lies in the closed interval. Inverted intervals are empty.
    fn any_in(&self, lo: &K, hi: &K) -> bool;
}
impl<K: Ord, S: ExactIntervalMembership<K> + ?Sized> ExactIntervalMembership<K> for &S {
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        S::any_in(*self, lo, hi)
    }
}
pub struct Membership<'a, S>(pub &'a S);
impl<K: Ord, S: ExactIntervalMembership<K>> ReadPredicate<K> for Membership<'_, S> {
    fn test(&self, key: &K) -> bool {
        self.0.contains(key)
    }
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        self.0.any_in(lo, hi)
    }
}
pub struct RowPredicate<F>(pub F);
impl<K, F: Fn(&K) -> bool> ReadPredicate<K> for RowPredicate<F> {
    fn test(&self, key: &K) -> bool {
        (self.0)(key)
    }
}
pub struct Point<K>(pub K);
impl<K: Ord> ReadPredicate<K> for Point<K> {
    fn test(&self, key: &K) -> bool {
        *key == self.0
    }
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        lo <= &self.0 && &self.0 <= hi
    }
}
pub struct Window<K> {
    pub low: Bound<K>,
    pub high: Bound<K>,
}
impl<K: Ord> ReadPredicate<K> for Window<K> {
    fn test(&self, key: &K) -> bool {
        (match &self.low {
            Bound::Unbounded => true,
            Bound::Included(x) => key >= x,
            Bound::Excluded(x) => key > x,
        }) && (match &self.high {
            Bound::Unbounded => true,
            Bound::Included(x) => key <= x,
            Bound::Excluded(x) => key < x,
        })
    }
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        if lo > hi {
            return false;
        }
        // Exact order-interval intersection, without assuming a discrete successor.
        let low = crate::intersect_lower(Bound::Included(lo), self.low.as_ref());
        let high = crate::intersect_upper(Bound::Included(hi), self.high.as_ref());
        match (low, high) {
            (Bound::Included(a), Bound::Included(b)) => a <= b,
            (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => {
                a < b
            }
            _ => true,
        }
    }
}

/// Scan a static predicate and a conservative join filter together. Both
/// independent block bounds must admit a block; static row tests precede key
/// filtering, so rejected rows incur no filter hashing or exact-set lookup.
/// Identity facts are cached once, but their mode is dispatched outside the
/// row loop so constant-false predicates do not acquire per-row flag checks.
pub fn read_with_filter<H, C, D, P, Q>(
    host: H,
    column: C,
    predicate: P,
    filter_column: D,
    filter: &Q,
) -> impl Iterator<Item = RowId>
where
    H: BlockRead<C> + BlockRead<D>,
    C: Column<H>,
    D: Column<H>,
    P: ReadPredicate<C::Key>,
    Q: ReadPredicate<D::Key>,
{
    let predicate_identity = predicate.is_identity();
    let filter_identity = filter.is_identity();
    let mut identity_positions = (predicate_identity && filter_identity).then(|| host.positions());
    let mut blocks = host.blocks().fuse();
    let mut positions = None;
    std::iter::from_fn(move || match (predicate_identity, filter_identity) {
        (true, true) => identity_positions
            .as_mut()
            .expect("identity positions")
            .next(),
        (false, false) => next_filtered_position::<H, C, D, P, Q, _, _, _, false, false>(
            host,
            column,
            &predicate,
            filter_column,
            filter,
            &mut blocks,
            &mut positions,
            |block| host.block_positions(block),
        ),
        (true, false) => next_filtered_position::<H, C, D, P, Q, _, _, _, true, false>(
            host,
            column,
            &predicate,
            filter_column,
            filter,
            &mut blocks,
            &mut positions,
            |block| host.block_positions(block),
        ),
        (false, true) => next_filtered_position::<H, C, D, P, Q, _, _, _, false, true>(
            host,
            column,
            &predicate,
            filter_column,
            filter,
            &mut blocks,
            &mut positions,
            |block| host.block_positions(block),
        ),
    })
}

#[inline]
fn next_filtered_position<H, C, D, P, Q, B, I, F, const P_IDENTITY: bool, const Q_IDENTITY: bool>(
    host: H,
    column: C,
    predicate: &P,
    filter_column: D,
    filter: &Q,
    blocks: &mut B,
    positions: &mut Option<I>,
    block_positions: F,
) -> Option<RowId>
where
    H: BlockRead<C> + BlockRead<D>,
    C: Column<H>,
    D: Column<H>,
    P: ReadPredicate<C::Key>,
    Q: ReadPredicate<D::Key>,
    B: Iterator<Item = BlockId>,
    I: Iterator<Item = RowId>,
    F: Fn(BlockId) -> I,
{
    loop {
        while let Some(id) = positions.as_mut().and_then(Iterator::next) {
            crate::count!(rows_tested);
            if (P_IDENTITY || predicate.test(&column.key(host, id)))
                && (Q_IDENTITY || filter.test(&filter_column.key(host, id)))
            {
                return Some(id);
            }
        }
        let block = blocks.next()?;
        crate::count!(blocks_considered);
        if (!P_IDENTITY
            && <H as BlockRead<C>>::bounds(host, column, block)
                .is_some_and(|(lo, hi)| !predicate.any_in(&lo, &hi)))
            || (!Q_IDENTITY
                && <H as BlockRead<D>>::bounds(host, filter_column, block)
                    .is_some_and(|(lo, hi)| !filter.any_in(&lo, &hi)))
        {
            crate::count!(blocks_skipped);
            crate::count!(blocks_skipped_metadata);
            continue;
        }
        crate::count!(blocks_visited);
        *positions = Some(block_positions(block));
    }
}
