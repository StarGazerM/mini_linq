//! Round-local exact u64 hash terminal. A failed bounded probe invalidates the
//! round; partial output must never be admitted. Benchmarks use a second mode
//! that elects one inserting lane per distinct key within the current warp.
use cuda_device::{atomic::{AtomicOrdering::Relaxed, DeviceAtomicU32, DeviceAtomicU64}, device, thread, warp};

pub const HD_BITS: u32 = 44;
pub const HD_MULT: u64 = 0xa0761d6478bd642f;
#[device]
fn hd_mix(x: u64) -> u64 {
    let x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    let x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

#[device]
unsafe fn hd_claim(table: *mut u64, mask: u32, key: u64, counters: *mut u32) -> bool {
    // The zero key has its own slot, so every possible 64-bit key is legal.
    if key == 0 {
        return unsafe { DeviceAtomicU32::from_ptr(counters.add(2)) }
            .compare_exchange(0, 1, Relaxed, Relaxed).is_ok();
    }
    let mut i = hd_mix(key) as u32 & mask;
    let mut attempts = 0;
    while attempts < 128 {
        let slot = unsafe { DeviceAtomicU64::from_ptr(table.add(i as usize)) };
        let existing = slot.load(Relaxed);
        if existing == key { return false; }
        if existing == 0 {
            match slot.compare_exchange(0, key, Relaxed, Relaxed) {
                Ok(_) => return true,
                Err(k) if k == key => return false,
                _ => {}
            }
        }
        i = (i + 1) & mask;
        attempts += 1;
    }
    unsafe { DeviceAtomicU32::from_ptr(counters.add(1)) }.store(1, Relaxed);
    false
}

#[cuda_device::cuda_module]
pub mod hd_kernels {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn hd_generate(input: *mut u64, rows: u32, distinct: u32, factor: u32, local: bool) {
        let mut i = thread::blockIdx_x() * 256 + thread::threadIdx_x();
        let stride = thread::gridDim_x() * 256;
        while i < rows {
            // First distributed pass guarantees full domain coverage. Later
            // draws are deterministic, near-uniform, with replacement.
            let id = if local { i / factor } else if i < distinct { i }
                else { (hd_mix(i as u64) % distinct as u64) as u32 };
            let key = (id as u64).wrapping_mul(HD_MULT) & ((1u64 << HD_BITS) - 1);
            unsafe { *input.add(i as usize) = key; }
            i += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn hd_insert(input: *const u64, rows: u32, table: *mut u64, mask: u32,
                     output: *mut u64, capacity: u32, counters: *mut u32, prefilter: bool) {
        let lane = warp::lane_id();
        let mut warp_start = thread::blockIdx_x() * 256 + thread::threadIdx_x() - lane;
        let stride = thread::gridDim_x() * 256;
        while warp_start < rows {
            let i = warp_start + lane;
            let valid = i < rows;
            let key = if valid { unsafe { *input.add(i as usize) } } else { 0 };
            let leader = !prefilter || warp::match_any_i64_sync(u32::MAX, key).trailing_zeros() == lane;
            let won = valid && leader && unsafe { hd_claim(table, mask, key, counters) };
            let winners = warp::ballot_sync(u32::MAX, won);
            if winners != 0 {
                let mut base = 0;
                if lane == 0 { base = unsafe { DeviceAtomicU32::from_ptr(counters) }.fetch_add(winners.count_ones(), Relaxed); }
                let base = warp::shuffle_u64_sync(u32::MAX, base as u64, 0) as u32;
                if won {
                    let at = base + (winners & warp::lanemask_lt()).count_ones();
                    if at < capacity { unsafe { *output.add(at as usize) = key; } }
                    else { unsafe { DeviceAtomicU32::from_ptr(counters.add(1)) }.store(1, Relaxed); }
                }
            }
            warp_start += stride;
        }
    }
}
