//! Exact membership cache for a large monotone set of packed pairs. The
//! committed runs remain authoritative; this index can always be discarded.
use crate::{Buffer, Packing, Pool, Result};
use cuda_core::{DeviceCopy, IntoResult, LaunchConfig1D};
use std::sync::{Arc, OnceLock, atomic::{AtomicBool, Ordering}};

// Only contend for this extra memory once a large *admission* actually occurs.
const MIN_COMMITTED: usize = 1 << 24;
const MIN_QUERY: usize = 1 << 19;
const MAX_CODES: u32 = 1 << 22;
const MAX_RANKS: u32 = 1 << 16;
const LEAF: u32 = 2048;
const MAX_BYTES: usize = 2 << 30;
static IN_USE: AtomicBool = AtomicBool::new(false);
struct Lease;
impl Drop for Lease {
    fn drop(&mut self) { IN_USE.store(false, Ordering::Release); }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Shape { shift: u32, major: u32, minor: u32, ranks: u32, pages: u32 }
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct View {
    s: Shape,
    rank: *mut u32, // major code -> dense rank + 1
    dir: *mut u32,  // (rank / LEAF, minor code) -> bitmap page + 1
    bits: *mut u32,
    meta: *mut u32, // next rank, next page, overflow
}
// SAFETY: all pointers refer to same-stream buffers owned by Index.
unsafe impl DeviceCopy for View {}

#[derive(Default)]
pub(crate) struct Cache {
    plan: OnceLock<Option<Shape>>,
    attempted: bool,
    pub(crate) index: Option<Box<Index>>,
}
impl Cache {
    pub(crate) fn consider(&self, committed: impl FnOnce() -> usize, query: usize, packing: &Packing, order: Option<&[usize]>) {
        if query < MIN_QUERY || self.plan.get().is_some() || committed() < MIN_COMMITTED {
            return;
        }
        self.plan.get_or_init(|| {
            let mut widths = packing.widths();
            if widths.len() != 2 || std::env::var_os("GPU_DATALOG_PAIR_BITMAP").as_deref() == Some(std::ffi::OsStr::new("0")) {
                return None;
            }
            let (a, b) = (widths.next()?, widths.next()?);
            let (a, b) = match order {
                None | Some([0, 1]) => (a, b),
                Some([1, 0]) => (b, a),
                _ => return None,
            };
            if a > 32 || b > 32 { return None; }
            let domain = |bits: u32| (1u64 << bits).min(MAX_CODES as u64) as u32;
            let major = domain(a);
            Some(Shape { shift: b, major, minor: domain(b), ranks: major.min(MAX_RANKS), pages: 0 })
        });
    }
    pub(crate) fn update_needed(&self) -> bool {
        self.index.is_some() || !self.attempted && self.plan.get().is_some_and(Option::is_some)
    }
    pub(crate) fn begin(&mut self, pool: &Arc<Pool>) -> Result<Option<Box<Index>>> {
        self.attempted = true;
        let Some(s) = self.plan.get().copied().flatten() else { return Ok(None); };
        pool.context().bind_to_thread()?;
        let (mut free, mut total) = (0, 0);
        unsafe { cuda_core::sys::cuMemGetInfo_v2(&mut free, &mut total).result()?; }
        // Leave at least 3/4 of currently free VRAM and 15/16 of the card for
        // relation data and scratch. A process-wide lease caps simultaneous caches.
        let budget = MAX_BYTES.min(total / 16).min(free / 4);
        let directory_entries = s.ranks.div_ceil(LEAF) as usize * s.minor as usize;
        let fixed = 4 * (s.major as usize + directory_entries + 3);
        let available_pages = budget.saturating_sub(fixed) / (LEAF as usize / 8);
        if available_pages < directory_entries.min(1 << 19)
            || IN_USE.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err()
        {
            return Ok(None);
        }
        let pages = available_pages.min(directory_entries) as u32;
        let index = Index::new(pool, Shape { pages, ..s }, Some(Lease))?;
        Ok(Some(Box::new(index)))
    }
}

pub(crate) struct Index {
    shape: Shape,
    rank: Buffer<u32>,
    dir: Buffer<u32>,
    bits: Buffer<u32>,
    meta: Buffer<u32>,
    module: kernels::LoadedModule,
    _lease: Option<Lease>,
}
impl Index {
    #[cfg(test)]
    pub(crate) fn tiny(pool: &Arc<Pool>) -> Result<Box<Self>> {
        Ok(Box::new(Self::new(pool, Shape { shift: 8, major: 8, minor: 8, ranks: 2, pages: 2 }, None)?))
    }
    fn new(pool: &Arc<Pool>, s: Shape, lease: Option<Lease>) -> Result<Self> {
        Ok(Self {
            shape: s,
            rank: Buffer::zeroed(pool.clone(), s.major as usize)?,
            dir: Buffer::zeroed(pool.clone(), s.ranks.div_ceil(LEAF) as usize * s.minor as usize)?,
            bits: Buffer::zeroed(pool.clone(), s.pages as usize * (LEAF / 32) as usize)?,
            meta: Buffer::zeroed(pool.clone(), 3)?,
            module: unsafe { kernels::load(pool.context())? },
            _lease: lease,
        })
    }
    fn view(&self) -> View {
        View { s: self.shape, rank: self.rank.ptr(), dir: self.dir.ptr(), bits: self.bits.ptr(), meta: self.meta.ptr() }
    }
    pub(crate) fn bytes(&self) -> usize {
        self.rank.bytes() + self.dir.bytes() + self.bits.bytes() + self.meta.bytes()
    }
    pub(crate) fn insert(&self, keys: *const u64, n: usize) -> Result<()> {
        for off in (0..n).step_by(u32::MAX as usize) {
            let count = (n - off).min(u32::MAX as usize) as u32;
            let keys = keys.wrapping_add(off);
            let cfg = || LaunchConfig1D::new(count.div_ceil(256), 256, 0);
            let stream = self.meta.pool().stream();
            self.module.pair_ranks(stream, &self.module.prepare_pair_ranks(cfg())?, keys, count, self.view())?;
            self.module.pair_pages(stream, &self.module.prepare_pair_pages(cfg())?, keys, count, self.view())?;
            self.module.pair_insert(stream, &self.module.prepare_pair_insert(cfg())?, keys, count, self.view())?;
        }
        Ok(())
    }
    /// Only complete caches may be used for subtraction. Overflow never means absent.
    pub(crate) fn complete(&self) -> Result<bool> {
        let pool = self.meta.pool();
        let mut overflow = 0u32;
        // SAFETY: meta[2] is initialized; the stack target lives through sync.
        unsafe {
            cuda_core::simt::memory::memcpy_dtoh_async(
                &mut overflow, self.meta.cu_deviceptr() + 8, 4, pool.stream().cu_stream(),
            )?;
        }
        crate::record_readback();
        pool.stream().synchronize()?;
        Ok(overflow == 0)
    }
    pub(crate) fn absent(&self, keys: *const u64, n: usize, flags: *mut u32) -> Result<()> {
        for off in (0..n).step_by(u32::MAX as usize) {
            let count = (n - off).min(u32::MAX as usize) as u32;
            let cfg = LaunchConfig1D::new(count.div_ceil(256), 256, 0);
            self.module.pair_absent(self.meta.pool().stream(), &self.module.prepare_pair_absent(cfg)?,
                keys.wrapping_add(off), count, self.view(), flags.wrapping_add(off))?;
        }
        Ok(())
    }
}

#[cuda_device::cuda_module]
pub mod kernels {
    use super::*;
    use cuda_device::{atomic::{AtomicOrdering::Relaxed, DeviceAtomicU32}, device, kernel, launch_bounds, launch_contract, thread};

