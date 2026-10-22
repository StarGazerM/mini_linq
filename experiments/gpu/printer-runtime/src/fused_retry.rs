//! Host-only fused retry policy. Candidate counts are bounds, never estimates.
use crate::{
    FusedRetryMetadata, OutputCount, Result, device_status_failure, failure, fused_slot_capacity,
};
use std::{cell::Cell, collections::BTreeMap, marker::PhantomData, mem::size_of, rc::Rc};

pub(crate) const MAX_ATTEMPTS: u32 = 12;
const MIN_CAPACITY: usize = 1024;
const MIN_HEADROOM: usize = 64 << 20;
thread_local! {
    static SCOPE: Cell<&'static str> = const { Cell::new("<unscoped>") };
}

/// Restores the previous hole label on drop. Guards must be nested on one
/// thread; the `!Send`/`!Sync` marker prevents moving a guard to another thread.
#[must_use = "retain the guard for the duration of the query hole"]
pub struct FusedRetryScope {
    previous: &'static str,
    _thread: PhantomData<Rc<()>>,
}

/// Label fused retry calls in the current thread until the returned guard drops.
/// Unlabelled calls use `"<unscoped>"`. Nested scopes restore their predecessor.
pub fn fused_retry_scope(label: &'static str) -> FusedRetryScope {
    FusedRetryScope {
        previous: SCOPE.with(|scope| scope.replace(label)),
        _thread: PhantomData,
    }
}
impl Drop for FusedRetryScope {
    fn drop(&mut self) {
        SCOPE.with(|scope| scope.set(self.previous));
    }
}

/// Cumulative reports per `(hole, site, tuple width)` within one Runtime.
/// `calls` includes terminal calls, even if allocation failed before a launch;
/// `attempts_per_call` records zero for those calls. Attempts count invocations
/// of the launch callback, and relaunches count invocations after the first.
///
/// With the `timings` feature, CUDA events bracket only the launch callback,
/// including its frontier reset and output compaction, not allocation, metadata
/// checks/readback, or later operators. Otherwise both timing fields are zero.
/// `kernel_ms` includes all timed attempts; `wasted_kernel_ms` is the portion
/// whose output was discarded. Failed CUDA operations remain terminal.
/// `peak_attempt_bytes` is the requested device storage for both output/arena
/// columns and pointer arrays, rounded slot table, count/status metadata,
/// dependency pointers and optional diagnostic counters. It excludes allocator
/// alignment/chunk slack, resident inputs/frontiers, and driver reservation.
#[derive(Clone, Debug, Default)]
pub struct FusedRetryReport {
    pub hole: &'static str,
    pub site: &'static str,
    pub calls: u64,
    pub attempts: u64,
    pub relaunches: u64,
    pub max_attempts: u32,
    pub attempts_per_call: Vec<u32>,
    pub wasted_kernel_ms: f64,
    pub kernel_ms: f64,
    pub peak_capacity: usize,
    pub peak_attempt_bytes: usize,
    pub last_distinct: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key {
    hole: &'static str,
    site: &'static str,
    width: usize,
}
impl Key {
    pub(crate) fn current(site: &'static str, width: usize) -> Self {
        Self {
            hole: SCOPE.with(Cell::get),
            site,
            width,
        }
    }
}

/// Bounded materialization estimates track candidates, not distinct admissions.
/// The checked successful count also lets callers bound downstream work exactly.
#[derive(Default)]
pub(crate) struct MaterializationHistory(BTreeMap<Key, u64>);
impl MaterializationHistory {
    pub(crate) fn capacity(&self, key: Key, hint: usize) -> usize {
        self.0
            .get(&key)
            .copied()
            .unwrap_or(0)
            .max(hint as u64)
            .max(MIN_CAPACITY as u64)
            .min(u32::MAX as u64) as usize
    }

