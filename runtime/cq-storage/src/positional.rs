//! Stable borrowed occurrences and exact predicates. IDs are base-relative;
//! callers must use only live IDs enumerated by that host.
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
    fn test(&self, key: &K) -> bool;
    /// False proves no satisfying key in the closed interval; true is conservative.
    fn any_in(&self, _lo: &K, _hi: &K) -> bool {
        true
    }
}
impl<K, P: ReadPredicate<K> + ?Sized> ReadPredicate<K> for &P {
    #[inline]
    fn test(&self, key: &K) -> bool {
        P::test(self, key)
    }
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        P::any_in(self, lo, hi)
    }
}
pub trait BlockRead<C>: PositionalRows
where
    C: Column<Self>,
{
    /// Bounds enclose every key in the block. None retains the block.
    fn bounds(self, _column: C, _block: BlockId) -> Option<(C::Key, C::Key)> {
        None
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
        let mut blocks = self.blocks().fuse();
        let mut positions = None;
        std::iter::from_fn(move || {
            loop {
                while let Some(id) = positions.as_mut().and_then(Iterator::next) {
                    crate::count!(rows_tested);
                    if predicate.test(&column.key(self, id)) {
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
                positions = Some(self.block_positions(block));
            }
        })
    }
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
    let mut blocks = host.blocks().fuse();
    let mut positions = None;
    std::iter::from_fn(move || {
        loop {
            while let Some(id) = positions.as_mut().and_then(Iterator::next) {
                crate::count!(rows_tested);
                if predicate.test(&column.key(host, id))
                    && filter.test(&filter_column.key(host, id))
                {
                    return Some(id);
                }
            }
            let block = blocks.next()?;
            crate::count!(blocks_considered);
            if <H as BlockRead<C>>::bounds(host, column, block)
                .is_some_and(|(lo, hi)| !predicate.any_in(&lo, &hi))
                || <H as BlockRead<D>>::bounds(host, filter_column, block)
                    .is_some_and(|(lo, hi)| !filter.any_in(&lo, &hi))
            {
                crate::count!(blocks_skipped);
                crate::count!(blocks_skipped_metadata);
                continue;
            }
            crate::count!(blocks_visited);
            positions = Some(host.block_positions(block));
        }
    })
}
