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
//! Fixture construction and independent BTreeSet oracles are not query costs.
use std::cell::Cell;
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub hash_computations: u64,
    pub index_builds: u64,
    pub lookups: u64,
    pub cursor_steps: u64,
    pub rows_enumerated: u64,
    pub distinct_insertions: u64,
}
thread_local! {
    static COUNTS: Cell<Snapshot> = const { Cell::new(Snapshot {
        hash_computations: 0, index_builds: 0, lookups: 0,
        cursor_steps: 0, rows_enumerated: 0, distinct_insertions: 0,
    }) };
}
pub fn reset() {
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
