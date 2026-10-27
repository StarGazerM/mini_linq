//! Process-wide MLX allocator statistics and explicit cache/residency budgets.
//!
//! These settings affect every MLX user in the process, not just one [`crate::Gpu`].
//! Calls install MCCL's mlx-c error handler even before a GPU context is created.
//! No private pool or implicit wired-memory budget is introduced. Applications
//! should coordinate limit changes and restore prior limits when appropriate.

use crate::{clear_mlx_error, ffi, mlx_status, Result};

/// MLX allocator accounting in bytes, shared by all contexts in this process.
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    /// Bytes held by live MLX allocations.
    pub active_bytes: usize,
    /// Bytes in MLX's reusable allocation cache.
    pub cached_bytes: usize,
    /// Peak active allocation bytes recorded by MLX.
    pub peak_bytes: usize,
}

/// Reads process-wide allocator accounting without synchronizing GPU work.
///
/// The three counters are queried separately, not as an atomic snapshot. They
/// describe MLX allocation accounting, not physical residency or GPU traffic.
pub fn stats() -> Result<Stats> {
    clear_mlx_error();
    let mut result = Stats {
        active_bytes: 0,
        cached_bytes: 0,
        peak_bytes: 0,
    };
    mlx_status(
        unsafe { ffi::mlx_get_active_memory(&mut result.active_bytes) },
        "get active memory",
    )?;
    mlx_status(
        unsafe { ffi::mlx_get_cache_memory(&mut result.cached_bytes) },
        "get cached memory",
    )?;
    mlx_status(
        unsafe { ffi::mlx_get_peak_memory(&mut result.peak_bytes) },
        "get peak memory",
    )?;
    Ok(result)
}

/// Releases unused buffers from MLX's process-wide allocation cache.
///
/// Live arrays, lazy graph dependencies, and in-flight GPU resources retain their
/// MLX-owned storage. This is not a GPU synchronization or a residency reset.
pub fn clear_cache() -> Result<()> {
    clear_mlx_error();
    mlx_status(unsafe { ffi::mlx_clear_cache() }, "clear allocation cache")
}

/// Sets the process-wide reusable-buffer cache limit in bytes, returning its
/// previous value. Zero disables caching; live arrays are not invalidated.
///
/// This forwards the explicit budget to MLX without changing the wired limit.
/// Changing the limit does not itself empty the existing cache; use
/// [`clear_cache`] to release currently unused cached allocations.
pub fn set_cache_limit(bytes: usize) -> Result<usize> {
    clear_mlx_error();
    let mut previous = 0;
    mlx_status(
        unsafe { ffi::mlx_set_cache_limit(&mut previous, bytes) },
        "set cache limit",
    )?;
    Ok(previous)
}

/// Sets MLX's process-wide wired-memory budget in bytes, returning its previous
/// value. Zero disables this budget; MCCL never enables it implicitly.
///
/// Wiring is an explicit residency policy, not an allocation pool or a guarantee
/// that every live array is resident. Invalid budgets are returned as errors.
/// Effective residency support depends on MLX's backend/platform; success alone
/// does not prove that allocations are wired. Live arrays stay valid.
pub fn set_wired_limit(bytes: usize) -> Result<usize> {
    clear_mlx_error();
    let mut previous = 0;
    mlx_status(
        unsafe { ffi::mlx_set_wired_limit(&mut previous, bytes) },
        "set wired limit",
    )?;
    Ok(previous)
}
