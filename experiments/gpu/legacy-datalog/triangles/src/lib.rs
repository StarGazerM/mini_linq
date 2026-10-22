//! Directed set triangles over a sorted, duplicate-free edge run.
//!
//! Input canonicalization is a host ingestion step, not part of device timing.
//! Dense domains use gpu-blocked-run's rank directory; over-budget domains use
//! 2048-row fences and binary search. A descriptor is 14,340 bytes, independent
//! of fanout. Pool locks protect both cursors and descriptor lifetime: a thief
//! copies under the lock before releasing it, so no outstanding reader can race
//! slot reuse. No block waits for another block to finish a chunk.

use cuda_core::{CudaContext, CudaStream, DeviceCopy, LaunchConfig1D};
use gpu_blocked_run::{Buffer, MemoryResource, RankView, Run};
use rccl_memory::ContiguousDeviceRange;

pub type Error = Box<dyn std::error::Error>;
pub const T: usize = 1024;
pub const DESCRIPTOR_WORDS: usize = 3585;
pub const SLOT_WORDS: usize = DESCRIPTOR_WORDS + 3;
pub const SHARED_BYTES: u32 = ((DESCRIPTOR_WORDS + 32) * 4) as u32;
pub const RANK_BUDGET: u64 = 64 << 20;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct GraphView {
    pub rank: RankView,
    pub keys: *const i64,
    pub neighbors: *const i64,
    pub fences: *const i64,
    pub rows: u32,
    pub dense: u32,
}
unsafe impl DeviceCopy for GraphView {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
#[repr(C)]
pub struct Triple {
    pub a: i64,
    pub b: i64,
    pub c: i64,
}
unsafe impl DeviceCopy for Triple {}

/// The counters are u64 to retain triangle counts above 2^32. `remaining`
/// counts owners, not chunks: thieves finish their chunk before checking it.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Stats {
    pub count: u64,
    pub tile_cursor: u64,
    pub remaining: u64,
    pub steals: u64,
    pub published: u64,
    pub pool_full: u64,
    pub max_expansion: u64,
    pub overflow: u64,
}
unsafe impl DeviceCopy for Stats {}

pub fn ptr<T: DeviceCopy>(buffer: &impl ContiguousDeviceRange<T>) -> *mut T {
    buffer.cu_deviceptr() as *mut T
}

pub fn dense_domain(domain: (i64, i64), rows: usize) -> bool {
    let span = (domain.1 as i128 - domain.0 as i128 + 1).max(0) as u128;
    // Include the builder's two full-domain scratch arrays in the budget.
    span <= (1u128 << 32)
        && span * 8 + span.div_ceil(64) * 16 <= RANK_BUDGET as u128
        && span <= (rows as u128 * 32).max(4096)
}

