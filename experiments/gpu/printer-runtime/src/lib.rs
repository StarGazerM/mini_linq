//! Shared resident columns and measurements for the GPU Sequence target.
//!
//! The runtime exposes the pooled storage and codec APIs consumed by Sequence.

pub mod nvtx;

pub use gpu_device_columns::{
    decode_stripe, decode_tile_pair, decode_value, detect_integer_codec, encode_value, Bounds,
    Buffer, Codec, ColumnView, DecodeValue, Delta, DeviceColumns, Dictionary, DictionaryCodec,
    EncodeValue, EncodedColumn, For, OwnedRowCodec, Plain, Pool, Representation, RowCodec,
};

pub type Error = Box<dyn std::error::Error>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Formal rule GPU-LAUNCH-OCCUPANCY:
/// Size a grid-stride launch from the concrete function's CUDA occupancy query,
/// including its register and static shared-memory requirements. `shared` is
/// dynamic shared memory only. Prepare a one-block launch to obtain the function,
/// then prepare the final launch with this grid. No kernel is launched by either
/// preparation. Empty-safe kernels receive one block for a zero-tile domain.
pub fn launch_grid_for_kernel(
    function: &cuda_core::CudaFunction,
    parts: usize,
    threads: u32,
    shared: u32,
) -> Result<u32> {
    let blocks = function.max_active_blocks_per_multiprocessor(threads, shared)?;
    let resident = u64::from(function.context().multiprocessor_count()?) * u64::from(blocks);
    if resident == 0 {
        return Err("launch shape has no resident CUDA blocks".into());
    }
    Ok((parts.max(1) as u64).min(resident).min(u64::from(u32::MAX)) as u32)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub upload_ms: f64,
    pub build_ms: f64,
    pub kernel_ms: f64,
    pub dedup_ms: f64,
    pub download_ms: f64,
    pub total_ms: f64,
    pub wall_ms: f64,
}

thread_local! {
    static LAST_TIMINGS: std::cell::Cell<Option<Timings>> = const { std::cell::Cell::new(None) };
    static ARENA_TIMING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn last_run_timings() -> Option<Timings> {
    LAST_TIMINGS.with(std::cell::Cell::get)
}

pub fn take_last_timings() -> Option<Timings> {
    LAST_TIMINGS.with(std::cell::Cell::take)
}

/// Enable synchronizing phase measurements for arena invocations on this thread.
/// Ordinary Sequence outputs retain their existing timing behavior.
pub fn enable_arena_timing(enabled: bool) {
    ARENA_TIMING.with(|timing| timing.set(enabled));
    LAST_TIMINGS.with(|last| last.set(None));
}

pub fn arena_timing_enabled() -> bool {
    ARENA_TIMING.with(std::cell::Cell::get)
}

/// Publish phase measurements from a checked external target interpreter.
pub fn publish_timings(timings: Timings) {
    LAST_TIMINGS.with(|last| last.set(Some(timings)));
}

