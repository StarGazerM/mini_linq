//! Machine caps: streaming, random gather, dependent search, atomics.
use cuda_device::device;

#[device]
pub fn mix(mut x: u64) -> u64 {
    // splitmix64 finalizer
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// First index in `[0, n)` with `data[index] >= key`.
#[device]
pub unsafe fn lower_bound(data: *const u64, n: u64, key: u64) -> u64 {
    let (mut low, mut high) = (0u64, n);
    while low < high {
        let middle = (low + high) / 2;
        if unsafe { *data.add(middle as usize) } < key {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

#[cuda_device::cuda_module]
pub mod caps_kernels {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract};

    #[device]
    fn global() -> (u64, u64) {
        let index = cuda_device::thread::blockIdx_x() as u64 * 256 + cuda_device::thread::threadIdx_x() as u64;
        (index, cuda_device::thread::gridDim_x() as u64 * 256)
    }

    /// `data[i] = i * step` (sorted, distinct for step > 0).
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn iota(data: *mut u64, n: u64, step: u64) {
        let (mut i, stride) = global();
        while i < n {
            unsafe { *data.add(i as usize) = i * step };
            i += stride;
        }
    }

    /// Coalesced grid-stride read of `n` words.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn stream_sum(data: *const u64, n: u64, out: *mut u64) {
        let (mut i, stride) = global();
        let mut acc = 0u64;
        while i < n {
            acc = acc.wrapping_add(unsafe { *data.add(i as usize) });
            i += stride;
        }
        if acc == u64::MAX {
            unsafe { *out = acc };
        }
    }

    /// `m` independent uniformly random 8-byte reads from `[0, n)`.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn gather_sum(data: *const u64, n: u64, m: u64, seed: u64, out: *mut u64) {
        let (mut q, stride) = global();
        let mut acc = 0u64;
        while q < m {
            let index = mix(q ^ seed) % n;
            acc = acc.wrapping_add(unsafe { *data.add(index as usize) });
            q += stride;
        }
        if acc == u64::MAX {
            unsafe { *out = acc };
        }
    }

    /// `m` binary searches into sorted `data[0, n)`; queries random or in
    /// ascending order across the grid (`sorted != 0`, neighbours share paths).
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn search_sum(data: *const u64, n: u64, m: u64, seed: u64, sorted: u32, out: *mut u64) {
        let (mut q, stride) = global();
        let top = unsafe { *data.add(n as usize - 1) };
        let mut acc = 0u64;
        while q < m {
            let key = if sorted != 0 { (q as u128 * top as u128 / m as u128) as u64 } else { mix(q ^ seed) % top };
            acc = acc.wrapping_add(unsafe { lower_bound(data, n, key) });
            q += stride;
        }
        if acc == u64::MAX {
            unsafe { *out = acc };
        }
    }

    /// Every thread adds `iters` times to one of `slots` counters.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn atomic_add(counters: *mut u64, slots: u64, iters: u32) {
        let (i, _) = global();
        let slot = unsafe { counters.add((i % slots) as usize) };
        let mut k = 0;
        while k < iters {
            unsafe { cuda_device::atomic::DeviceAtomicU64::from_ptr(slot) }
                .fetch_add(1, cuda_device::atomic::AtomicOrdering::Relaxed);
            k += 1;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn empty(out: *mut u64) {
        if cuda_device::thread::blockIdx_x() == u32::MAX {
            unsafe { *out = 0 };
        }
    }
}
