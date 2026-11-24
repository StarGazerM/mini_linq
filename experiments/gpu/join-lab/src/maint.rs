//! Stable admission benchmark: sorted unique input minus disjoint sorted runs.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MaintRun { pub ptr: *const u64, pub n: u32 }
unsafe impl cuda_core::DeviceCopy for MaintRun {}

pub const TILE: u32 = 2048;
pub const SHARED: u32 = (TILE + 1) * 8;

/// Monotonic pair-membership side index. Zero directory entries are empty;
/// IDs are assigned on first insertion. A leaf covers 2048 active heap ranks
/// for one variable. Capacity overflow is explicit, not a false negative.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MaintBitmap {
    pub heaps: *mut u32, pub directory: *mut u32, pub bits: *mut u32,
    pub meta: *mut u32, pub codes: u32, pub heap_limit: u32, pub page_limit: u32,
}
unsafe impl cuda_core::DeviceCopy for MaintBitmap {}

#[cuda_device::cuda_module]
pub mod maint_kernels {
    use super::*;
    use cuda_device::{device, kernel, launch_bounds, launch_contract, thread, DynamicSharedArray};

    #[device]
    unsafe fn maint_bound(p: *const u64, mut lo: u32, mut hi: u32, k: u64) -> u32 {
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if unsafe { *p.add(mid as usize) } < k { lo = mid + 1 } else { hi = mid }
        }
        lo
    }

    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn maint_binary(a: *const u64, n: u32, runs: *const MaintRun, k: u32, flags: *mut u32) {
        let i = thread::blockIdx_x() * 256 + thread::threadIdx_x();
        if i >= n { return; }
        unsafe {
            let key = *a.add(i as usize);
            let mut r = 0;
            while r < k {
                let run = *runs.add(r as usize);
                let j = maint_bound(run.ptr, 0, run.n, key);
                if j < run.n && *run.ptr.add(j as usize) == key { break; }
                r += 1;
            }
            *flags.add(i as usize) = (r == k) as u32;
        }
    }

    // Stable merge diagonal with A first on equal keys. This tie rule lets a
    // candidate test the current B key, even when a tile boundary splits a tie.
    #[device]
    unsafe fn maint_rank(a: *const u64, n: u32, b: *const u64, m: u32, d: u32) -> u32 {
        let (mut lo, mut hi) = (d.saturating_sub(m), n.min(d));
        while lo < hi {
            let i = lo + (hi - lo) / 2;
            let j = d - i;
            if j > 0 && unsafe { *b.add((j - 1) as usize) >= *a.add(i as usize) } { lo = i + 1; } else { hi = i; }
        }
        lo
    }

    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn maint_cuts(a: *const u64, n: u32, b: MaintRun, cuts: *mut u32) {
        let tile = thread::blockIdx_x() * 256 + thread::threadIdx_x();
        if tile <= (n + b.n).div_ceil(TILE) {
            let diag = (tile * TILE).min(n + b.n);
            unsafe { *cuts.add(tile as usize) = maint_rank(a, n, b.ptr, b.n, diag); }
        }
    }

    #[kernel] #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (16392, 16392))]
    pub fn maint_merge(a: *const u64, n: u32, b: MaintRun, cuts: *const u32, first: bool, flags: *mut u32) {
        let tile = thread::blockIdx_x();
        let lane = thread::threadIdx_x();
        let start = tile * TILE;
        let len = TILE.min(n + b.n - start);
        unsafe {
            let a0 = *cuts.add(tile as usize);
            let na = *cuts.add(tile as usize + 1) - a0;
            if na == 0 { return; }
            let b0 = start - a0;
            let nb = len - na;
            let sa = DynamicSharedArray::<u64, 8>::get_raw().cast::<u64>();
            let sb = sa.add(na as usize);
            let mut i = lane;
            while i < na { *sa.add(i as usize) = *a.add((a0 + i) as usize); i += 256; }
            i = lane;
            // One B lookahead for equality at the tile's right edge.
            while i <= nb && b0 + i < b.n { *sb.add(i as usize) = *b.ptr.add((b0 + i) as usize); i += 256; }
            thread::sync_threads();
            let diag = lane * (TILE / 256);
            if diag >= len { return; }
            let mut x = maint_rank(sa, na, sb, nb, diag);
            let mut y = diag - x;
            let mut d = diag;
            while d < (diag + TILE / 256).min(len) && x < na {
                let v = *sa.add(x as usize);
                if y == nb || v <= *sb.add(y as usize) {
                    let hit = b0 + y < b.n && v == *sb.add(y as usize);
                    if first || hit { *flags.add((a0 + x) as usize) = (!hit) as u32; }
                    x += 1;
                } else { y += 1; }
                d += 1;
            }
        }
    }

    // Candidate-balanced alternative for sparse A: partition each B at the
    // first key of each 128-row candidate tile, without streaming untouched B.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn maint_windows(a: *const u64, n: u32, runs: *const MaintRun, k: u32, windows: *mut u32) {
        let tile = thread::blockIdx_x() * 256 + thread::threadIdx_x();
        let tiles = n.div_ceil(128);
        if tile <= tiles { unsafe {
            let mut r = 0;
            while r < k {
                let b = *runs.add(r as usize);
                *windows.add((r * (tiles + 1) + tile) as usize) = if tile == tiles { b.n }
                    else { maint_bound(b.ptr, 0, b.n, *a.add((tile * 128) as usize)) };
                r += 1;
            }
        } }
    }

    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn maint_bounded(a: *const u64, n: u32, runs: *const MaintRun, k: u32, windows: *const u32, flags: *mut u32) {
        let i = thread::blockIdx_x() * 256 + thread::threadIdx_x();
        if i >= n { return; }
        unsafe {
            let key = *a.add(i as usize);
            let width = n.div_ceil(128) + 1;
            let mut r = 0;
            while r < k {
                let b = *runs.add(r as usize);
                let p = windows.add((r * width + i / 128) as usize);
                let end = *p.add(1);
                let j = maint_bound(b.ptr, *p, end, key);
                if j < end && *b.ptr.add(j as usize) == key { break; }
                r += 1;
            }
            *flags.add(i as usize) = (r == k) as u32;
        }
    }

    #[device]
    unsafe fn maint_overflow(t: MaintBitmap) {
        use cuda_device::atomic::{AtomicOrdering::Relaxed, DeviceAtomicU32};
        unsafe { DeviceAtomicU32::from_ptr(t.meta.add(2)) }.store(1, Relaxed);
    }

    /// Input is sorted (heap,var), so exactly one thread per heap may assign it.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn maint_bitmap_heaps(keys: *const u64, n: u32, t: MaintBitmap) {
        use cuda_device::atomic::{AtomicOrdering::Relaxed, DeviceAtomicU32};
        let i = thread::blockIdx_x() * 256 + thread::threadIdx_x();
        if i >= n { return; }
        unsafe {
            let h = (*keys.add(i as usize) >> 32) as u32;
            if i != 0 && (*keys.add(i as usize - 1) >> 32) as u32 == h { return; }
            if h >= t.codes { maint_overflow(t); return; }
            let p = t.heaps.add(h as usize);
            if *p == 0 {
                let rank = DeviceAtomicU32::from_ptr(t.meta).fetch_add(1, Relaxed);
                if rank < t.heap_limit { *p = rank + 1; } else { maint_overflow(t); }
            }
        }
    }

    #[device]
    unsafe fn maint_bitmap_slot(key: u64, t: MaintBitmap) -> Option<(usize,u32)> {
        let (h,v) = ((key >> 32) as u32,key as u32);
        if h >= t.codes || v >= t.codes { return None; }
        let r = unsafe { *t.heaps.add(h as usize) };
        if r == 0 { return None; }
        let rank = r-1;
        Some((((rank >> 11) * t.codes + v) as usize, rank & 2047))
    }

    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn maint_bitmap_pages(keys: *const u64, n: u32, t: MaintBitmap) {
        use cuda_device::atomic::{AtomicOrdering::Relaxed, DeviceAtomicU32};
        let i = thread::blockIdx_x() * 256 + thread::threadIdx_x();
        if i >= n { return; }
        unsafe {
            let Some((slot,_)) = maint_bitmap_slot(*keys.add(i as usize), t) else { maint_overflow(t); return; };
            let page = DeviceAtomicU32::from_ptr(t.directory.add(slot));
            if page.load(Relaxed) == 0 && page.compare_exchange(0,u32::MAX,Relaxed,Relaxed).is_ok() {
                let id = DeviceAtomicU32::from_ptr(t.meta.add(1)).fetch_add(1,Relaxed);
                if id < t.page_limit { page.store(id+1,Relaxed); } else { maint_overflow(t); }
            }
        }
    }

    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn maint_bitmap_test_set(keys: *const u64, n: u32, t: MaintBitmap, flags: *mut u32) {
        use cuda_device::atomic::{AtomicOrdering::Relaxed, DeviceAtomicU32};
        let i = thread::blockIdx_x() * 256 + thread::threadIdx_x();
        if i >= n { return; }
        unsafe {
            let Some((slot,bit)) = maint_bitmap_slot(*keys.add(i as usize), t) else { maint_overflow(t); return; };
            let page = *t.directory.add(slot);
            if page == 0 || page > t.page_limit { maint_overflow(t); return; }
            let word = DeviceAtomicU32::from_ptr(t.bits.add(((page-1) * 64 + bit/32) as usize));
            let mask = 1u32 << (bit & 31);
            let before = word.fetch_or(mask,Relaxed);
            if !flags.is_null() { *flags.add(i as usize) = ((before & mask) == 0) as u32; }
        }
    }

    /// Atomically admit a nonzero packed word (zero is reserved by this
    /// benchmark). Every run uses the same monotonic set, independent of LSM.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn maint_hash_insert(keys: *const u64, n: u32, table: *mut u64, mask: u32, flags: *mut u32) {
        use cuda_device::atomic::{AtomicOrdering::Relaxed, DeviceAtomicU64};
        let i = thread::blockIdx_x() * 256 + thread::threadIdx_x();
        if i >= n { return; }
        unsafe {
            let key = *keys.add(i as usize);
            let h = ((key ^ (key >> 32)).wrapping_mul(0x9e3779b97f4a7c15)) ^ key.rotate_left(23);
            let mut j = (h >> 32) as u32 & mask;
            let fresh = loop {
                let cell = DeviceAtomicU64::from_ptr(table.add(j as usize));
                match cell.compare_exchange(0,key,Relaxed,Relaxed) {
                    Ok(_) => break true, Err(v) if v == key => break false, _ => { j=(j+1)&mask; }
                }
            };
            if !flags.is_null() { *flags.add(i as usize) = fresh as u32; }
        }
    }
}