    #[device]
    fn tid() -> usize { (thread::blockIdx_x() * 256 + thread::threadIdx_x()) as usize }

    #[device]
    unsafe fn overflow(t: View) {
        unsafe { DeviceAtomicU32::from_ptr(t.meta.add(2)) }.store(1, Relaxed);
    }

    #[device]
    unsafe fn location(key: u64, t: View) -> Option<(usize, u32)> {
        let hi = key >> t.s.shift;
        let lo = key & ((1u64 << t.s.shift) - 1);
        if hi >= t.s.major as u64 || lo >= t.s.minor as u64 { return None; }
        let rank = unsafe { *t.rank.add(hi as usize) };
        if rank == 0 { return None; }
        let bit = rank - 1;
        Some(((bit / LEAF * t.s.minor + lo as u32) as usize, bit % LEAF))
    }
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn pair_ranks(keys: *const u64, n: u32, t: View) {
        let i = tid();
        if i >= n as usize { return; }
        // Sorted input: only the start of each high-code group assigns a rank.
        unsafe {
            let hi = *keys.add(i) >> t.s.shift;
            if i != 0 && *keys.add(i - 1) >> t.s.shift == hi { return; }
            if hi >= t.s.major as u64 { overflow(t); return; }
            let p = t.rank.add(hi as usize);
            if *p == 0 {
                let rank = DeviceAtomicU32::from_ptr(t.meta).fetch_add(1, Relaxed);
                if rank < t.s.ranks { *p = rank + 1; } else { overflow(t); }
            }
        }
    }
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn pair_pages(keys: *const u64, n: u32, t: View) {
        let i = tid();
        if i >= n as usize { return; }
        unsafe {
            let Some((slot, _)) = location(*keys.add(i), t) else { overflow(t); return; };
            let p = DeviceAtomicU32::from_ptr(t.dir.add(slot));
            // The winner allocates. No other thread waits for it: the next
            // stream-ordered kernel sees all completed page assignments.
            if p.load(Relaxed) == 0 && p.compare_exchange(0, u32::MAX, Relaxed, Relaxed).is_ok() {
                let page = DeviceAtomicU32::from_ptr(t.meta.add(1)).fetch_add(1, Relaxed);
                if page < t.s.pages { p.store(page + 1, Relaxed); } else { overflow(t); }
            }
        }
    }
    #[device]
    unsafe fn word(key: u64, t: View) -> Option<(*mut u32, u32)> {
        let (slot, bit) = unsafe { location(key, t) }?;
        let page = unsafe { *t.dir.add(slot) };
        if page == 0 || page > t.s.pages { return None; }
        Some((unsafe { t.bits.add(((page - 1) * (LEAF / 32) + bit / 32) as usize) }, 1 << (bit % 32)))
    }
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn pair_insert(keys: *const u64, n: u32, t: View) {
        let i = tid();
        if i < n as usize {
            unsafe {
                if let Some((p, mask)) = word(*keys.add(i), t) {
                    DeviceAtomicU32::from_ptr(p).fetch_or(mask, Relaxed);
                }
            }
        }
    }
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn pair_absent(keys: *const u64, n: u32, t: View, flags: *mut u32) {
        let i = tid();
        if i < n as usize {
            unsafe {
                let found = match word(*keys.add(i), t) {
                    Some((p, mask)) => *p & mask != 0,
                    None => false,
                };
                *flags.add(i) = (!found) as u32;
            }
        }
    }
}
