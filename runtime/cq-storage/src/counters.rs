//! Opt-in deterministic query costs, isolated by thread. Reset after fixtures
//! and oracle construction and snapshot after exhausting a fresh query.
//!
//! Hash computations count hasher `finish` calls, including table growth, in
//! the fixed-hasher collections below and runtime internal hash collections.
//! A host's opaque hasher is observable by choosing these counted collections;
//! native reads of ordinary host collections still count lookups and rows.
//! Builds count actual source-consuming index initialization (not lazy handles).
//! Lookups count exact reads, candidate reads/counts and ordered cursor entry.
//! Cursor steps count attempted next/seek calls at storage boundaries, including
//! exhaustion; rows count successful scan/cursor items at those boundaries.
//! Distinct insertions count attempts, including duplicates. Use `DistinctSet`
//! in the caller's distinct provider to include compiler canonicalization.
//! Retained bytes count runtime-owned capacities (excluding allocator overhead
//! and heap allocations inside user keys/payloads). Reset starts a new ownership
//! epoch; older allocations and allocations moved to another thread are excluded.
//! Keyset staging peak is the largest distinct staging-set ownership estimate.
//! Fixture construction and independent BTreeSet oracles are not query costs.
use std::cell::Cell;
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub blocks_considered: u64,
    pub blocks_visited: u64,
    pub blocks_skipped: u64,
    pub blocks_skipped_metadata: u64,
    pub blocks_skipped_empty_selection: u64,
    pub rows_tested: u64,
    pub rows_copied: u64,
    pub bytes_retained_current: u64,
    pub bytes_retained_peak: u64,
    pub keyset_staging_bytes_peak: u64,
    pub hash_computations: u64,
    pub index_builds: u64,
    pub lookups: u64,
    pub cursor_steps: u64,
    pub rows_enumerated: u64,
    pub distinct_insertions: u64,
}
thread_local! {
    static EPOCH: Cell<u64> = const { Cell::new(0) };
    static COUNTS: Cell<Snapshot> = const { Cell::new(Snapshot {
        blocks_considered: 0,
        blocks_visited: 0,
        blocks_skipped: 0,
        blocks_skipped_metadata: 0,
        blocks_skipped_empty_selection: 0,
        rows_tested: 0,
        rows_copied: 0,
        bytes_retained_current: 0,
        bytes_retained_peak: 0,
        keyset_staging_bytes_peak: 0,
        hash_computations: 0, index_builds: 0, lookups: 0,
        cursor_steps: 0, rows_enumerated: 0, distinct_insertions: 0,
    }) };
}
pub(crate) fn epoch() -> u64 {
    EPOCH.with(Cell::get)
}
pub fn reset() {
    EPOCH.with(|epoch| epoch.set(epoch.get().wrapping_add(1)));
    COUNTS.with(|counts| counts.set(Snapshot::default()));
}
pub fn snapshot() -> Snapshot {
    COUNTS.with(Cell::get)
}
pub(crate) fn record(update: impl FnOnce(&mut Snapshot)) {
    COUNTS.with(|cell| {
        let mut counts = cell.get();
        update(&mut counts);
        cell.set(counts);
    });
}

/// Fixed keys eliminate RandomState's variation in collisions and table growth.
/// These are measurement collections, not a replacement for untrusted host maps.
#[derive(Default)]
pub struct CountingHasher(std::collections::hash_map::DefaultHasher);
impl Hasher for CountingHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }
    fn finish(&self) -> u64 {
        record(|counts| counts.hash_computations += 1);
        self.0.finish()
    }
}
pub type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<CountingHasher>>;
pub type HashSet<K> = std::collections::HashSet<K, BuildHasherDefault<CountingHasher>>;

pub struct DistinctSet<K>(HashSet<K>);
impl<K> Default for DistinctSet<K> {
    fn default() -> Self {
        Self(HashSet::default())
    }
}
impl<K: Eq + std::hash::Hash> DistinctSet<K> {
    pub fn new() -> Self {
        Self(HashSet::default())
    }
    pub fn insert(&mut self, row: K) -> bool {
        record(|counts| counts.distinct_insertions += 1);
        self.0.insert(row)
    }
}
