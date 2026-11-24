//! Real-data hole 136 replay. Tables are full, sorted rows in capture order.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Table {
    pub words: *const u32,
    pub rows: u32,
}
unsafe impl cuda_core::DeviceCopy for Table {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Lookups {
    pub method: Table,
    pub heap_type: Table,
    pub reachable: Table,
    pub name: Table,
    pub descriptor: Table,
    pub lookup: Table,
    pub this_var: Table,
    pub base: Table,
}
unsafe impl cuda_core::DeviceCopy for Lookups {}

#[cuda_device::cuda_module]
pub mod replay_kernels {
    use super::*;
    use cuda_device::{device, kernel, launch_bounds, launch_contract, thread, warp};
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32};

    #[device]
    fn replay_lane() -> u32 { thread::blockIdx_x() * 256 + thread::threadIdx_x() }

    #[device]
    unsafe fn replay_lower(t: Table, stride: usize, key: u32) -> u32 {
        let (mut lo, mut hi) = (0, t.rows);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if unsafe { *t.words.add(mid as usize * stride) } < key { lo = mid + 1; } else { hi = mid; }
        }
        lo
    }

    // A matching 2-column prefix range is consumed until the first nonmatch;
    // no unique-key assumptions, and one lower_bound rather than two.
    #[device]
    unsafe fn replay_next(t: Table, at: &mut u32, key: u32) -> Option<u32> {
        if *at == t.rows { return None; }
        let p = unsafe { t.words.add(*at as usize * 2) };
        if unsafe { *p } != key { return None; }
        *at += 1;
        Some(unsafe { *p.add(1) })
    }

    #[device]
    unsafe fn replay_lookup_lower(t: Table, name: u32, desc: u32, ty: u32) -> u32 {
        let (mut lo, mut hi) = (0, t.rows);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let p = unsafe { t.words.add(mid as usize * 4) };
            let less = unsafe { *p < name || (*p == name && (*p.add(1) < desc || (*p.add(1) == desc && *p.add(2) < ty))) };
            if less { lo = mid + 1; } else { hi = mid; }
        }
        lo
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn replay_build(t: Table, bits: *mut u32) {
        let i = replay_lane();
        if i < t.rows { unsafe {
            let key = *t.words.add(i as usize * 2);
            DeviceAtomicU32::from_ptr(bits.add((key >> 5) as usize)).fetch_or(1 << (key & 31), AtomicOrdering::Relaxed);
        } }
    }

    /// Warp-aggregated append. Lane order is preserved within each warp, not
    /// across warps; Datalog candidate multisets are independent of row order.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn replay_filter(rows: *const u64, n: u32, bits: *const u32, nwords: u32, kept: *mut u64, used: *mut u32) {
        let i = replay_lane();
        let row = if i < n { unsafe { *rows.add(i as usize) } } else { 0 };
        let key = (row >> 32) as u32;
        let yes = i < n && key >> 5 < nwords && unsafe { *bits.add((key >> 5) as usize) } & (1 << (key & 31)) != 0;
        let mask = warp::ballot_sync(u32::MAX, yes);
        if mask != 0 {
            let start = if warp::lane_id() == 0 {
                unsafe { DeviceAtomicU32::from_ptr(used) }.fetch_add(mask.count_ones(), AtomicOrdering::Relaxed)
            } else { 0 };
            let at = warp::shuffle_u64_sync(u32::MAX, start as u64, 0) as u32;
            if yes { unsafe { *kept.add((at + (mask & warp::lanemask_lt()).count_ones()) as usize) = row; } }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn replay_chain(rows: *const u64, n: *const u32, t: Lookups, out: *mut u64, capacity: u32, count: *mut u32) {
        let mut i = replay_lane();
        let n = unsafe { *n };
        while i < n { unsafe {
            let row = *rows.add(i as usize);
            let (heap, base) = (row as u32, (row >> 32) as u32);
            let mut a = replay_lower(t.base, 2, base);
            while let Some(inv) = replay_next(t.base, &mut a, base) {
                let mut b = replay_lower(t.method, 2, inv);
                while let Some(method) = replay_next(t.method, &mut b, inv) {
                    let c = replay_lower(t.reachable, 1, method);
                    if c == t.reachable.rows || *t.reachable.words.add(c as usize) != method { continue; }
                    let mut d = replay_lower(t.heap_type, 2, heap);
                    while let Some(ty) = replay_next(t.heap_type, &mut d, heap) {
                        let mut e = replay_lower(t.name, 2, inv);
                        while let Some(name) = replay_next(t.name, &mut e, inv) {
                            let mut f = replay_lower(t.descriptor, 2, inv);
                            while let Some(desc) = replay_next(t.descriptor, &mut f, inv) {
                                let mut g = replay_lookup_lower(t.lookup, name, desc, ty);
                                while g < t.lookup.rows {
                                    let p = t.lookup.words.add(g as usize * 4);
                                    if *p != name || *p.add(1) != desc || *p.add(2) != ty { break; }
                                    let target = *p.add(3);
                                    let mut h = replay_lower(t.this_var, 2, target);
                                    while let Some(this) = replay_next(t.this_var, &mut h, target) {
                                        let slot = DeviceAtomicU32::from_ptr(count).fetch_add(1, AtomicOrdering::Relaxed);
                                        if slot < capacity { *out.add(slot as usize) = (heap as u64) << 32 | this as u64; }
                                    }
                                    g += 1;
                                }
                            }
                        }
                    }
                }
            }
            i += thread::gridDim_x() * 256;
        } }
    }
}
