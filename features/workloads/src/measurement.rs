//! Compile-time separation of uninstrumented timing and counted execution.
#[cfg(feature = "counters")]
pub use cq_storage::counters::{HashMap, Snapshot, reset, snapshot};
#[cfg(not(feature = "counters"))]
pub use std::collections::HashMap;
#[cfg(not(feature = "counters"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct Snapshot {
    pub hash_computations: u64,
    pub index_builds: u64,
    pub lookups: u64,
    pub cursor_steps: u64,
    pub rows_enumerated: u64,
    pub distinct_insertions: u64,
}
#[cfg(not(feature = "counters"))]
pub fn reset() {}
#[cfg(not(feature = "counters"))]
pub fn snapshot() -> Snapshot {
    Snapshot::default()
}
pub const INSTRUMENTED: bool = cfg!(feature = "counters");
