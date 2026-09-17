//! Physical access traits and concrete data structures for generated programs.
//!
//! These structures do not implement relational operators, choose access paths,
//! or couple a collection to a query-wide representation. A program can use a
//! [`BlockStore`] alongside ordinary vectors, maps, and independently built
//! auxiliary structures. Traversal and the association of masks with storage
//! belong to the program using them.
//!
//! Query access is available only through the operation traits. Concrete fields
//! and layout helpers are private; [`BlockStore::from_rows`] is an ingestion
//! convenience, while compiled construction uses [`BuildBlocks`].

mod block_access;
mod blocks;
mod mask;
mod membership;
mod metrics;
mod replay;
mod selection;
mod summary;

pub use block_access::{BlockDescriptor, BlockRead, BuildBlocks, CoordinateBounds};
pub use blocks::{Block, BlockStore};
pub use mask::BlockMask;
pub use membership::ExactMembership;
pub use metrics::{Counters, Work};
pub use replay::{BuildReplay, Replay, ReplayCursor, ReplayRead};
pub use selection::{BlockSelection, MakeSelection};
pub use summary::{RangeSummary, ZoneSummary};

type Range = (i64, i64);
