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
//!
//! # Abstraction functions
//!
//! Every type here is a representation of a relation and is admissible only
//! because an abstraction function `α` relates it to that relation, and each
//! operation commutes with `α`. Storage never adds meaning: `α` of a built
//! value is exactly the supplied rows. The laws, checked in
//! `tests/representation_laws.rs`:
//!
//! - `α(BlockStore) = { row(c) | c ∈ coordinates }`, a set; `pack_unique` and
//!   `from_rows` satisfy `α(build(R)) = set(R)`, blocks partition the
//!   coordinates, and every block's bounds enclose its rows.
//! - `α(selection over S) = { row(c) | c ∈ selection } ⊆ α(S)`; `all` has
//!   `α = α(S)`, `empty` has `α = ∅`, `insert(c)` adds exactly `row(c)`.
//! - `α(index) = bag of (key, item)`; `build(I)` has `α = bag(I)` and
//!   `lookup(k)` enumerates exactly the items paired with `k`, with
//!   multiplicity and in source order.
//! - `α(membership) = key set`; `contains(k) ⇔ k ∈ α`.
//! - `α(replay) = the source sequence`; every cursor enumerates it.
//! - A range summary is internal to a representation's `next`: `may_match`
//!   is a superset test whose exact residual the representation owns.

mod algorithms;
mod block_access;
mod blocks;
mod hash_index;
mod host;
mod mask;
mod membership;
mod metrics;
mod ordered;
mod replay;
mod selection;
mod summary;

pub use algorithms::{pruned_rows, selected_rows};
pub use block_access::{BlockDescriptor, BlockRead, BuildBlocks, CoordinateBounds};
pub use blocks::{Block, BlockStore};
pub use hash_index::{BucketCursor, BuildIndex, HashIndex, IndexRead};
pub use mask::BlockMask;
pub use membership::ExactMembership;
pub use metrics::{Counters, Work};
pub use ordered::{GroupedCursor, KeyCursor, Merge, OrderedCursor, OrderedRead, merge};
pub use replay::{BuildReplay, Replay, ReplayCursor, ReplayRead};
pub use selection::{BlockSelection, MakeSelection};
pub use summary::{RangeSummary, ZoneSummary};

type Range = (i64, i64);
