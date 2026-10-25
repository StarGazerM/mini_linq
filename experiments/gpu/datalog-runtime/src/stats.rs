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
    ordered.sort_unstable_by_key(|relation| std::cmp::Reverse(
        relation.accepted_bytes + relation.layout_bytes + relation.arena_bytes));
    for relation in ordered.into_iter().filter(|relation|
        relation.accepted_bytes + relation.layout_bytes + relation.arena_bytes != 0)
    {
        eprintln!("gpu_datalog_owner\tstage={stage}\trelation={}\trows={}\taccepted_live_bytes={}\taccepted_bytes={}\tlayout_live_bytes={}\tlayout_bytes={}\tarena_bytes={}",
            relation.name, relation.rows, relation.accepted_live_bytes, relation.accepted_bytes,
            relation.layout_live_bytes, relation.layout_bytes, relation.arena_bytes);
    }
}
