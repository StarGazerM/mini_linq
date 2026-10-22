//! Shared resident columns and measurements for the GPU Sequence target.
//!
//! The default runtime exposes the pooled storage and codec APIs consumed by
//! Sequence. Structures execution, blocked runs, and Datalog publication are
//! available only through `legacy-datalog` for the isolated legacy workspace.

pub mod nvtx;

pub use gpu_device_columns::{
    decode_stripe, decode_tile_pair, decode_value, detect_integer_codec, encode_value, Bounds,
    Buffer, Codec, ColumnView, DecodeValue, Delta, DeviceColumns, Dictionary, DictionaryCodec,
    EncodeValue, EncodedColumn, For, OwnedRowCodec, Plain, Pool, Representation, RowCodec,
};

pub type Error = Box<dyn std::error::Error>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

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
}

pub fn last_run_timings() -> Option<Timings> {
    LAST_TIMINGS.with(std::cell::Cell::get)
}

pub fn take_last_timings() -> Option<Timings> {
    LAST_TIMINGS.with(std::cell::Cell::take)
}

/// Publish phase measurements from a checked external target interpreter.
pub fn publish_timings(timings: Timings) {
    LAST_TIMINGS.with(|last| last.set(Some(timings)));
}

#[cfg(feature = "legacy-datalog")]
include!("legacy.rs");