    pub(crate) fn success(&mut self, key: Key, rows: u64) {
        if rows != 0 {
            self.0.insert(key, rows);
        }
    }
}

#[derive(Default)]
pub(crate) struct Records(BTreeMap<Key, Record>);
#[derive(Default)]
struct Record {
    report: FusedRetryReport,
    nonempty_distinct: Option<u64>,
}
impl Records {
    pub(crate) fn begin(&mut self, key: Key, hint: Option<usize>) -> (usize, usize) {
        let record = self.0.entry(key).or_insert_with(|| Record {
            report: FusedRetryReport {
                hole: key.hole,
                site: key.site,
                ..Default::default()
            },
            nonempty_distinct: None,
        });
        let capacity = initial_capacity(record.nonempty_distinct, hint);
        let call = record.report.attempts_per_call.len();
        record.report.calls += 1;
        record.report.attempts_per_call.push(0);
        (call, capacity)
    }

    pub(crate) fn launch(&mut self, key: Key, call: usize, capacity: usize, bytes: usize) {
        let report = &mut self.0.get_mut(&key).expect("registered fused retry").report;
        let attempts = &mut report.attempts_per_call[call];
        report.relaunches += u64::from(*attempts != 0);
        *attempts += 1;
        report.attempts += 1;
        report.max_attempts = report.max_attempts.max(*attempts);
        report.peak_capacity = report.peak_capacity.max(capacity);
        report.peak_attempt_bytes = report.peak_attempt_bytes.max(bytes);
    }

    pub(crate) fn timed(&mut self, key: Key, ms: f64, discarded: bool) {
        let report = &mut self.0.get_mut(&key).expect("registered fused retry").report;
        report.kernel_ms += ms;
        if discarded {
            report.wasted_kernel_ms += ms;
        }
    }

    pub(crate) fn success(&mut self, key: Key, distinct: u64) {
        let record = self.0.get_mut(&key).expect("registered fused retry");
        // Empty rounds provide no useful size prediction for the next nonempty
        // invocation; do not make every subsequent wave rediscover its capacity.
        if distinct != 0 {
            record.nonempty_distinct = Some(distinct);
        }
        record.report.last_distinct = distinct;
    }

