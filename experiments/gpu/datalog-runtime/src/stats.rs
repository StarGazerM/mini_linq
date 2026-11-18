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

fn device_memory(pool: &crate::Pool) -> (usize, usize) {
    let (mut free, mut total) = (0, 0);
    if pool.context().bind_to_thread().is_ok() {
        // SAFETY: both out-pointers are live locals and the context is current.
        let _ = unsafe { cuda_core::sys::cuMemGetInfo_v2(&mut free, &mut total) };
    }
    (free, total)
}

/// Step-level memory diagnostics under `GPU_DATALOG_TRACE_MEMORY`: one line
/// before each admission or publication operation allocates, with the pool
/// counters and the driver's free bytes. The last line before a failure names
/// the failing step. `rows * row_bytes` is the volume the step processes.
pub fn trace_step(pool: &crate::Pool, step: &'static str, rows: usize, row_bytes: usize) {
    static ENABLED: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("GPU_DATALOG_TRACE_MEMORY").is_some());
    if profiling() {
        profile_step(pool, step, rows);
    }
    if !*ENABLED {
        return;
    }
    let stats = pool.memory_stats();
    let (free, total) = device_memory(pool);
    eprintln!("gpu_datalog_step\tstep={step}\trows={rows}\tbytes={}\tcurrent_bytes={}\treserved_bytes={}\tcached_bytes={}\tdevice_free_bytes={free}\tdevice_total_bytes={total}",
        rows * row_bytes, stats.current_bytes, stats.reserved_bytes, stats.cached_bytes);
}

/// Whether `GPU_DATALOG_PROFILE` asks for the wall-clock breakdown.
pub fn profiling() -> bool {
    static ENABLED: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("GPU_DATALOG_PROFILE").is_some());
    *ENABLED
}

type ProfileKey = (usize, &'static str, String, &'static str);
struct Profile {
    last: std::time::Instant,
    key: ProfileKey,
    /// Seconds, entries and rows per (stratum, phase, relation, step).
    totals: std::collections::BTreeMap<ProfileKey, (f64, u64, u64)>,
}
thread_local! {
    static PROFILE: std::cell::RefCell<Option<Profile>> = const { std::cell::RefCell::new(None) };
}

fn profile_switch(pool: &crate::Pool, key: impl FnOnce(&ProfileKey) -> ProfileKey, rows: usize) {
    // The breakdown charges wall time, so each boundary waits for the stream.
    let _ = pool.stream().synchronize();
    PROFILE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let now = std::time::Instant::now();
        let profile = slot.get_or_insert_with(|| Profile {
            last: now,
            key: (0, "setup", String::new(), ""),
            totals: Default::default(),
        });
        let elapsed = (now - profile.last).as_secs_f64();
        profile.totals.entry(profile.key.clone()).or_default().0 += elapsed;
        profile.key = key(&profile.key);
        let entry = profile.totals.entry(profile.key.clone()).or_default();
        entry.1 += 1;
        entry.2 += rows as u64;
        profile.last = now;
    });
}

/// Opt-in profile (`GPU_DATALOG_PROFILE`): charge the wall time since the
/// previous boundary to it and enter `phase` of `relation` in `stratum`.
/// Boundaries synchronize the stream, so totals exceed an unprofiled run.
pub fn profile_enter(pool: &crate::Pool, stratum: usize, phase: &'static str, relation: &str) {
    profile_switch(pool, |_| (stratum, phase, relation.to_owned(), ""), 0);
}

fn profile_step(pool: &crate::Pool, step: &'static str, rows: usize) {
    profile_switch(pool, |key| (key.0, key.1, key.2.clone(), step), rows);
}

/// Print and clear the profile: one line per (stratum, phase, relation, step).
pub fn profile_report(pool: &crate::Pool) {
    profile_switch(pool, |key| key.clone(), 0);
    PROFILE.with(|slot| {
        if let Some(profile) = slot.borrow_mut().take() {
            for ((stratum, phase, relation, step), (seconds, count, rows)) in profile.totals {
                eprintln!("gpu_datalog_profile\tstratum={stratum}\tphase={phase}\trelation={relation}\tstep={step}\tseconds={seconds:.6}\tcount={count}\trows={rows}");
            }
        }
    });
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

/// Feature-gated per-host-thread submission accounting (`radix-counters`).
/// Reading/resetting never reads device memory, synchronizes, or emits output.
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

#[cfg(feature = "radix-counters")]
thread_local! {
    static WORK: Cell<WorkCounters> = const { Cell::new(WorkCounters {
        canonical: SortWork { sorts: 0, passes: 0, radix_bits: 0, bytes: 0 },
        layout: SortWork { sorts: 0, passes: 0, radix_bits: 0, bytes: 0 },
        occurrence: SortWork { sorts: 0, passes: 0, radix_bits: 0, bytes: 0 },
        merges: 0, grouping_readbacks: 0,
        descriptor_materializations: 0, descriptor_bytes: 0,
        offset_materializations: 0, offset_bytes: 0,
    }) };
}

#[cfg(feature = "radix-counters")]
pub fn work_counters() -> WorkCounters {
    WORK.with(Cell::get)
}

#[cfg(feature = "radix-counters")]
pub fn reset_work_counters() {
    WORK.with(|value| value.set(WorkCounters::default()));
}

#[cfg(feature = "radix-counters")]
pub(crate) fn record_work(record: impl FnOnce(&mut WorkCounters)) {
    WORK.with(|value| {
        let mut counters = value.get();
        record(&mut counters);
        value.set(counters);
    });
}

#[cfg(not(feature = "radix-counters"))]
#[inline(always)]
pub fn work_counters() -> WorkCounters {
    WorkCounters::default()
}

#[cfg(not(feature = "radix-counters"))]
#[inline(always)]
pub fn reset_work_counters() {}

#[cfg(not(feature = "radix-counters"))]
#[inline(always)]
pub(crate) fn record_work(_: impl FnOnce(&mut WorkCounters)) {}

pub(crate) enum SortKind {
    Canonical,
    Layout,
    Occurrence,
}

#[cfg(feature = "radix-counters")]
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

#[cfg(not(feature = "radix-counters"))]
#[inline(always)]
pub(crate) fn record_sort(_: SortKind, _: usize, _: u32, _: bool) {}

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
#[cfg(feature = "radix-counters")]
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

#[cfg(not(feature = "radix-counters"))]
#[inline(always)]
pub fn trace_work(_: &str) {}