pub enum Graph<'a> {
    Rank(Run<'a>),
    Fences {
        keys: Buffer<'a, i64>,
        neighbors: Buffer<'a, i64>,
        fences: Buffer<'a, i64>,
    },
}
impl<'a> Graph<'a> {
    /// Input must be lexicographically sorted and distinct (checked by runner).
    pub fn build(
        ctx: &CudaContext,
        stream: &'a CudaStream,
        memory: &'a MemoryResource,
        keys: &impl ContiguousDeviceRange<i64>,
        neighbors: &impl ContiguousDeviceRange<i64>,
        domain: (i64, i64),
    ) -> Result<Self, Error> {
        if keys.len() != neighbors.len() || keys.len() > u32::MAX as usize {
            return Err("invalid edge column cardinalities".into());
        }
        if dense_domain(domain, keys.len()) {
            let run = Run::build(ctx, stream, memory, keys, &[neighbors], domain, None)?;
            gpu_blocked_run::read_stats(memory, stream, &[&run])?;
            Ok(Self::Rank(run))
        } else {
            let out_keys = Buffer::zeroed_in(memory, stream, keys.len())?;
            let out_neighbors = Buffer::zeroed_in(memory, stream, keys.len())?;
            for (dst, src) in [
                (ptr(&out_keys), keys.cu_deviceptr()),
                (ptr(&out_neighbors), neighbors.cu_deviceptr()),
            ] {
                unsafe {
                    cuda_core::memory::memcpy_dtod_async(
                        dst as u64,
                        src,
                        keys.len() * 8,
                        stream.cu_stream(),
                    )?;
                }
            }
            let fences = Buffer::zeroed_in(memory, stream, keys.len().div_ceil(2048))?;
            let module = unsafe { kernels::load(stream.context())? };
            let launch = module.prepare_make_fences(LaunchConfig1D::new(
                fences.len().div_ceil(256).max(1) as u32,
                256,
                0,
            ))?;
            unsafe {
                module.make_fences(
                    stream,
                    &launch,
                    ptr(&out_keys),
                    keys.len() as u32,
                    ptr(&fences),
                )?;
            }
            Ok(Self::Fences {
                keys: out_keys,
                neighbors: out_neighbors,
                fences,
            })
        }
    }
    pub fn view(&self) -> GraphView {
        match self {
            Self::Rank(run) => GraphView {
                rank: run.view(),
                keys: ptr(&run.keys),
                neighbors: ptr(&run.payloads[0]),
                fences: ptr(&run.fences),
                rows: run.keys.len() as u32,
                dense: 1,
            },
            Self::Fences {
                keys,
                neighbors,
                fences,
            } => GraphView {
                rank: RankView {
                    bits: core::ptr::null(),
                    prefix: core::ptr::null(),
                    offsets: core::ptr::null(),
                    min_key: 0,
                    max_key: -1,
                },
                keys: ptr(keys),
                neighbors: ptr(neighbors),
                fences: ptr(fences),
                rows: keys.len() as u32,
                dense: 0,
            },
        }
    }
}

/// Pool storage and counters are preallocated from the fixed RCCL pool.
pub struct Workspace<'a> {
    pub pool: Buffer<'a, u32>,
    pub stats: Buffer<'a, Stats>,
    pub capacity: u32,
}
impl<'a> Workspace<'a> {
    pub fn new(
        memory: &'a MemoryResource,
        stream: &'a CudaStream,
        capacity: u32,
    ) -> Result<Self, Error> {
        Ok(Self {
            pool: Buffer::zeroed_in(memory, stream, capacity as usize * SLOT_WORDS)?,
            stats: Buffer::zeroed_in(memory, stream, 1)?,
            capacity,
        })
    }
    /// mode 0: triangles; mode 1: materialize every wedge (BFS first stage).
    /// Optional triangle materialization is bounded; overflow is reported, never dropped silently.
    pub fn launch(
        &self,
        stream: &CudaStream,
        graph: GraphView,
        k: u32,
        grid: u32,
        output: *mut Triple,
        output_capacity: u64,
        mode: u32,
    ) -> Result<(), Error> {
        let module = unsafe { kernels::load(stream.context())? };
        let init = module.prepare_triangle_initialize(LaunchConfig1D::new(1, 256, 0))?;
        let run = module.prepare_triangles(LaunchConfig1D::new(grid, 256, SHARED_BYTES))?;
        unsafe {
            cuda_core::memory::memset_d8_async(
                self.pool.cu_deviceptr(),
                0,
                self.pool.len() * 4,
                stream.cu_stream(),
            )?;
            module.triangle_initialize(stream, &init, ptr(&self.stats), graph.rows)?;
            module.triangles(
                stream,
                &run,
                graph,
                ptr(&self.pool),
                self.capacity,
                k,
                ptr(&self.stats),
                output,
                output_capacity,
                mode,
            )?;
        }
        Ok(())
    }
    pub fn result(&self) -> Result<Stats, Error> {
        let result = self.stats.to_host_vec()?[0];
        if result.overflow != 0 {
            return Err(format!("triangle descriptor/output overflow: {}", result.overflow).into());
        }
        Ok(result)
    }
}

use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32, DeviceAtomicU64};
use cuda_device::{device, kernel, launch_bounds, launch_contract, thread, DynamicSharedArray};