    pub(crate) fn reports(&self) -> Vec<FusedRetryReport> {
        self.0
            .values()
            .map(|record| record.report.clone())
            .collect()
    }
}

pub(crate) fn initial_capacity(previous: Option<u64>, hint: Option<usize>) -> usize {
    previous
        .unwrap_or(0)
        .saturating_mul(2)
        .saturating_add(hint.unwrap_or(0) as u64)
        .max(MIN_CAPACITY as u64)
        .min(u32::MAX as u64) as usize
}

/// Validate every attempt before authorizing either publication or growth.
/// Only completed, successful distinct occupancy becomes the retained estimate.
pub(crate) fn next_capacity(
    metadata: FusedRetryMetadata,
    capacity: usize,
    expected_candidates: Option<u64>,
    attempts: u32,
) -> Result<Option<usize>> {
    if metadata.dependency_error != 0 {
        return Err(device_status_failure(metadata.dependency_error));
    }
    if metadata.key_error != 0 {
        return Err(device_status_failure(metadata.key_error));
    }
    let output_full = metadata.rows > capacity as u64;
    if metadata.error != 0 && !(metadata.error == 4 && output_full) {
        return Err(device_status_failure(metadata.error));
    }
    if metadata.rows > u32::MAX as u64 {
        return Err(device_status_failure(5));
    }
    if let Some(expected) = expected_candidates {
        if metadata.candidates != expected {
            return Err(failure(
                "CQ_GPU_RETRY_COUNT",
                format!(
                    "pure fused retry changed candidate count from {expected} to {}",
                    metadata.candidates,
                ),
            ));
        }
    }
    if metadata.distinct > capacity as u64 || metadata.distinct > metadata.candidates {
        return Err(failure(
            "CQ_GPU_FUSED_RETRY_METADATA",
            format!(
                "invalid distinct occupancy {} for capacity {capacity}, candidates {}",
                metadata.distinct, metadata.candidates,
            ),
        ));
    }
    if metadata.overflow == 0 && !output_full {
        if metadata.rows != metadata.distinct {
            return Err(failure(
                "CQ_GPU_FUSED_RETRY_METADATA",
                format!(
                    "successful fused output rows {} differ from arena distinct {}",
                    metadata.rows, metadata.distinct,
                ),
            ));
        }
        return Ok(None);
    }
    if attempts >= MAX_ATTEMPTS {
        return Err(failure(
            "CQ_GPU_FUSED_RETRY_ATTEMPTS",
            format!(
                "bounded fused retry exhausted {MAX_ATTEMPTS} attempts: capacity={capacity}, distinct={}, candidates={}",
                metadata.distinct, metadata.candidates,
            ),
        ));
    }
    let target = metadata
        .distinct
        .saturating_mul(4)
        .min(metadata.candidates)
        .min(u32::MAX as u64) as usize;
    if target <= capacity {
        return Err(failure(
            "CQ_GPU_FUSED_RETRY_RESOURCE",
            format!(
                "fused retry cannot grow: capacity={capacity}, distinct={}, candidates={}, geometric_target={target}, row_limit={}",
                metadata.distinct,
                metadata.candidates,
                u32::MAX,
            ),
        ));
    }
    Ok(Some(target))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AttemptBytes {
    pub(crate) requested: usize,
    pub(crate) pooled: usize,
}
impl AttemptBytes {
    fn add(&mut self, count: usize, bytes: usize) -> Option<()> {
        // Buffer uses Layout::array; do not propose allocations beyond isize.
        if bytes > isize::MAX as usize {
            return None;
        }
        let alignment = rccl_memory::DEVICE_ALLOCATION_ALIGNMENT;
        let rounded = bytes.checked_add(alignment - 1)? & !(alignment - 1);
        self.requested = self.requested.checked_add(bytes.checked_mul(count)?)?;
        self.pooled = self.pooled.checked_add(rounded.checked_mul(count)?)?;
        Some(())
    }
}

/// Every device allocation owned by one attempt, with per-allocation pool
/// alignment for budgeting and unaligned requested bytes for reporting.
pub(crate) fn attempt_bytes(
    width: usize,
    capacity: usize,
    dependencies: usize,
) -> Result<AttemptBytes> {
    let calculate = || -> Option<AttemptBytes> {
        let mut bytes = AttemptBytes::default();
        bytes.add(
            width.checked_mul(2)?,
            capacity.checked_mul(size_of::<i64>())?,
        )?;
        bytes.add(2, width.checked_mul(size_of::<u64>())?)?;
        bytes.add(2, size_of::<OutputCount>())?;
        bytes.add(
            1,
            fused_slot_capacity(capacity)
                .ok()?
                .checked_mul(size_of::<u64>())?,
        )?;
        bytes.add(1, size_of::<u64>())?;
        bytes.add(1, size_of::<u32>())?;
        bytes.add(1, size_of::<FusedRetryMetadata>())?;
        bytes.add(1, dependencies.checked_mul(size_of::<u64>())?)?;
        #[cfg(feature = "dedup-diagnostics")]
        bytes.add(1, 568 * 6 * size_of::<u64>())?;
        Some(bytes)
    };
    calculate().ok_or_else(|| failure("CQ_GPU_FUSED_RETRY_SIZE", format!(
        "fused attempt allocation size overflow: width={width}, capacity={capacity}, dependencies={dependencies}",
    )))
}

/// Bounded materialization owns output only, not a fused key arena or hash slots.
pub(crate) fn materialization_bytes(width: usize, capacity: usize) -> Result<AttemptBytes> {
    let calculate = || -> Option<AttemptBytes> {
        let mut bytes = AttemptBytes::default();
        bytes.add(width, capacity.checked_mul(size_of::<i64>())?)?;
        bytes.add(1, width.checked_mul(size_of::<u64>())?)?;
        bytes.add(1, size_of::<OutputCount>())?;
        Some(bytes)
    };
    calculate().ok_or_else(|| {
        failure(
            "CQ_GPU_RETRY_SIZE",
            format!("bounded output allocation size overflow: width={width}, capacity={capacity}"),
        )
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Budget {
    pub(crate) bytes: usize,
    pub(crate) materialization_bytes: usize,
    pub(crate) cached: usize,
    pub(crate) upstream: usize,
    pub(crate) headroom: usize,
}
impl Budget {
    /// CUDA free excludes custom-pool cached storage. Count that storage once,
    /// respect the pool maximum, reserve 5% of total VRAM (at least 64 MiB),
    /// and reserve half the remaining upstream space for fused geometric slack.
    /// Exact bounded materialization needs no such growth allowance.
    /// Aggregate stats cannot reveal fragmentation or private next_growth; an
    /// actual allocator failure is terminal, not permission to replay at OOM.
    pub(crate) fn new(
        cuda_free: usize,
        cuda_total: usize,
        pool_size: usize,
        allocated: usize,
        cached: usize,
        maximum: Option<usize>,
    ) -> Self {
        let headroom = MIN_HEADROOM.max(cuda_total / 20);
        let cached = cached.min(pool_size.saturating_sub(allocated));
        let upstream = (cuda_free.saturating_sub(headroom) / 2)
            .min(maximum.unwrap_or(usize::MAX).saturating_sub(pool_size));
        let materialization_upstream = cuda_free
            .saturating_sub(headroom)
            .min(maximum.unwrap_or(usize::MAX).saturating_sub(pool_size));
        Self {
            bytes: cached.saturating_add(upstream),
            materialization_bytes: cached.saturating_add(materialization_upstream),
            cached,
            upstream,
            headroom,
        }
    }

    /// Oversized hints count first with no output storage instead of risking OOM.
    pub(crate) fn materialization_capacity(self, width: usize, target: usize) -> Result<usize> {
        Ok(
            if materialization_bytes(width, target)?.pooled <= self.materialization_bytes {
                target
            } else {
                0
            },
        )
    }

    pub(crate) fn capacity(
        self,
        width: usize,
        dependencies: usize,
        target: usize,
        previous: Option<usize>,
    ) -> Result<usize> {
        let mut low = 0usize;
        let mut high = target.min(u32::MAX as usize);
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            if attempt_bytes(width, middle, dependencies)
                .is_ok_and(|bytes| bytes.pooled <= self.bytes)
            {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let minimum = previous.map_or(MIN_CAPACITY, |capacity| capacity.saturating_add(1));
        if low < minimum {
            return Err(failure(
                "CQ_GPU_FUSED_RETRY_RESOURCE",
                format!(
                    "fused retry budget cannot fit {minimum} rows: width={width}, target={target}, capacity_cap={low}, budget_bytes={}, cached_bytes={}, upstream_allowance={}, reserved_headroom={}",
                    self.bytes, self.cached, self.upstream, self.headroom,
                ),
            ));
        }
        Ok(low)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuntimeError;

    #[test]
    fn materialization_history_tracks_successful_candidates_per_site() {
        let mut history = MaterializationHistory::default();
        let key = Key::current("bounded", 3);
        assert_eq!(history.capacity(key, 1024), 1024);
        history.success(key, 4096);
        assert_eq!(history.capacity(key, 1024), 4096);
        assert_eq!(history.capacity(Key::current("other", 3), 1024), 1024);
        assert_eq!(history.capacity(Key::current("bounded", 4), 1024), 1024);
        {
            let _scope = fused_retry_scope("other-hole");
            assert_eq!(history.capacity(Key::current("bounded", 3), 1024), 1024);
        }
        history.success(key, 0);
        assert_eq!(history.capacity(key, 1024), 4096);
        history.success(key, u32::MAX as u64);
        assert_eq!(history.capacity(key, 1024), u32::MAX as usize);
    }

    #[test]
    fn bounded_history_never_doubles_large_materializations() {
        let mut history = MaterializationHistory::default();
        let key = Key::current("large", 3);
        let rows = 1_000_000_000;
        history.success(key, rows);
        assert_eq!(history.capacity(key, 1024), rows as usize);
    }

    #[test]
    fn oversized_bounded_estimates_use_a_count_only_attempt() {
        let bytes = materialization_bytes(3, 4096).unwrap().pooled;
        let budget = Budget {
            materialization_bytes: bytes,
            ..unlimited()
        };
        assert_eq!(budget.materialization_capacity(3, 4096).unwrap(), 4096);
        assert_eq!(budget.materialization_capacity(3, 8192).unwrap(), 0);
        assert_eq!(budget.materialization_capacity(3, 0).unwrap(), 0);
        let smaller = Budget {
            materialization_bytes: bytes - 1,
            ..budget
        };
        assert_eq!(smaller.materialization_capacity(3, 4096).unwrap(), 0);
    }

    #[test]
    fn bounded_budget_reserves_headroom_and_respects_pool_maximum() {
        let mib = 1usize << 20;
        let budget = Budget::new(
            1024 * mib,
            2048 * mib,
            512 * mib,
            256 * mib,
            256 * mib,
            Some(768 * mib),
        );
        assert_eq!(budget.materialization_bytes, 512 * mib);
        let uncapped = Budget::new(
            1024 * mib,
            2048 * mib,
            512 * mib,
            256 * mib,
            256 * mib,
            None,
        );
        assert_eq!(uncapped.materialization_bytes, 1280 * mib - 2048 * mib / 20);
    }

    fn code<T: std::fmt::Debug>(result: Result<T>) -> &'static str {
        result.unwrap_err().downcast::<RuntimeError>().unwrap().code
    }
    fn full(capacity: usize, candidates: u64) -> FusedRetryMetadata {
        FusedRetryMetadata {
            rows: capacity as u64,
            distinct: capacity as u64,
            candidates,
            overflow: 1,
            ..Default::default()
        }
    }
    fn unlimited() -> Budget {
        Budget {
            bytes: usize::MAX,
            materialization_bytes: usize::MAX,
            cached: 0,
            upstream: usize::MAX,
            headroom: 0,
        }
    }

    #[test]
    fn initial_estimate_uses_only_last_success_plus_hint() {
        for (previous, hint, expected) in [
            (None, None, 1024),
            (None, Some(17), 1024),
            (None, Some(4096), 4096),
            (Some(3000), None, 6000),
            (Some(3000), Some(256), 6256),
            (Some(0), None, 1024),
            (Some(u64::MAX), Some(usize::MAX), u32::MAX as usize),
        ] {
            assert_eq!(initial_capacity(previous, hint), expected);
        }
    }

    #[test]
    fn duplicate_heavy_candidates_only_cap_geometric_growth() {
        let mut capacity = 1024;
        for expected in [4096, 16_384, 65_536, 262_144, 1_048_576, 4_194_304] {
            capacity = next_capacity(full(capacity, 769_000_000), capacity, Some(769_000_000), 1)
                .unwrap()
                .unwrap();
            assert_eq!(capacity, expected);
        }
        let done = FusedRetryMetadata {
            rows: 3_090_000,
            distinct: 3_090_000,
            candidates: 769_000_000,
            ..Default::default()
        };
        assert_eq!(
            next_capacity(done, capacity, Some(769_000_000), 7).unwrap(),
            None
        );
        assert_eq!(
            next_capacity(full(1024, 3000), 1024, None, 1).unwrap(),
            Some(3000)
        );
        assert_eq!(
            next_capacity(full(1024, u64::MAX), 1024, None, 1).unwrap(),
            Some(4096)
        );
        let sparse = FusedRetryMetadata {
            rows: 1,
            distinct: 1024,
            ..full(4096, 769_000_000)
        };
        assert_eq!(
            code(next_capacity(sparse, 4096, None, 1)),
            "CQ_GPU_FUSED_RETRY_RESOURCE"
        );
    }

    #[test]
    fn twelve_attempts_span_u32_and_never_launch_thirteen() {
        let mut capacity = 1024;
        for attempt in 1..MAX_ATTEMPTS {
            let grown = next_capacity(full(capacity, u64::MAX), capacity, None, attempt)
                .unwrap()
                .unwrap();
            assert!(grown > capacity);
            capacity = grown;
        }
        assert_eq!(capacity, u32::MAX as usize);
        assert_eq!(
            code(next_capacity(
                full(capacity, u64::MAX),
                capacity,
                None,
                MAX_ATTEMPTS
            )),
            "CQ_GPU_FUSED_RETRY_ATTEMPTS"
        );
        let done = FusedRetryMetadata {
            overflow: 0,
            ..full(capacity, u64::MAX)
        };
        assert_eq!(
            next_capacity(done, capacity, None, MAX_ATTEMPTS).unwrap(),
            None
        );
        assert_eq!(
            code(next_capacity(full(capacity, u64::MAX), capacity, None, 1)),
            "CQ_GPU_FUSED_RETRY_RESOURCE"
        );
    }

    #[test]
    fn fatal_statuses_survive_exhaustion_on_every_attempt() {
        for (status, expected) in [
            (1, "CQ_GPU_OUTPUT_OVERFLOW"),
            (2, "CQ_GPU_EXPANSION_OVERFLOW"),
            (3, "CQ_GPU_SET_FULL"),
            (4, "CQ_GPU_OUTPUT_CAPACITY"),
            (5, "CQ_GPU_OUTPUT_ROWS"),
            (6, "CQ_GPU_MEMBERSHIP_DOMAIN"),
            (7, "CQ_GPU_RUN_BUILD"),
            (99, "CQ_GPU_KERNEL_ERROR"),
        ] {
            for attempt in [1, 2, MAX_ATTEMPTS] {
                for source in 0..3 {
                    let mut metadata = full(1024, 65_537);
                    match source {
                        0 => metadata.dependency_error = status,
                        1 => metadata.key_error = status,
                        _ => metadata.error = status,
                    }
                    assert_eq!(
                        code(next_capacity(metadata, 1024, Some(65_537), attempt)),
                        expected
                    );
                }
            }
        }
    }

    #[test]
    fn metadata_checks_precede_growth_and_publication() {
        for overflow in [0, 1] {
            for candidates in [0, 65_536, 65_538] {
                let metadata = FusedRetryMetadata {
                    overflow,
                    candidates,
                    ..full(1024, 65_537)
                };
                assert_eq!(
                    code(next_capacity(metadata, 1024, Some(65_537), 2)),
                    "CQ_GPU_RETRY_COUNT"
                );
            }
        }
        for metadata in [
            FusedRetryMetadata {
                distinct: 1025,
                ..full(1024, 65_537)
            },
            FusedRetryMetadata {
                candidates: 1,
                ..full(1024, 65_537)
            },
            FusedRetryMetadata {
                rows: 17,
                overflow: 0,
                ..full(1024, 65_537)
            },
        ] {
            assert_eq!(
                code(next_capacity(metadata, 1024, None, 1)),
                "CQ_GPU_FUSED_RETRY_METADATA"
            );
        }
        let oversized = FusedRetryMetadata {
            rows: u32::MAX as u64 + 1,
            ..full(1024, u64::MAX)
        };
        assert_eq!(
            code(next_capacity(oversized, 1024, None, 1)),
            "CQ_GPU_OUTPUT_ROWS"
        );
        for error in [0, 4] {
            let metadata = FusedRetryMetadata {
                rows: 1025,
                error,
                ..full(1024, 65_537)
            };
            assert_eq!(next_capacity(metadata, 1024, None, 2).unwrap(), Some(4096));
        }
        for (rows, candidates) in [(0, 0), (1, u64::MAX), (1024, 65_537)] {
            let metadata = FusedRetryMetadata {
                rows,
                distinct: rows,
                candidates,
                ..Default::default()
            };
            assert_eq!(next_capacity(metadata, 1024, None, 1).unwrap(), None);
        }
    }

    #[test]
    fn metadata_and_slot_layouts_match_budget() {
        assert_eq!(
            size_of::<FusedRetryMetadata>(),
            if cfg!(feature = "dedup-diagnostics") {
                88
            } else {
                40
            }
        );
        assert_eq!(std::mem::align_of::<FusedRetryMetadata>(), 8);
        for (capacity, slots) in [(0, 2), (1, 2), (3, 8), (1024, 2048), (1025, 4096)] {
            assert_eq!(fused_slot_capacity(capacity).unwrap(), slots);
        }
        assert_eq!(code(fused_slot_capacity(usize::MAX)), "CQ_GPU_SET_CAPACITY");
        let bytes = attempt_bytes(2, 1024, 3).unwrap();
        let expected = 2 * 2 * 1024 * 8
            + 2 * 2 * 8
            + 2 * size_of::<OutputCount>()
            + 2048 * 8
            + 8
            + 4
            + size_of::<FusedRetryMetadata>()
            + 3 * 8
            + if cfg!(feature = "dedup-diagnostics") {
                568 * 6 * 8
            } else {
                0
            };
        assert_eq!(bytes.requested, expected);
        assert!(bytes.pooled >= bytes.requested);
        assert_eq!(bytes.pooled % rccl_memory::DEVICE_ALLOCATION_ALIGNMENT, 0);
        assert!(attempt_bytes(0, 1024, 0).unwrap().requested > 0);
        for (width, capacity, dependencies) in
            [(usize::MAX, 1, 0), (1, usize::MAX, 0), (1, 1, usize::MAX)]
        {
            assert_eq!(
                code(attempt_bytes(width, capacity, dependencies)),
                "CQ_GPU_FUSED_RETRY_SIZE"
            );
        }
    }

    #[test]
    fn budget_counts_cached_storage_pool_limits_headroom_and_rounding() {
        let mib = 1 << 20;
        let budget = Budget::new(
            1024 * mib,
            2048 * mib,
            512 * mib,
            128 * mib,
            384 * mib,
            Some(768 * mib),
        );
        assert_eq!(budget.headroom, 2048 * mib / 20);
        assert_eq!(budget.cached, 384 * mib);
        assert_eq!(budget.upstream, 256 * mib);
        assert_eq!(budget.bytes, 640 * mib);
        let cached = Budget::new(
            0,
            1024 * mib,
            512 * mib,
            128 * mib,
            384 * mib,
            Some(512 * mib),
        );
        assert_eq!(cached.bytes, 384 * mib);
        let exhausted = Budget::new(0, 0, 0, 0, 0, None);
        assert_eq!(exhausted.bytes, 0);
        assert_eq!(
            code(exhausted.capacity(2, 0, 1024, None)),
            "CQ_GPU_FUSED_RETRY_RESOURCE"
        );
        let bytes = attempt_bytes(2, 1024, 0).unwrap().pooled;
        let exact = Budget {
            bytes,
            ..unlimited()
        };
        assert_eq!(exact.capacity(2, 0, 4096, None).unwrap(), 1024);
        assert_eq!(
            code(exact.capacity(2, 0, 4096, Some(1024))),
            "CQ_GPU_FUSED_RETRY_RESOURCE"
        );
        assert_eq!(
            code(
                Budget {
                    bytes: bytes - 1,
                    ..exact
                }
                .capacity(2, 0, 1024, None)
            ),
            "CQ_GPU_FUSED_RETRY_RESOURCE"
        );
        assert_eq!(
            unlimited().capacity(1, 0, usize::MAX, None).unwrap(),
            u32::MAX as usize
        );
        assert_eq!(
            Budget::new(usize::MAX, 0, usize::MAX, 0, usize::MAX, None).bytes,
            usize::MAX
        );
    }

    #[test]
    fn scope_restores_nested_and_unwound_labels() {
        let before = Key::current("site", 2);
        {
            let _outer = fused_retry_scope("outer");
            assert_eq!(Key::current("site", 2).hole, "outer");
            let _ = std::panic::catch_unwind(|| {
                let _inner = fused_retry_scope("inner");
                assert_eq!(Key::current("site", 2).hole, "inner");
                panic!("scope unwind");
            });
            assert_eq!(Key::current("site", 2).hole, "outer");
        }
        assert_eq!(Key::current("site", 2), before);
    }

    #[test]
    fn reports_and_estimates_are_stable_per_hole_site_and_width() {
        let mut records = Records::default();
        let key = Key {
            hole: "z",
            site: "kernel",
            width: 2,
        };
        let (call, capacity) = records.begin(key, None);
        assert_eq!(capacity, 1024);
        records.launch(key, call, 1024, 10_000);
        records.timed(key, 2.0, true);
        records.launch(key, call, 4096, 40_000);
        records.timed(key, 3.0, false);
        records.success(key, 3000);
        let (call, capacity) = records.begin(key, Some(17));
        assert_eq!(capacity, 6017);
        records.launch(key, call, capacity, 60_000);
        records.timed(key, 1.0, true); // A terminal failed call cannot replace 3000.
        assert_eq!(records.begin(key, None).1, 6000);
        for independent in [
            Key { hole: "a", ..key },
            Key {
                site: "other",
                ..key
            },
            Key { width: 3, ..key },
        ] {
            assert_eq!(records.begin(independent, None).1, 1024);
        }
        let reports = records.reports();
        assert_eq!(reports[0].hole, "a");
        assert_eq!(reports[1].site, "kernel");
        let report = &reports[1];
        assert_eq!(
            (
                report.calls,
                report.attempts,
                report.relaunches,
                report.max_attempts
            ),
            (3, 3, 1, 2)
        );
        assert_eq!(report.attempts_per_call, [2, 1, 0]);
        assert_eq!(report.kernel_ms, 6.0);
        assert_eq!(report.wasted_kernel_ms, 3.0);
        assert_eq!(report.peak_capacity, 6017);
        assert_eq!(report.peak_attempt_bytes, 60_000);
        assert_eq!(report.last_distinct, 3000);
        assert_eq!(reports[2].last_distinct, 0);
        assert_eq!(reports[3].site, "other");
        records.success(key, 0);
        assert_eq!(records.begin(key, None).1, 6000);
        assert_eq!(records.reports()[1].last_distinct, 0);
    }

    #[test]
    fn alternating_empty_calls_preserve_both_retry_estimates() {
        let mut fused = Records::default();
        let mut bounded = MaterializationHistory::default();
        let key = Key::current("alternating", 3);
        fused.begin(key, None);
        let mut previous = 0;
        for rows in [8192, 0, 0, 4096, 0, 0] {
            fused.success(key, rows);
            bounded.success(key, rows);
            if rows != 0 {
                previous = rows as usize;
            }
            assert_eq!(fused.begin(key, None).1, previous * 2);
            assert_eq!(bounded.capacity(key, 0), previous);
        }
    }

    #[test]
    fn fused_readback_counter_is_separate_and_resets_with_host_counters() {
        crate::reset_host_wait_count();
        crate::record_host_wait(true);
        crate::FUSED_RETRY_READBACKS.with(|count| count.set(1));
        assert_eq!(crate::fused_retry_readback_count(), 1);
        assert_eq!(crate::host_wait_count(), 1);
        assert_eq!(crate::control_readback_count(), 1);
        assert_eq!(crate::resume_readback_count(), 0);
        assert_eq!(crate::compaction_readback_count(), 0);
        crate::reset_host_wait_count();
        assert_eq!(crate::fused_retry_readback_count(), 0);
        assert_eq!(crate::host_wait_count(), 0);
        assert_eq!(crate::control_readback_count(), 0);
    }
}
