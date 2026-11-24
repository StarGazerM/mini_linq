//! Semi-join operators for the synthetic Class-A benchmark. All inputs are SoA u64.
#[cuda_device::cuda_module]
pub mod probe_kernels {
    use cuda_device::{device, kernel, launch_bounds, launch_contract, thread, warp};

    #[device]
    fn probe_lane() -> (u32, u32) {
        (thread::blockIdx_x() * 256 + thread::threadIdx_x(), thread::gridDim_x() * 256)
    }

    #[device]
    unsafe fn probe_contains(keys: *const u64, begin: u32, end: u32, b: u64) -> bool {
        let (mut lo, mut hi) = (begin, end);
        while lo < hi {
            let mid = lo + ((hi - lo) >> 1);
            if unsafe { *keys.add(mid as usize) } < b { lo = mid + 1; } else { hi = mid; }
        }
        lo < end && unsafe { *keys.add(lo as usize) == b }
    }

    #[device]
    unsafe fn probe_finish(id: u32, mut count: u64, mut sum: u64, out: *mut u64) {
        let mut d = 16;
        while d > 0 {
            count += warp::shuffle_down_u64(count, d);
            sum = sum.wrapping_add(warp::shuffle_down_u64(sum, d));
            d /= 2;
        }
        if id & 31 == 0 { unsafe {
            let p = out.add((id >> 5) as usize * 2);
            *p = count; *p.add(1) = sum;
        } }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn search_runs(a: *const u64, b: *const u64, n: u32, keys: *const u64,
        run_len: u32, runs: u32, out: *mut u64) {
        let (id, step) = probe_lane();
        let (mut i, mut count, mut sum) = (id, 0, 0u64);
        while i < n {
            let x = unsafe { *b.add(i as usize) };
            let mut r = 0;
            while r < runs {
                if unsafe { probe_contains(keys, r * run_len, (r + 1) * run_len, x) } {
                    sum = sum.wrapping_add(unsafe { *a.add(i as usize) } ^ x.rotate_left(21));
                    count += 1;
                    break;
                }
                r += 1;
            }
            i += step;
        }
        unsafe { probe_finish(id, count, sum, out); }
    }

    // Equal b values are contiguous. Exactly one membership test per distinct b;
    // all rows read their group result in the subsequent expand kernel. No clearing
    // needed because each queried slot is assigned before the expand launch.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn distinct_heads(b: *const u64, n: u32, keys: *const u64, m: u32, hit: *mut u8) {
        let (mut i, step) = probe_lane();
        while i < n {
            let x = unsafe { *b.add(i as usize) };
            if i == 0 || unsafe { *b.add((i-1) as usize) } != x {
                unsafe { *hit.add(x as usize) = probe_contains(keys, 0, m, x) as u8; }
            }
            i += step;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn expand_heads(a: *const u64, b: *const u64, n: u32, hit: *const u8, out: *mut u64) {
        let (id, step) = probe_lane();
        let (mut i, mut count, mut sum) = (id, 0, 0u64);
        while i < n {
            let x = unsafe { *b.add(i as usize) };
            if unsafe { *hit.add(x as usize) } != 0 {
                sum = sum.wrapping_add(unsafe { *a.add(i as usize) } ^ x.rotate_left(21));
                count += 1;
            }
            i += step;
        }
        unsafe { probe_finish(id, count, sum, out); }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn bitmap_probe(a: *const u64, b: *const u64, n: u32, bits: *const u32, out: *mut u64) {
        let (id, step) = probe_lane();
        let (mut i, mut count, mut sum) = (id, 0, 0u64);
        while i < n {
            let x = unsafe { *b.add(i as usize) };
            if (unsafe { *bits.add((x >> 5) as usize) } >> (x as u32 & 31)) & 1 != 0 {
                sum = sum.wrapping_add(unsafe { *a.add(i as usize) } ^ x.rotate_left(21));
                count += 1;
            }
            i += step;
        }
        unsafe { probe_finish(id, count, sum, out); }
    }

    // Co-rank a diagonal of the stable merged stream; index keys precede delta
    // rows on ties. The last consumed index key is sufficient, even when a
    // duplicate group crosses any number of partition boundaries.
    #[device]
    unsafe fn probe_delta_rank(b: *const u64, n: u32, keys: *const u64, m: u32, diag: u32) -> u32 {
        let (mut lo, mut hi) = (diag.saturating_sub(m), diag.min(n));
        while lo < hi {
            let p = lo + (hi - lo) / 2;
            let q = diag - p;
            if q > 0 && unsafe { *b.add(p as usize) < *keys.add((q-1) as usize) } { lo = p + 1; }
            else { hi = p; }
        }
        lo
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn merge_path(a: *const u64, b: *const u64, n: u32, keys: *const u64, m: u32, out: *mut u64) {
        const TILE: u32 = 32;
        let (id, step) = probe_lane();
        let (mut start, mut count, mut sum) = (id * TILE, 0, 0u64);
        while start < n + m {
            let mut p = unsafe { probe_delta_rank(b, n, keys, m, start) };
            let mut q = start - p;
            let end = (n + m).min(start + TILE);
            let mut last = if q == 0 { 0 } else { unsafe { *keys.add((q - 1) as usize) } };
            while p + q < end {
                if p == n { break; }
                let x = unsafe { *b.add(p as usize) };
                if q != m && unsafe { *keys.add(q as usize) } <= x {
                    last = unsafe { *keys.add(q as usize) }; q += 1;
                } else {
                    if q > 0 && last == x {
                        sum = sum.wrapping_add(unsafe { *a.add(p as usize) } ^ x.rotate_left(21));
                        count += 1;
                    }
                    p += 1;
                }
            }
            start += step * TILE;
        }
        unsafe { probe_finish(id, count, sum, out); }
    }
}
