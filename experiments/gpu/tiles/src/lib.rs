//! Cross-crate device primitives for 256-thread, four-items-per-thread tiles.
//!
//! These are device-only entry points, not host-callable pointer utilities.
//! Cooperative calls (`scan`, `compact`, `sort`) require exactly `(256, 1, 1)`
//! converged threads, identical shared pointers, and identical scalar sizes.
//! Callers own pointer validity and shared-memory allocation. No helper allocates.

use cuda_device::{thread, warp};

pub const BLOCK_THREADS: u32 = 256;
pub const TILE_ITEMS: u32 = 1024;
pub const SCAN_SCRATCH_WORDS: u32 = 8;

/// Exclusive block sum in thread-index order, plus the total in every thread.
/// Addition wraps modulo 2^32, including the returned total.
///
/// `scratch` must point to **8 initialized-or-uninitialized u32 words (32 bytes)**
/// of writable, aligned shared memory, disjoint from other live data. All 256
/// threads must call together. The scratch can be reused immediately on return;
/// the final barrier ensures no reader remains. No 256/512-word work array is needed.
#[cuda_device::device]
#[inline(always)]
pub fn scan(value: u32, scratch: *mut u32) -> (u32, u32) {
    let tid = thread::threadIdx_x();
    let lane = tid & 31;
    let warp_index = tid >> 5;
    let mut inclusive = value;
    let mut distance = 1;
    while distance < 32 {
        let previous = warp::shuffle_up(inclusive, distance);
        if lane >= distance {
            inclusive = inclusive.wrapping_add(previous);
        }
        distance <<= 1;
    }
    if lane == 31 {
        unsafe { *scratch.add(warp_index as usize) = inclusive; }
    }
    thread::sync_threads();
    if warp_index == 0 {
        let mut sum = if lane < 8 {
            unsafe { *scratch.add(lane as usize) }
        } else {
            0
        };
        let mut distance = 1;
        while distance < 8 {
            let previous = warp::shuffle_up(sum, distance);
            if lane >= distance {
                sum = sum.wrapping_add(previous);
            }
            distance <<= 1;
        }
        if lane < 8 {
            unsafe { *scratch.add(lane as usize) = sum; }
        }
    }
    thread::sync_threads();
    let before_warp = if warp_index == 0 {
        0
    } else {
        unsafe { *scratch.add((warp_index - 1) as usize) }
    };
    let total = unsafe { *scratch.add(7) };
    let exclusive = before_warp.wrapping_add(inclusive.wrapping_sub(value));
    thread::sync_threads();
    (exclusive, total)
}

/// Stable compaction of a 1024-element mask into shared-memory original indices.
/// Thread `t` supplies mask entries for original indices `4*t .. 4*t+4`.
/// For a partial tile, pass false for every out-of-range entry.
///
/// `packed` has capacity 1024 u32 words; only `[0, returned_count)` is written.
/// `scratch` is a separate 8-word shared allocation, as required by `scan`.
/// The count is returned to every thread; packed indices are visible on return.
#[cuda_device::device]
#[inline(always)]
pub fn compact(mask: [bool; 4], packed: *mut u32, scratch: *mut u32) -> u32 {
    let count = mask[0] as u32 + mask[1] as u32 + mask[2] as u32 + mask[3] as u32;
    let (mut destination, total) = scan(count, scratch);
    let base = thread::threadIdx_x() * 4;
    let mut item = 0;
    while item < 4 {
        if mask[item] {
            unsafe { *packed.add(destination as usize) = base + item as u32; }
            destination += 1;
        }
        item += 1;
    }
    thread::sync_threads();
    total
}

/// Find the parent containing `position`, using upper_bound(offsets, position)-1.
/// `n` is the number of parents, NOT the number of offsets; `offsets` has n+1
/// nondecreasing entries. Empty parents (repeated offsets) are skipped correctly.
/// Returns `n` when no parent contains the position, including n=0, a position
/// before offsets[0], or a position at/after offsets[n]. Offsets need not start at 0.
/// This noncooperative helper needs no barriers and may be called by one thread.
#[cuda_device::device]
#[inline(always)]
pub fn owner(offsets: *const u32, n: u32, position: u32) -> u32 {
    if n == 0 || position < unsafe { *offsets } || position >= unsafe { *offsets.add(n as usize) } {
        return n;
    }
    let mut low = 0;
    let mut high = n;
    while low < high {
        let middle = low + (high - low) / 2;
        if unsafe { *offsets.add(middle as usize) } <= position {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low - 1
}

/// Sort `n <= 1024` shared-memory u32 keys ascending with a bitonic network.
/// All threads must supply the same n. Capacity must be at least the next power
/// of two of max(n, 1); slots from n to that capacity are padded with u32::MAX.
/// Real MAX keys are supported. This key-only sort makes no stability promise.
/// Input writes by the calling block are synchronized before the first compare;
/// all sorted keys (and padding) are visible when the function returns.
#[cuda_device::device]
#[inline(always)]
pub fn sort(keys: *mut u32, n: u32) {
    let tid = thread::threadIdx_x();
    let mut width = 1;
    while width < n {
        width <<= 1;
    }
    let mut index = tid;
    while index < width {
        if index >= n {
            unsafe { *keys.add(index as usize) = u32::MAX; }
        }
        index += BLOCK_THREADS;
    }
    thread::sync_threads();
    let mut sequence = 2;
    while sequence <= width {
        let mut distance = sequence >> 1;
        while distance != 0 {
            // Each comparator has one writer for both endpoints. Mapping the
            // comparator number around the distance bit avoids duplicate loads.
            let mut pair = tid;
            while pair < width / 2 {
                let left = (pair / distance) * (distance * 2) + pair % distance;
                let right = left + distance;
                let a = unsafe { *keys.add(left as usize) };
                let b = unsafe { *keys.add(right as usize) };
                if ((left & sequence) == 0 && a > b) || ((left & sequence) != 0 && a < b) {
                    unsafe {
                        *keys.add(left as usize) = b;
                        *keys.add(right as usize) = a;
                    }
                }
                pair += BLOCK_THREADS;
            }
            thread::sync_threads();
            distance >>= 1;
        }
        sequence <<= 1;
    }
}

/// Partition the first `diagonal` elements of the ascending merge of A and B.
/// Returns (elements_from_a, elements_from_b), with A before B on equal keys.
/// Inputs must be sorted, a_len+b_len must fit u32, and diagonal <= that sum.
/// An empty input is never dereferenced. This helper is noncooperative; it can
/// partition global or shared arrays independently in each participating thread.
#[cuda_device::device]
#[inline(always)]
pub fn merge_path(a: *const u32, a_len: u32, b: *const u32, b_len: u32, diagonal: u32) -> (u32, u32) {
    let mut low = if diagonal > b_len { diagonal - b_len } else { 0 };
    let mut high = if diagonal < a_len { diagonal } else { a_len };
    while low < high {
        let i = low + (high - low) / 2;
        let j = diagonal - i;
        if i < a_len && j > 0 && unsafe { *a.add(i as usize) <= *b.add((j - 1) as usize) } {
            low = i + 1;
        } else {
            high = i;
        }
    }
    (low, diagonal - low)
}
