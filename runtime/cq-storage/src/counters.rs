//! Opt-in deterministic query costs. Reset after fixtures and oracle construction
//! and snapshot after exhausting a fresh query and joining all its tasks.
//!
//! Hash computations count hasher `finish` calls, including table growth, in
//! the fixed-hasher collections below and runtime internal hash collections.
//! A host's opaque hasher is observable by choosing these counted collections;
//! native reads of ordinary host collections still count lookups and rows.
//! Builds count actual source-consuming index initialization (not lazy handles).
//! Lookups count exact reads, candidate reads/counts and ordered cursor entry.
//! Cursor steps count attempted next/seek calls at storage boundaries, including
//! exhaustion; rows count successful scan/cursor items at those boundaries.
//! Distinct insertions count attempts, including duplicates. Native hash canon
//! providers are wrapped with `DistinctSet::from_set`, preserving their hasher;
//! custom providers can use `DistinctSet` directly to expose canonicalization.
//! Retained bytes count runtime-owned capacities (excluding allocator overhead
//! and heap allocations inside user keys/payloads). Reset starts a new ownership
//! epoch. Task contexts share that epoch: transferred allocations retain their
//! attribution, and worker event counts merge once per joined task. Retained
//! peak is the shared epoch's actual tracked-capacity high-water mark, not a
//! sum of worker peaks. Keyset staging peak is the largest individual staging
//! set estimate, not simultaneous staging memory.
//! Fixture construction and independent BTreeSet oracles are not query costs.
mod operators;
#[cfg(feature = "counters")]
pub(crate) use operators::build_cardinality;
pub(crate) use operators::{DeferredBuild, defer_build};
pub use operators::{
    OperatorDrained, OperatorInput, OperatorLabel, OperatorOutput, OperatorPredicate,
    OperatorProgram, OperatorRecord, OperatorScope, OperatorStream, operators, render_operators,
};

use std::cell::{Cell, RefCell};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub blocks_considered: u64,
    pub blocks_visited: u64,
    pub blocks_skipped: u64,
    pub blocks_skipped_metadata: u64,
    pub blocks_skipped_empty_selection: u64,
    pub rows_tested: u64,
    pub join_filter_tested: u64,
    pub join_filter_passed: u64,
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
    /// Subset of distinct_insertions in runtime candidate enumeration, rather
    /// than compiler-emitted dedup/keyset builds.
    pub candidate_insertions: u64,
}
impl Snapshot {
    fn merge(&mut self, other: Self) {
        self.blocks_considered += other.blocks_considered;
        self.blocks_visited += other.blocks_visited;
        self.blocks_skipped += other.blocks_skipped;
        self.blocks_skipped_metadata += other.blocks_skipped_metadata;
        self.blocks_skipped_empty_selection += other.blocks_skipped_empty_selection;
        self.rows_tested += other.rows_tested;
        self.join_filter_tested += other.join_filter_tested;
        self.join_filter_passed += other.join_filter_passed;
        self.rows_copied += other.rows_copied;
        self.keyset_staging_bytes_peak = self
            .keyset_staging_bytes_peak
            .max(other.keyset_staging_bytes_peak);
        self.hash_computations += other.hash_computations;
        self.index_builds += other.index_builds;
        self.lookups += other.lookups;
        self.cursor_steps += other.cursor_steps;
        self.rows_enumerated += other.rows_enumerated;
        self.distinct_insertions += other.distinct_insertions;
        self.candidate_insertions += other.candidate_insertions;
    }
}

#[derive(Default)]
struct SharedCounts {
    counts: Mutex<Snapshot>,
    memory: Mutex<(u64, u64)>,
    build: Mutex<BuildWall>,
    operators: Mutex<Vec<Arc<operators::State>>>,
}
#[derive(Default)]
struct BuildWall {
    visited: bool,
    active: usize,
    started: Option<Instant>,
    elapsed: Duration,
}

