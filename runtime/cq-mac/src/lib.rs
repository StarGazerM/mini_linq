//! Synchronous MCCL/MLX indexing over reusable immutable numeric relations.
//!
//! [`MacContext::relation_u32`] and [`MacContext::relation_i64`] ingest once.
//! [`MacContext::build`] projects a shared source on the GPU and constructs a
//! stable index with original row IDs or a selected payload column. Explicit CPU
//! comparison and byte-radix baselines consume that same source. There is no
//! automatic CPU fallback. GPU builds and merges complete before publication.
//!
//! Runs retain their originating context and remain readable after it is dropped.
//! MLX alone manages allocation caching and completion lifetimes; retained runs
//! and snapshots cannot be recycled. Merge requires the originating context, not
//! merely the same physical device. Unsupported platforms return explicit errors.

pub mod backend;
pub mod maintained;

pub use backend::{IndexSpec, MacContext, MacError, MacRun, Payload, Relation, RunCursor};
#[cfg(target_os = "macos")]
pub use backend::MacRadixRun;
pub use maintained::{MaintainedIndex, MaintenancePolicy, Snapshot, SnapshotCursor};

/// Process-wide MLX allocator controls; this provider owns no separate cache.
#[cfg(target_os = "macos")]
pub use mccl::memory;
