use cuda_device::thread;
use gpu_device_columns::{decode_source, SourceView};

/// One warp owns this many consecutive directory entries.
pub(crate) const DIRECTORY_TILE: usize = 256;

#[cuda_device::device]
#[inline(always)]
unsafe fn key_lower_bound(keys: *const u32, mut lo: u32, mut hi: u32, key: u32) -> u32 {
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if unsafe { *keys.add(mid as usize) } < key {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

#[cuda_device::cuda_module]
pub mod build {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract};

    /// Exact numeric source bounds, including strided and packed publications.
    /// Every lane participates, including lanes outside a short final prefix.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn numeric_bounds(input: SourceView, rows: u32, partial: *mut i64) {
        let lane = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        let mut row = lane;
        let mut min = i64::MAX;
        let mut max = i64::MIN;
        while row < rows as usize {
            // DECODE V: bound reduction reconstructs every source value before index planning; P6 reuse recorded encoded-domain bounds.
            let value = unsafe { gpu_device_columns::decode_source(input, row as u64) };
            min = min.min(value);
            max = max.max(value);
            row += stride;
        }
        let mut distance = 16;
        while distance > 0 {
            min = min.min(cuda_device::warp::shuffle_xor_u64(min as u64, distance) as i64);
            max = max.max(cuda_device::warp::shuffle_xor_u64(max as u64, distance) as i64);
            distance /= 2;
        }
        if lane % 32 == 0 {
            unsafe {
                *partial.add((lane / 32) * 2) = min;
                *partial.add((lane / 32) * 2 + 1) = max;
            }
        }
    }

    /// Decode a key through an optional base-row permutation.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn keys_and_rows(
        key: SourceView,
        base: i64,
        source: *const u32,
        rows: u32,
        keys: *mut u32,
        order: *mut u32,
    ) {
        let mut i = thread::index_1d().get() as u32;
        let stride = thread::gridDim_x() * 256;
        while i < rows {
            unsafe {
                let row = if source.is_null() {
                    i
                } else {
                    *source.add(i as usize)
                };
                // DECODE V: index key extraction reconstructs values before sorting; P6 sort recorded codes or keep index runs plain.
                *keys.add(i as usize) = (decode_source(key, row as u64) - base) as u32;
                *order.add(i as usize) = row;
            }
            i = match i.checked_add(stride) {
                Some(next) => next,
                None => break,
            };
        }
    }

    /// Locate a tile's endpoint ranks together. Neighboring sorted rows then
    /// delimit disjoint directory intervals, including holes and the sentinel.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn dense_directory(keys: *const u32, rows: u32, entries: u32, offsets: *mut u32) {
        use cuda_device::warp;
        let lane = warp::lane_id() as usize;
        let mut start = thread::index_1d().get() / 32 * DIRECTORY_TILE;
        let stride = thread::gridDim_x() as usize * (256 / 32) * DIRECTORY_TILE;
        while start < entries as usize {
            let last = (start + DIRECTORY_TILE).min(entries as usize) - 1;
            let target = if lane == 0 { start } else { last };
            let rank = if target == 0 {
                0
            } else if target + 1 == entries as usize {
                rows
            } else if lane < 2 {
                unsafe { key_lower_bound(keys, 0, rows, target as u32) }
            } else {
                0
            };
            let lo = warp::shuffle_sync(u32::MAX, rank, 0);
            let hi = warp::shuffle_sync(u32::MAX, rank, 1);
            let span = hi - lo;
            // Do not walk a huge duplicate run for a tiny directory slice.
            // Search locally only if that takes fewer probes than a linear pass.
            let search_cost = (last - start + 1) * (u32::BITS - span.leading_zeros()) as usize;
            if span == 0 || span as usize > search_cost {
                let mut d = start + lane;
                while d <= last {
                    let rank = unsafe { key_lower_bound(keys, lo, hi, d as u32) };
                    unsafe { *offsets.add(d) = rank };
                    d += 32;
                }
            } else {
                let mut row = lo as usize + lane;
                while row <= hi as usize {
                    let mut d = if row == lo as usize {
                        start
                    } else {
                        unsafe { *keys.add(row - 1) as usize + 1 }
                    };
                    let end = if row == hi as usize { last } else { unsafe { *keys.add(row) as usize } };
                    while d <= end {
                        unsafe { *offsets.add(d) = row as u32 };
                        d += 1;
                    }
                    row += 32;
                }
            }
            start += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    // DECODE V: multi-key index build expands each column before lexicographic sorting; P6 retain encoded keys.
    pub fn sequence_decode_column(
        key: SourceView,
        base: i64,
        source: *const u32,
        rows: u32,
        out: *mut u32,
    ) {
        let mut i = thread::index_1d().get() as u32;
        let stride = thread::gridDim_x() * 256;
        while i < rows {
            unsafe {
                let row = if source.is_null() {
                    i
                } else {
                    *source.add(i as usize)
                };
                // DECODE V: permutation gather reconstructs keys for a later sort; P6 compare encoded fields.
                *out.add(i as usize) = (decode_source(key, row as u64) - base) as u32;
            }
            i = match i.checked_add(stride) {
                Some(next) => next,
                None => break,
            };
        }
    }

    /// Selection uses decoded integers, including negative signed payloads.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn range_flags(column: SourceView, rows: u32, start: i64, end: i64, flags: *mut u32) {
        let mut row = thread::index_1d().get() as u32;
        let stride = thread::gridDim_x() * 256;
        while row < rows {
            unsafe {
                // DECODE V: range selection decodes before relational comparison; P6 translate bounds into the encoded domain.
                let value = decode_source(column, row as u64);
                *flags.add(row as usize) = (start <= value && value < end) as u32;
            }
            row = match row.checked_add(stride) {
                Some(next) => next,
                None => break,
            };
        }
    }
}
