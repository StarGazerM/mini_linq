//! Opt-in whole-program phase timing (`GPU_DATALOG_PHASES`).
//!
//! When enabled, each boundary synchronizes the pool stream so the elapsed
//! wall time attributes all queued device work to the phase that issued it,
//! then prints `phase\t<name>\t<seconds>` to stderr. When disabled, marks do
//! nothing: no synchronization is added to the evaluation.
use crate::{Pool, Result};
use std::cell::Cell;
use std::time::Instant;

// Scope and class are separate nested ranges so both names remain static.
// Nsight correlates CUDA API calls to this stack on the issuing CPU thread.
macro_rules! admission_scope {
    ($name:expr) => {
        #[cfg(feature = "admission-trace")]
        let _admission_op = cq_gpu_runtime::nvtx::range($name);
    };
    ($name:expr, $rows:expr) => {
        $crate::phases::admission_scope!($name);
        #[cfg(feature = "admission-trace")]
        let _admission_size = cq_gpu_runtime::nvtx::range(match $rows {
            0 => c"rows/0",
            1..=256 => c"rows/1-256",
            257..=32768 => c"rows/257-32K",
            32769..=131072 => c"rows/32K-128K",
            131073..=2_000_000 => c"rows/128K-2M",
            _ => c"rows/over-2M",
        });
    };
}
pub(crate) use admission_scope;

thread_local! {
    static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
}

fn enabled() -> bool {
    std::env::var_os("GPU_DATALOG_PHASES").is_some()
}

/// Start timing at the beginning of a program evaluation.
pub fn start() {
    if enabled() {
        LAST.with(|last| last.set(Some(Instant::now())));
    }
}

/// Close the phase that began at the previous mark.
pub fn mark(pool: &Pool, name: &str) -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    pool.stream().synchronize()?;
    // Match the existing synchronized boundaries in an optional Nsight capture.
    // Disabled NVTX builds emit neither a CUDA call nor a string allocation.
    match name {
        "setup" => drop(cq_gpu_runtime::nvtx::range(c"gpu_datalog/setup_end")),
        "fixpoint" => drop(cq_gpu_runtime::nvtx::range(c"gpu_datalog/fixpoint_end")),
        _ => {}
    }
    let now = Instant::now();
    LAST.with(|last| {
        if let Some(previous) = last.replace(Some(now)) {
            eprintln!("phase\t{name}\t{:.6}", (now - previous).as_secs_f64());
        }
    });
    if name == "fixpoint" {
        let memory = pool.memory_stats();
        eprintln!("fixpoint_pool\tcurrent_bytes={}\tpeak_bytes={}", memory.current_bytes, memory.peak_bytes);
    }
    Ok(())
}
