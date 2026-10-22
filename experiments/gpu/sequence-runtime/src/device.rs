//! Allocation-free device primitives shared by generated Sequence kernels.
//!
//! Cooperative scans use the same 256-thread contract as `gpu_tiles`; range
//! and membership searches are noncooperative and work on resident columns.

use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32};
use cuda_device::{device, thread};
use gpu_device_columns::ColumnView;
pub use gpu_device_columns::{decode, decode_u32};

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

/// Read the half-open row range for a dense key from a compressed directory.
///
/// # Safety
/// The column and its owners must be live, and both `key` and `key + 1` must
/// address initialized logical entries whose decoded values fit u32.
#[device]
#[inline(always)]
pub unsafe fn range(offsets: ColumnView, key: u32) -> (u32, u32) {
    unsafe {
        (
            decode_u32(offsets, key as u64),
            decode_u32(offsets, key as u64 + 1),
        )
    }
}

/// Exact membership in an ascending resident u32 column over `[begin, end)`.
/// Empty ranges return false without reading a value.
///
/// # Safety
/// The column and its owners must be live. `begin <= end` must delimit an
/// initialized logical range sorted by decoded u32 value.
#[device]
#[inline(always)]
pub unsafe fn contains_u32(column: ColumnView, begin: u32, end: u32, value: u32) -> bool {
    let (mut low, mut high) = (begin, end);
    while low < high {
        let middle = low + (high - low) / 2;
        if unsafe { decode_u32(column, middle as u64) } < value {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low < end && unsafe { decode_u32(column, low as u64) } == value
}