#[device]
unsafe fn add(p: *mut u64, value: u64) -> u64 {
    DeviceAtomicU64::from_ptr(p).fetch_add(value, AtomicOrdering::Relaxed)
}
#[device]
unsafe fn lock(p: *mut u32, expected: u32) -> bool {
    DeviceAtomicU32::from_ptr(p)
        .compare_exchange(
            expected,
            1,
            AtomicOrdering::Acquire,
            AtomicOrdering::Relaxed,
        )
        .is_ok()
}
#[device]
unsafe fn unlock(p: *mut u32, state: u32) {
    DeviceAtomicU32::from_ptr(p).store(state, AtomicOrdering::Release);
}
#[device]
unsafe fn range(g: GraphView, key: i64) -> (u32, u32) {
    if g.dense != 0 {
        return gpu_blocked_run::point(g.rank, key).unwrap_or((0, 0));
    }
    // Bound both equal-range searches by fences. Repeated fence keys require
    // lower_bound-1 for the left end and upper_bound for the right end.
    let nf = (g.rows as u64 + 2047) / 2048;
    let mut l = 0u32;
    let mut h = nf as u32;
    while l < h {
        let m = l + (h - l) / 2;
        if *g.fences.add(m as usize) < key {
            l = m + 1;
        } else {
            h = m;
        }
    }
    let begin = l.saturating_sub(1) * 2048;
    l = 0;
    h = nf as u32;
    while l < h {
        let m = l + (h - l) / 2;
        if *g.fences.add(m as usize) <= key {
            l = m + 1;
        } else {
            h = m;
        }
    }
    let end = ((l as u64 * 2048).min(g.rows as u64)) as u32;
    l = begin;
    h = end;
    while l < h {
        let m = l + (h - l) / 2;
        if *g.keys.add(m as usize) < key {
            l = m + 1;
        } else {
            h = m;
        }
    }
    let first = l;
    h = end;
    while l < h {
        let m = l + (h - l) / 2;
        if *g.keys.add(m as usize) <= key {
            l = m + 1;
        } else {
            h = m;
        }
    }
    (first, l)
}
#[device]
unsafe fn contains(values: *const i64, begin: u32, end: u32, key: i64) -> bool {
    let mut l = begin;
    let mut h = end;
    while l < h {
        let m = l + (h - l) / 2;
        if *values.add(m as usize) < key {
            l = m + 1;
        } else {
            h = m;
        }
    }
    l < end && *values.add(l as usize) == key
}

// Descriptor words: 1024 edge-row bindings, 1024 expansion begins, 1025
// exclusive offsets, 1024 u16 parent slots (high bit selects N(a) expansion).
#[device]
unsafe fn consume(
    g: GraphView,
    d: *mut u32,
    start: u32,
    stats: *mut Stats,
    output: *mut Triple,
    capacity: u64,
    mode: u32,
    scratch: *mut u32,
) {
    let tid = thread::threadIdx_x() as usize;
    let offsets = d.add(2048);
    let parents = d.add(3073) as *const u16;
    let total = *offsets.add(1024);
    let mut hits = [false; 4];
    let mut triples = [Triple { a: 0, b: 0, c: 0 }; 4];
    let mut count = 0;
    let mut j = 0;
    while j < 4 {
        let pos = start as u64 + tid as u64 + j as u64 * 256;
        if pos < total as u64 {
            let owner = gpu_tiles::owner(offsets, 1024, pos as u32) as usize;
            let parent = *parents.add(owner);
            let row = *d.add((parent & 1023) as usize) as usize;
            let a = *g.keys.add(row);
            let b = *g.neighbors.add(row);
            let c = *g
                .neighbors
                .add((*d.add(1024 + owner) + pos as u32 - *offsets.add(owner)) as usize);
            let probe = if parent & 32768 != 0 { b } else { a };
            let (lo, hi) = range(g, probe);
            let hit = mode == 1 || contains(g.neighbors, lo, hi, c);
            hits[j] = hit;
            triples[j] = Triple { a, b, c };
            count += hit as u32;
        }
        j += 1;
    }
    let (prefix, block_count) = gpu_tiles::scan(count, scratch);
    if tid == 0 {
        let base = add(core::ptr::addr_of_mut!((*stats).count), block_count as u64);
        *(scratch.add(16) as *mut u64) = base;
        if !output.is_null() && base + block_count as u64 > capacity {
            add(core::ptr::addr_of_mut!((*stats).overflow), 1);
        }
    }
    thread::sync_threads();
    let base = *(scratch.add(16) as *const u64);
    let mut offset = prefix as u64;
    j = 0;
    while j < 4 {
        if hits[j] {
            if !output.is_null() && base + offset < capacity {
                *output.add((base + offset) as usize) = triples[j];
            }
            offset += 1;
        }
        j += 1;
    }
    thread::sync_threads();
}

