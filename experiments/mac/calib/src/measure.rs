//! Shared measurement contract for CPU, GPU and hybrid calibration.
//!
//! Every primitive is a `Work`: a fixed amount of bytes/items that can be run
//! repeatedly. Timing wraps `run` with an optional start barrier so a CPU work
//! and a GPU work can be started simultaneously for contention measurements.

use serde::Serialize;
use std::sync::Barrier;
use std::time::Instant;

#[derive(Clone, Debug, Serialize)]
pub struct Measurement {
    /// Primitive name, e.g. `seq_read`, `gather_line`, `rmw_atomic`, `dispatch_empty`.
    pub op: String,
    /// `cpu` or `gpu`.
    pub processor: String,
    /// CPU worker threads (1 for GPU).
    pub threads: usize,
    /// Bytes moved per run as the model counts them (reads + writes).
    pub bytes: u64,
    /// Items per run (rows, lines, probes, dispatches).
    pub items: u64,
    pub reps: usize,
    pub median_ms: f64,
    pub min_ms: f64,
    /// bytes / median seconds / 1e9.
    pub gbps: f64,
    /// median ns per item.
    pub ns_per_item: f64,
    /// Free-form parameters (footprint, stride, hit rate, concurrent partner...).
    pub params: serde_json::Value,
}

impl Measurement {
    pub fn new(
        op: &str,
        processor: &str,
        threads: usize,
        bytes: u64,
        items: u64,
        mut times_ms: Vec<f64>,
        params: serde_json::Value,
    ) -> Self {
        times_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median_ms = times_ms[times_ms.len() / 2];
        let min_ms = times_ms[0];
        Self {
            op: op.into(),
            processor: processor.into(),
            threads,
            bytes,
            items,
            reps: times_ms.len(),
            median_ms,
            min_ms,
            gbps: bytes as f64 / (median_ms * 1e-3) / 1e9,
            ns_per_item: median_ms * 1e6 / items.max(1) as f64,
            params,
        }
    }
}

/// A repeatable unit of work. `run` must do exactly `bytes()`/`items()` of
/// work per call and return a checksum so the compiler cannot elide it.
pub trait Work: Send + Sync {
    fn name(&self) -> String;
    fn processor(&self) -> &'static str;
    fn threads(&self) -> usize;
    fn bytes(&self) -> u64;
    fn items(&self) -> u64;
    fn params(&self) -> serde_json::Value;
    /// One timed run. For GPU works this encodes, commits and waits.
    fn run(&self) -> u64;
}

/// Time `work` for `warmup + reps` runs; if `start` is given, wait on it once
/// before the first timed run (hybrid pairing).
pub fn time_work(work: &dyn Work, warmup: usize, reps: usize, start: Option<&Barrier>) -> Measurement {
    for _ in 0..warmup {
        std::hint::black_box(work.run());
    }
    if let Some(b) = start {
        b.wait();
    }
    let mut times = Vec::with_capacity(reps);
    for _ in 0..reps {
        let t = Instant::now();
        std::hint::black_box(work.run());
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    Measurement::new(
        &work.name(),
        work.processor(),
        work.threads(),
        work.bytes(),
        work.items(),
        times,
        work.params(),
    )
}

/// Run `work` back-to-back for at least `seconds`, returning the completed-run
/// timings. Used to keep a hybrid partner busy for the whole paired window.
pub fn run_for(work: &dyn Work, seconds: f64, start: Option<&Barrier>) -> Measurement {
    std::hint::black_box(work.run());
    if let Some(b) = start {
        b.wait();
    }
    let window = Instant::now();
    let mut times = Vec::new();
    while window.elapsed().as_secs_f64() < seconds || times.is_empty() {
        let t = Instant::now();
        std::hint::black_box(work.run());
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    Measurement::new(
        &work.name(),
        work.processor(),
        work.threads(),
        work.bytes(),
        work.items(),
        times,
        work.params(),
    )
}
