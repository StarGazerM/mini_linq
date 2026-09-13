//! Selection storage over an immutable coordinate domain.

use crate::BlockRead;

/// An exact set of coordinates, independent of payload and query operators.
///
/// `insert` adds precisely one coordinate and returns whether it was absent;
/// `contains` observes that set; `len` counts distinct coordinates; and
/// `block_nonempty` is true exactly when that block has a selected coordinate.
/// Coordinate operations may panic outside the domain established at creation.
/// The selection itself need not retain an owner: its caller is responsible for
/// associating it with the immutable address space used to construct it.
pub trait BlockSelection {
    type BlockId: Copy;
    type RowId: Copy;

    fn contains(&self, block: Self::BlockId, row: Self::RowId) -> bool;

    fn insert(&mut self, block: Self::BlockId, row: Self::RowId) -> bool;

    fn block_nonempty(&self, block: Self::BlockId) -> bool;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Construct a selection over exactly one store's coordinate domain.
///
/// `all` contains every valid address and `empty` contains none. Both retain the
/// same domain for subsequent operations, including addresses in empty blocks.
/// Construction may read metadata but must not open or enumerate payload rows.
/// Different selection representations may implement this trait for the same
/// store; the store does not choose the caller's selection strategy.
pub trait MakeSelection<S: BlockRead>:
    BlockSelection<BlockId = S::BlockId, RowId = S::RowId> + Sized
{
    fn all(store: &S) -> Self;

    fn empty(store: &S) -> Self;
}
