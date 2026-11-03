//! Per-host-thread metadata readback accounting for device execution phases.
use std::cell::Cell;

thread_local! {
    static READBACKS: Cell<u64> = const { Cell::new(0) };
}

/// Number of successful instrumented device metadata readbacks on this thread.
pub fn readback_count() -> u64 {
    READBACKS.with(Cell::get)
}

/// Record one successful device metadata readback, not a diagnostic row download.
pub fn record_readback() {
    READBACKS.with(|count| count.set(count.get().wrapping_add(1)));
}

/// Enable descriptor and dispatch diagnostics without downloading candidate rows.
pub fn trace_kernels() -> bool {
    static ENABLED: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("GPU_DATALOG_TRACE_KERNELS").is_some());
    *ENABLED
}

/// Declared radix work, not elapsed time or measured DRAM traffic.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SortWork {
    /// Nonempty logical sorts (including zero-bit singleton construction).
    pub sorts: u64,
    /// Eight-bit digit equivalents: ceil(active_bits / 8) per sort.
    /// Small inputs may execute these digits inside one block kernel.
    pub passes: u64,
    /// Sum of the caller-declared significant widths of nonempty logical sorts.
    pub radix_bits: u64,
    /// Input plus output bytes per digit: rows * passes * 2 * (8 + index_bytes).
    /// Excludes histograms, gathering, compaction and the initial input copy.
    pub bytes: u64,
}

/// Opt-in per-host-thread submission accounting. Reading/resetting these
/// counters never reads device memory, synchronizes a stream, or emits output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkCounters {
    /// Unknown-unique full-heading sorts (with compaction unless zero-bit).
    pub canonical: SortWork,
    /// Certified unique full-heading layout sorts (no count readback).
    pub layout: SortWork,
    /// Stable full-word occurrence permutations.
    pub occurrence: SortWork,
    pub merges: u64,
    /// Occurrence group-count downloads, including occurrence merges.
    /// Canonical admission counts remain included in `readback_count()`.
    pub grouping_readbacks: u64,
    /// Descriptor upload batches (snapshot tables or subtraction run tables).
    pub descriptor_materializations: u64,
    pub descriptor_bytes: u64,
    /// Materialized occurrence offset buffers; full-key set offsets are implicit.
    pub offset_materializations: u64,
    pub offset_bytes: u64,
}

thread_local! {
    static WORK_ENABLED: Cell<bool> = const { Cell::new(false) };
    static WORK: Cell<WorkCounters> = const { Cell::new(WorkCounters {
        canonical: SortWork { sorts: 0, passes: 0, radix_bits: 0, bytes: 0 },
        layout: SortWork { sorts: 0, passes: 0, radix_bits: 0, bytes: 0 },
        occurrence: SortWork { sorts: 0, passes: 0, radix_bits: 0, bytes: 0 },
        merges: 0, grouping_readbacks: 0,
        descriptor_materializations: 0, descriptor_bytes: 0,
        offset_materializations: 0, offset_bytes: 0,
    }) };
}

pub fn set_work_counters_enabled(enabled: bool) {
    WORK_ENABLED.with(|value| value.set(enabled));
}

pub fn work_counters() -> WorkCounters {
    WORK.with(Cell::get)
}

pub fn reset_work_counters() {
    WORK.with(|value| value.set(WorkCounters::default()));
}

pub(crate) fn record_work(record: impl FnOnce(&mut WorkCounters)) {
    if WORK_ENABLED.with(Cell::get) {
        WORK.with(|value| {
            let mut counters = value.get();
            record(&mut counters);
            value.set(counters);
        });
    }
}

pub(crate) enum SortKind {
    Canonical,
    Layout,
    Occurrence,
}

pub(crate) fn record_sort(kind: SortKind, rows: usize, bits: u32, indexed: bool) {
    if rows == 0 {
        return;
    }
    record_work(|counters| {
        let work = match kind {
            SortKind::Canonical => &mut counters.canonical,
            SortKind::Layout => &mut counters.layout,
            SortKind::Occurrence => &mut counters.occurrence,
        };
        let passes = u64::from(bits.div_ceil(8));
        work.sorts += 1;
        work.passes += passes;
        work.radix_bits += u64::from(bits);
        work.bytes += rows as u64 * passes * 2 * if indexed { 12 } else { 8 };
    });
}

/// Per-relation ownership footprints; shared layout owners are not additive.
#[derive(Clone, Copy, Default)]
pub struct MemoryUse {
    pub name: &'static str,
    pub rows: usize,
    pub accepted_live_bytes: usize,
    pub accepted_bytes: usize,
    pub layout_live_bytes: usize,
    pub layout_bytes: usize,
    pub arena_bytes: usize,
}

/// Opt-in diagnostic snapshot. Pool counters count actual allocations; relation
/// records identify owners and may overlap when layouts share canonical runs.
#[inline(never)]
pub fn trace_memory(pool: &crate::Pool, stage: &str, relations: &[MemoryUse]) {
    let stats = pool.memory_stats();
    eprintln!("gpu_datalog_memory\tstage={stage}\tcurrent_bytes={}\tpeak_bytes={}\treserved_bytes={}\tcached_bytes={}",
        stats.current_bytes, stats.peak_bytes, stats.reserved_bytes, stats.cached_bytes);
    let mut ordered: Vec<_> = relations.iter().collect();
    ordered.sort_unstable_by_key(|relation| {
        std::cmp::Reverse(relation.accepted_bytes + relation.layout_bytes + relation.arena_bytes)
    });
    for relation in ordered.into_iter().filter(|relation| {
        relation.accepted_bytes + relation.layout_bytes + relation.arena_bytes != 0
    }) {
        eprintln!("gpu_datalog_owner\tstage={stage}\trelation={}\trows={}\taccepted_live_bytes={}\taccepted_bytes={}\tlayout_live_bytes={}\tlayout_bytes={}\tarena_bytes={}",
            relation.name, relation.rows, relation.accepted_live_bytes, relation.accepted_bytes,
            relation.layout_live_bytes, relation.layout_bytes, relation.arena_bytes);
    }
}

/// Explicit opt-in evidence output. Counters are host submission plans:
/// `passes` are eight-bit digit equivalents, not nsys kernel launches, and
/// `bytes` are modeled radix traffic, not measured DRAM bytes.
pub fn trace_work(stage: &str) {
    let work = work_counters();
    for (kind, sort) in [
        ("canonical", work.canonical),
        ("layout", work.layout),
        ("occurrence", work.occurrence),
    ] {
        eprintln!("gpu_datalog_work\tstage={stage}\tkind={kind}\tsorts={}\tpasses={}\tradix_bits={}\tbytes={}",
            sort.sorts, sort.passes, sort.radix_bits, sort.bytes);
    }
    eprintln!("gpu_datalog_work\tstage={stage}\tkind=metadata\tmerges={}\tgrouping_readbacks={}\tdescriptor_materializations={}\tdescriptor_bytes={}\toffset_materializations={}\toffset_bytes={}",
        work.merges, work.grouping_readbacks, work.descriptor_materializations,
        work.descriptor_bytes, work.offset_materializations, work.offset_bytes);
}
