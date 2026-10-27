//! Opt-in whole-program phase timing (`GPU_DATALOG_PHASES`).
//!
//! When enabled, each boundary synchronizes the pool stream so the elapsed
//! wall time attributes all queued device work to the phase that issued it,
//! then prints `phase\t<name>\t<seconds>` to stderr. When disabled, marks do
//! nothing: no synchronization is added to the evaluation.
use crate::{Pool, Result};
use std::cell::Cell;
use std::time::Instant;

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
    let now = Instant::now();
    LAST.with(|last| {
        if let Some(previous) = last.replace(Some(now)) {
            eprintln!("phase\t{name}\t{:.6}", (now - previous).as_secs_f64());
        }
    });
    Ok(())
}
