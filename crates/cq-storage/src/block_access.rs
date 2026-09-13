//! Borrowed access to an immutable block directory and its payload.

use std::collections::HashSet;
use std::num::NonZeroUsize;

/// An immutable address space whose metadata can be traversed before payload.
///
/// Implementations must obey these laws throughout a shared borrow:
///
/// - Block IDs are unique within the directory, and row IDs are unique within
///   their block. Enumerating all descriptors and their rows visits every stored
///   row exactly once. Neither address nor value changes during the borrow.
/// - [`blocks`](Self::blocks) enumerates metadata without opening or enumerating
///   payload rows. Descriptors retain the owner whose directory produced them.
/// - Repeated descriptor traversal preserves the same address-to-row mapping
///   and borrowed row references. No independent coordinate lookup is required.
/// - Equal identities mean interchangeable immutable address spaces: the same
///   valid coordinates resolve the same borrowed payload, and descriptors can
///   be used with either owner. Distinct stores must not accidentally compare
///   equal merely because their row values or zero-sized payload addresses do.
///   An identity remains stable during the borrow. It need not survive a move
///   of an unborrowed store.
///
/// These are semantic requirements on implementations, not facts that Rust's
/// type checker proves. There is no relational operator or access-path choice
/// in this interface.
pub trait BlockRead: Sized {
    type Row;
    type BlockId: Copy;
    type RowId: Copy;
    type Identity: Copy + Eq;
    type Descriptor<'a>: BlockDescriptor<'a, Store = Self>
    where
        Self: 'a;

    fn identity(&self) -> Self::Identity;

    fn blocks(&self) -> impl Iterator<Item = Self::Descriptor<'_>>;
}

/// A borrowed metadata descriptor with an explicit payload-opening operation.
///
/// `store` and `id` must refer to the directory entry that produced this
/// descriptor and must not open payload. `rows` opens only this entry's payload
/// and yields each valid local row ID exactly once. The address-to-row association
/// remains stable throughout the shared borrow, including repeated traversals.
/// An empty block is allowed.
pub trait BlockDescriptor<'a>: Copy {
    type Store: BlockRead + 'a;

    fn store(&self) -> &'a Self::Store;

    fn id(&self) -> <Self::Store as BlockRead>::BlockId;

    fn rows(
        self,
    ) -> impl Iterator<
        Item = (
            <Self::Store as BlockRead>::RowId,
            &'a <Self::Store as BlockRead>::Row,
        ),
    >;
}

/// Integer-coordinate bounds read entirely from a descriptor's metadata.
///
/// Each returned closed interval encloses that coordinate of every row in the
/// descriptor's payload. Bounds may be wider than necessary, and continue to
/// enclose every subset of the same immutable payload. Reading them must not
/// open payload. This interface is optional: a block directory need not provide
/// range metadata, and a different summary family can use a different trait.
pub trait CoordinateBounds {
    fn bounds(&self) -> &[(i64, i64)];
}

/// Build immutable blocks from rows that have already been deduplicated.
///
/// The resulting address space must contain exactly the supplied set. Chunk
/// order and layout are implementation choices; `block_rows` is the requested
/// maximum number of rows in each nonempty block. Construction is independent
/// of subsequent access-path or selection choices.
pub trait BuildBlocks: BlockRead {
    fn pack_unique(rows: HashSet<Self::Row>, block_rows: NonZeroUsize) -> Self;
}