/// A query epoch carried into synchronous Rayon work. Event increments stay
/// thread-local; each task restores its caller's counters and merges once,
/// including when a worker executes nested work or unwinds.
#[derive(Clone, Default)]
pub struct TaskContext(Arc<SharedCounts>, operators::BuildContext);
impl TaskContext {
    pub fn capture() -> Self {
        TASK_CONTEXT.with(|context| Self(context.borrow().0.clone(), operators::capture_build()))
    }
    pub fn run<T>(&self, run: impl FnOnce() -> T) -> T {
        struct Restore {
            previous: TaskContext,
            build: operators::BuildContext,
            #[cfg(feature = "counters")]
            counts: Snapshot,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                #[cfg(feature = "counters")]
                {
                    let counts = COUNTS.with(|counts| counts.replace(self.counts));
                    let current = TaskContext::capture();
                    current
                        .0
                        .counts
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .merge(counts);
                }
                TASK_CONTEXT.with(|context| {
                    *context.borrow_mut() = self.previous.clone();
                });
                operators::replace_build(self.build.take());
            }
        }
        let restore = Restore {
            previous: TASK_CONTEXT.with(|context| context.replace(self.clone())),
            build: operators::replace_build(self.1.clone()),
            #[cfg(feature = "counters")]
            counts: COUNTS.with(|counts| counts.replace(Snapshot::default())),
        };
        let result = run();
        drop(restore);
        result
    }
    #[cfg(feature = "counters")]
    pub(crate) fn grow(&self, bytes: usize) {
        let mut memory = self
            .0
            .memory
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        memory.0 += bytes as u64;
        memory.1 = memory.1.max(memory.0);
    }
    #[cfg(feature = "counters")]
    pub(crate) fn shrink(&self, bytes: usize) {
        self.0
            .memory
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .0 -= bytes as u64;
    }
    pub(super) fn start_build(&self) {
        let mut build = self
            .0
            .build
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        build.visited = true;
        if build.active == 0 {
            build.started = Some(Instant::now());
        }
        build.active += 1;
    }
    pub(super) fn finish_build(&self) {
        let mut build = self
            .0
            .build
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        build.active -= 1;
        if build.active == 0 {
            let elapsed = build.started.take().expect("active build phase").elapsed();
            build.elapsed += elapsed;
        }
    }
    fn build_wall(&self) -> Option<Duration> {
        let build = self
            .0
            .build
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        build.visited.then(|| {
            build.elapsed
                + build
                    .started
                    .map_or(Duration::ZERO, |start| start.elapsed())
        })
    }
}
thread_local! {
    static TASK_CONTEXT: RefCell<TaskContext> = RefCell::new(TaskContext::default());
    static COUNTS: Cell<Snapshot> = const { Cell::new(Snapshot {
        blocks_considered: 0,
        blocks_visited: 0,
        blocks_skipped: 0,
        blocks_skipped_metadata: 0,
        blocks_skipped_empty_selection: 0,
        rows_tested: 0,
        join_filter_tested: 0,
        join_filter_passed: 0,
        rows_copied: 0,
        bytes_retained_current: 0,
        bytes_retained_peak: 0,
        keyset_staging_bytes_peak: 0,
        hash_computations: 0, index_builds: 0, lookups: 0,
        cursor_steps: 0, rows_enumerated: 0, distinct_insertions: 0, candidate_insertions: 0,
    }) };
}
pub fn reset() {
    TASK_CONTEXT.with(|context| *context.borrow_mut() = TaskContext::default());
    COUNTS.with(|counts| counts.set(Snapshot::default()));
}
pub fn snapshot() -> Snapshot {
    let mut counts = COUNTS.with(Cell::get);
    let context = TaskContext::capture();
    counts.merge(
        *context
            .0
            .counts
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
    );
    let memory = context
        .0
        .memory
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    counts.bytes_retained_current = memory.0;
    counts.bytes_retained_peak = memory.1;
    counts
}
/// Primitive costs are validated separately when profiling wall time.
pub const STORAGE_COUNTERS_ENABLED: bool = cfg!(feature = "counters");
#[inline]
pub(crate) fn record(update: impl FnOnce(&mut Snapshot)) {
    #[cfg(not(feature = "counters"))]
    let _ = update;
    #[cfg(feature = "counters")]
    COUNTS.with(|cell| {
        let mut counts = cell.get();
        update(&mut counts);
        cell.set(counts);
    });
}

/// Fixed keys eliminate RandomState's variation in collisions and table growth.
/// Timing-only mode uses the ordinary runtime's aHash algorithm; enabling
/// `counters` preserves the historical deterministic DefaultHasher measurements.
/// These are measurement collections, not a replacement for untrusted host maps.
#[cfg(feature = "counters")]
type CountingInner = std::collections::hash_map::DefaultHasher;
#[cfg(not(feature = "counters"))]
type CountingInner = ahash::AHasher;

pub struct CountingHasher(CountingInner);
impl Default for CountingHasher {
    fn default() -> Self {
        #[cfg(feature = "counters")]
        let inner = CountingInner::default();
        // `BuildHasherDefault` builds one hasher per hash, so the keys must be
        // fixed: a fresh `RandomState` per hasher hashes equal keys differently
        // and sets stop deduplicating.
        #[cfg(not(feature = "counters"))]
        let inner =
            std::hash::BuildHasher::build_hasher(&ahash::RandomState::with_seeds(0, 0, 0, 0));
        Self(inner)
    }
}
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

/// Count actual canonical set insertions without replacing the selected hasher.
pub struct DistinctSet<K, S = BuildHasherDefault<CountingHasher>>(
    std::collections::HashSet<K, S>,
    crate::counters_memory::Allocation,
);
impl<K, S: Default> Default for DistinctSet<K, S> {
    fn default() -> Self {
        Self(
            std::collections::HashSet::with_hasher(S::default()),
            crate::counters_memory::Allocation::new(0),
        )
    }
}
impl<K, S> DistinctSet<K, S> {
    /// Instrument a native set independently of operator input-row accounting.
    /// The set and its hash state are moved, not rebuilt or rehashed.
    pub fn from_set(set: std::collections::HashSet<K, S>) -> Self {
        let bytes = set.capacity() * (std::mem::size_of::<K>() + 1);
        Self(set, crate::counters_memory::Allocation::new(bytes))
    }
}
impl<K: Eq + std::hash::Hash> DistinctSet<K> {
    pub fn new() -> Self {
        Self(
            HashSet::default(),
            crate::counters_memory::Allocation::new(0),
        )
    }
}
impl<K: Eq + std::hash::Hash, S: std::hash::BuildHasher> DistinctSet<K, S> {
    pub fn insert(&mut self, row: K) -> bool {
        record(|counts| counts.distinct_insertions += 1);
        let before = self.0.capacity();
        let inserted = self.0.insert(row);
        self.1
            .grow((self.0.capacity() - before) * (std::mem::size_of::<K>() + 1));
        inserted
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn instrumented_sets_deduplicate_equal_keys() {
        let mut seen = super::DistinctSet::new();
        assert!(seen.insert((7_u64, String::from("x"))));
        assert!(!seen.insert((7_u64, String::from("x"))));
    }
}