#[cuda_device::cuda_module]
pub mod kernels {
    use super::*;
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1, block=(256,1,1))]
    pub unsafe fn make_fences(keys: *const i64, rows: u32, fences: *mut i64) {
        let i = thread::index_1d().get();
        if i * 2048 < rows as usize {
            *fences.add(i) = *keys.add(i * 2048);
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1, block=(256,1,1))]
    pub unsafe fn triangle_initialize(stats: *mut Stats, rows: u32) {
        if thread::index_1d().get() == 0 {
            *stats = Stats {
                count: 0,
                tile_cursor: 0,
                remaining: (rows as u64 + 1023) / 1024,
                steals: 0,
                published: 0,
                pool_full: 0,
                max_expansion: 0,
                overflow: 0,
            };
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1, block=(256,1,1), dynamic_shared_range=(14468,14468))]
    pub unsafe fn triangles(
        g: GraphView,
        pool: *mut u32,
        pool_capacity: u32,
        k: u32,
        stats: *mut Stats,
        output: *mut Triple,
        output_capacity: u64,
        mode: u32,
    ) {
        let tid = thread::threadIdx_x() as usize;
        let scratch = DynamicSharedArray::<u8>::get_raw() as *mut u32;
        let d = scratch.add(32);
        let offsets = d.add(2048);
        let parents = d.add(3073) as *mut u16;
        let tiles = (g.rows as u64 + 1023) / 1024;
        loop {
            if tid == 0 {
                *scratch.add(8) = add(core::ptr::addr_of_mut!((*stats).tile_cursor), 1)
                    .min(u32::MAX as u64) as u32;
            }
            thread::sync_threads();
            let tile = *scratch.add(8) as u64;
            if tile >= tiles {
                // No root work: acquire one tail chunk and a private descriptor.
                if tid == 0 {
                    *scratch.add(9) = u32::MAX;
                    let mut s = 0;
                    while s < pool_capacity {
                        let slot = pool.add(s as usize * SLOT_WORDS);
                        if lock(slot, 2) {
                            if *slot.add(1) < *slot.add(2) {
                                *slot.add(2) -= 1;
                                *scratch.add(10) = *slot.add(2) * 1024;
                                *scratch.add(9) = s;
                                add(core::ptr::addr_of_mut!((*stats).steals), 1);
                                break;
                            }
                            unlock(slot, 2);
                        }
                        s += 1;
                    }
                    *scratch.add(11) =
                        (DeviceAtomicU64::from_ptr(core::ptr::addr_of_mut!((*stats).remaining))
                            .load(AtomicOrdering::Acquire)
                            == 0) as u32;
                }
                thread::sync_threads();
                let s = *scratch.add(9);
                if s != u32::MAX {
                    let slot = pool.add(s as usize * SLOT_WORDS);
                    let mut i = tid;
                    while i < DESCRIPTOR_WORDS {
                        *d.add(i) = *slot.add(3 + i);
                        i += 256;
                    }
                    thread::sync_threads();
                    if tid == 0 {
                        unlock(slot, 2);
                    }
                    thread::sync_threads();
                    consume(
                        g,
                        d,
                        *scratch.add(10),
                        stats,
                        output,
                        output_capacity,
                        mode,
                        scratch,
                    );
                } else if *scratch.add(11) != 0 {
                    break;
                }
                continue;
            }
            let mut sum = 0u64;
            let mut stripe = 0;
            while stripe < 4 {
                let p = tid + stripe * 256;
                let row = tile * 1024 + p as u64;
                let mut begin = 0;
                let mut count = 0;
                let mut parent = p as u16;
                if row < g.rows as u64 {
                    let (aa, az) = range(g, *g.keys.add(row as usize));
                    let (ba, bz) = range(g, *g.neighbors.add(row as usize));
                    if mode == 0 && az - aa < bz - ba {
                        begin = aa;
                        count = az - aa;
                        parent |= 32768;
                    } else {
                        begin = ba;
                        count = bz - ba;
                    }
                }
                *d.add(p) = row as u32;
                *d.add(1024 + p) = begin;
                *parents.add(p) = parent;
                // Four 256-item scans; detect sum overflow before consuming.
                // Each per-stripe count is bounded by total edge cardinality:
                // disjoint edge rows can repeat a range, so use a conservative
                // per-item limit and record rejection rather than wrap silently.
                if count > u32::MAX / 256 {
                    add(core::ptr::addr_of_mut!((*stats).overflow), 1);
                    count = 0;
                }
                let (prefix, total) = gpu_tiles::scan(count, scratch);
                *offsets.add(p) = sum as u32 + prefix;
                sum += total as u64;
                stripe += 1;
            }
            if tid == 0 {
                *offsets.add(1024) = sum as u32;
                DeviceAtomicU64::from_ptr(core::ptr::addr_of_mut!((*stats).max_expansion))
                    .fetch_max(sum, AtomicOrdering::Relaxed);
                if sum > u32::MAX as u64 {
                    add(core::ptr::addr_of_mut!((*stats).overflow), 1);
                }
                *scratch.add(9) = u32::MAX;
                if sum <= u32::MAX as u64 && k != 0 && sum > k as u64 * 1024 {
                    let mut s = 0;
                    while s < pool_capacity {
                        if lock(pool.add(s as usize * SLOT_WORDS), 0) {
                            *scratch.add(9) = s;
                            break;
                        }
                        s += 1;
                    }
                    if *scratch.add(9) == u32::MAX {
                        add(core::ptr::addr_of_mut!((*stats).pool_full), 1);
                    }
                }
            }
            thread::sync_threads();
            let slot_id = *scratch.add(9);
            let chunks = (sum + 1023) / 1024;
            if slot_id != u32::MAX {
                let slot = pool.add(slot_id as usize * SLOT_WORDS);
                let mut i = tid;
                while i < DESCRIPTOR_WORDS {
                    *slot.add(3 + i) = *d.add(i);
                    i += 256;
                }
                thread::sync_threads();
                if tid == 0 {
                    *slot.add(1) = 0;
                    *slot.add(2) = chunks as u32;
                    add(core::ptr::addr_of_mut!((*stats).published), 1);
                    unlock(slot, 2);
                }
                thread::sync_threads();
                loop {
                    if tid == 0 {
                        *scratch.add(10) = u32::MAX;
                        *scratch.add(12) = 0;
                        if lock(slot, 2) {
                            if *slot.add(1) < *slot.add(2) {
                                *scratch.add(10) = *slot.add(1) * 1024;
                                *slot.add(1) += 1;
                                unlock(slot, 2);
                            } else {
                                *scratch.add(12) = 1;
                                unlock(slot, 0);
                            }
                        }
                    }
                    thread::sync_threads();
                    if *scratch.add(12) != 0 {
                        break;
                    }
                    if *scratch.add(10) != u32::MAX {
                        consume(
                            g,
                            d,
                            *scratch.add(10),
                            stats,
                            output,
                            output_capacity,
                            mode,
                            scratch,
                        );
                    }
                }
            } else if sum <= u32::MAX as u64 {
                let mut chunk = 0;
                while chunk < chunks {
                    consume(
                        g,
                        d,
                        (chunk * 1024) as u32,
                        stats,
                        output,
                        output_capacity,
                        mode,
                        scratch,
                    );
                    chunk += 1;
                }
            }
            if tid == 0 {
                DeviceAtomicU64::from_ptr(core::ptr::addr_of_mut!((*stats).remaining))
                    .fetch_sub(1, AtomicOrdering::Release);
            }
            thread::sync_threads();
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1, block=(256,1,1), dynamic_shared_range=(32,32))]
    pub unsafe fn filter_wedges(g: GraphView, wedges: *const Triple, count: u64, answer: *mut u64) {
        let i = thread::index_1d().get() as u64;
        let mut hit = false;
        if i < count {
            let t = *wedges.add(i as usize);
            let (a, z) = range(g, t.a);
            hit = contains(g.neighbors, a, z, t.c);
        }
        let scratch = DynamicSharedArray::<u8>::get_raw() as *mut u32;
        let (_, total) = gpu_tiles::scan(hit as u32, scratch);
        if thread::threadIdx_x() == 0 {
            add(answer, total as u64);
        }
    }
}
