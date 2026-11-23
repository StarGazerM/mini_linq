//! Class A semi-join prefilter: one pass over a delta tests every rule
//! version's first-probe key set at once through a dense per-symbol mask
//! (`mask[code]` bit v = code is a key of version v's index) and appends the
//! surviving rows per version. Baseline: per-version binary search.
use cuda_device::device;

#[device]
pub unsafe fn contains_sorted(keys: *const u32, n: u32, key: u32) -> bool {
    let (mut low, mut high) = (0u32, n);
    while low < high {
        let middle = (low + high) / 2;
        if unsafe { *keys.add(middle as usize) } < key {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low < n && unsafe { *keys.add(low as usize) } == key
}

#[cuda_device::cuda_module]
pub mod filter_kernels {
    use super::*;
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32, DeviceAtomicU64};
    use cuda_device::{kernel, launch_bounds, launch_contract};

    #[device]
    fn filter_global() -> (u64, u64) {
        let index = cuda_device::thread::blockIdx_x() as u64 * 256 + cuda_device::thread::threadIdx_x() as u64;
        (index, cuda_device::thread::gridDim_x() as u64 * 256)
    }

    #[device]
    fn filter_hash(mut x: u64) -> u64 {
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^ (x >> 31)
    }

    /// Synthetic delta: `rows[i] = (heap << 32) | var`, var uniform in `[0, domain)`.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn synth_delta(rows: *mut u64, n: u64, domain: u32, seed: u64) {
        let (mut i, stride) = filter_global();
        while i < n {
            let h = filter_hash(i ^ seed);
            unsafe { *rows.add(i as usize) = ((h >> 40) << 32) | ((h as u32) % domain) as u64 };
            i += stride;
        }
    }

    /// `mask[keys[i]] |= 1 << version`.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn set_mask(keys: *const u32, n: u32, version: u32, mask: *mut u32) {
        let (mut i, stride) = filter_global();
        while i < n as u64 {
            let key = unsafe { *keys.add(i as usize) };
            unsafe { DeviceAtomicU32::from_ptr(mask.add(key as usize)) }.fetch_or(1 << version, AtomicOrdering::Relaxed);
            i += stride;
        }
    }

    /// Prefilter: one mask load per delta row; a row with bit v set is
    /// appended to version v's output (`outs + v * capacity`).
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn filter_mask(rows: *const u64, n: u64, mask: *const u32, outs: *mut u64, capacity: u64, counts: *mut u64) {
        let (mut i, stride) = filter_global();
        while i < n {
            let row = unsafe { *rows.add(i as usize) };
            let mut bits = unsafe { *mask.add((row & 0xffff_ffff) as usize) };
            while bits != 0 {
                let version = bits.trailing_zeros();
                bits &= bits - 1;
                let at = unsafe { DeviceAtomicU64::from_ptr(counts.add(version as usize)) }.fetch_add(1, AtomicOrdering::Relaxed);
                if at < capacity {
                    unsafe { *outs.add((version as u64 * capacity + at) as usize) = row };
                }
            }
            i += stride;
        }
    }

    /// As `filter_mask`, with one atomic per (warp, version) instead of per
    /// surviving row: lanes holding version v ballot, the leader reserves the
    /// whole group, each lane writes at its rank. The loop is warp-uniform.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn filter_mask_warp(rows: *const u64, n: u64, mask: *const u32, versions: u32, outs: *mut u64, capacity: u64, counts: *mut u64) {
        let (first, stride) = filter_global();
        let lane = cuda_device::warp::lane_id();
        let below = cuda_device::warp::lanemask_lt();
        let mut warp_base = first - lane as u64;
        while warp_base < n {
            let i = warp_base + lane as u64;
            let (row, bits) = if i < n {
                let row = unsafe { *rows.add(i as usize) };
                (row, unsafe { *mask.add((row & 0xffff_ffff) as usize) })
            } else {
                (0, 0)
            };
            let mut any = cuda_device::warp::ballot_sync(u32::MAX, bits != 0);
            let mut version = 0;
            while any != 0 && version < versions {
                let holders = cuda_device::warp::ballot_sync(u32::MAX, bits & (1 << version) != 0);
                if holders != 0 {
                    let leader = holders.trailing_zeros();
                    let mut at = 0u64;
                    if lane == leader {
                        at = unsafe { DeviceAtomicU64::from_ptr(counts.add(version as usize)) }
                            .fetch_add(holders.count_ones() as u64, AtomicOrdering::Relaxed);
                    }
                    let at = cuda_device::warp::shuffle_u64_sync(u32::MAX, at, leader);
                    if bits & (1 << version) != 0 {
                        let slot = at + (holders & below).count_ones() as u64;
                        if slot < capacity {
                            unsafe { *outs.add((version as u64 * capacity + slot) as usize) = row };
                        }
                    }
                }
                version += 1;
                any = cuda_device::warp::ballot_sync(u32::MAX, bits >> version.min(31) != 0 && version < 32);
            }
            warp_base += stride;
        }
    }

    /// Baseline: per row and version, a binary search in that version's sorted keys
    /// (`keys + v * stride`, `lengths[v]` keys each).
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn filter_search(rows: *const u64, n: u64, keys: *const u32, key_stride: u32, lengths: *const u32, versions: u32, counts: *mut u64) {
        let (mut i, stride) = filter_global();
        while i < n {
            let var = unsafe { *rows.add(i as usize) } as u32;
            let mut version = 0;
            while version < versions {
                let base = unsafe { keys.add((version * key_stride) as usize) };
                if unsafe { contains_sorted(base, *lengths.add(version as usize), var) } {
                    unsafe { DeviceAtomicU64::from_ptr(counts.add(version as usize)) }.fetch_add(1, AtomicOrdering::Relaxed);
                }
                version += 1;
            }
            i += stride;
        }
    }
}
