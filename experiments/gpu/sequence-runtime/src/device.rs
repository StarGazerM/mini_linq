//! Allocation-free device primitives shared by generated Sequence kernels.
//!
//! Cooperative scans use the same 256-thread contract as `gpu_tiles`; range
//! and membership searches are noncooperative and work on resident columns.

use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32};
use cuda_device::{device, thread};
use gpu_device_columns::ColumnView;
pub use gpu_device_columns::{decode, decode_u32, original_position};

/// A null permutation denotes identity, including projections snapshotted
/// directly into occurrence order.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RangePayloadView {
    pub column: ColumnView,
    pub order: *const u32,
}
unsafe impl cuda_core::DeviceCopy for RangePayloadView {}

/// # Safety
/// `position` is a live range occurrence and both owners are live on this stream.
#[device]
#[inline(always)]
pub unsafe fn range_payload(view: RangePayloadView, position: u64) -> i64 {
    let row = if view.order.is_null() {
        position
    } else {
        unsafe { *view.order.add(position as usize) as u64 }
    };
    // DECODE V: a range probe reconstructs only the demanded occurrence; P6 keeps codes for relational consumers.
    unsafe { decode(view.column, row) }
}

pub use gpu_tiles::{BLOCK_THREADS, SCAN_SCRATCH_WORDS, TILE_ITEMS};

/// Set a sticky overflow word without racing other threads.
///
/// # Safety
/// `overflow` must address an aligned, writable device u32 initialized to zero
/// before the operation. It must not be reset while any producer is running.
#[device]
#[inline(always)]
pub unsafe fn flag_overflow(overflow: *mut u32) {
    unsafe { DeviceAtomicU32::from_ptr(overflow).store(1, AtomicOrdering::Relaxed) };
}

/// Exclusive u32 block scan and aggregate, flagging any mathematical overflow.
///
/// Results wrap modulo 2^32 when the flag is set and must not then be used as
/// allocation sizes or unchecked write offsets. A prefix crossing a multiple
/// of 2^32 has an inclusive sum smaller than its exclusive sum, even if the
/// complete block total crosses that boundary more than once.
///
/// # Safety
/// Exactly `(256, 1, 1)` converged threads must call with the same pointers.
/// `scratch` addresses eight aligned shared-memory u32 words, disjoint from
/// other live data. `overflow` meets `flag_overflow`'s contract. Scratch is
/// reusable and all overflow stores have been issued when the call returns.
#[device]
#[inline(always)]
pub unsafe fn checked_scan(value: u32, scratch: *mut u32, overflow: *mut u32) -> (u32, u32) {
    let (exclusive, aggregate) = gpu_tiles::scan(value, scratch);
    if exclusive.wrapping_add(value) < exclusive {
        unsafe { flag_overflow(overflow) };
    }
    thread::sync_threads();
    (exclusive, aggregate)
}

pub use gpu_tiles::owner;

/// Read adjacent native endpoints of the half-open range for a dense key.
///
/// # Safety
/// `offsets[key..=key+1]` must be live, initialized and fit u32.
#[device]
#[inline(always)]
pub unsafe fn range(offsets: *const i64, key: u32) -> (u32, u32) {
    unsafe { (*offsets.add(key as usize) as u32, *offsets.add(key as usize + 1) as u32) }
}
