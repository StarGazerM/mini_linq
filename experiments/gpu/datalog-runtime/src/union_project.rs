//! Support-preserving binary composition for a checked set terminal. A raw
//! expansion and a grouped union read the same two immutable snapshots.
//! Each grouped witness is looked up in every run; compact word fragments
//! are shared by its driver edges. Output groups can intern identical source
//! sets by comparing all membership words, or OR fragments in bounded stripes.
use cuda_device::{
    atomic::{AtomicOrdering::Relaxed, DeviceAtomicU32, DeviceAtomicU64},
    device, thread, warp,
};
use crate::radix_run::device::{decode_key, encode_prefix, interval, run};
use crate::{
    Buffer, CandidateArena, CandidateBatch, Packing, RadixLayout, RadixLayoutView, ScalarTuple, Word,
};

/// The two orientations are independent: physical driver column of the
/// witness, and canonical head column of the driver endpoint.
#[derive(Clone, Copy)]
#[repr(C)]
struct Projection {
    witness: u32,
    group_first: bool,
    group_signed: bool,
    member_signed: bool,
    low_width: u32,
    output_mask: u64,
    sign_bias: u64,
}
// SAFETY: plain fixed-width scalars, no references or ownership.
unsafe impl cuda_core::DeviceCopy for Projection {}

#[device]
fn up_pack(p: Projection, group: u32, member: u32) -> u64 {
    let (hi, lo) = if p.group_first { (group, member) } else { (member, group) };
    (((u64::from(hi) << p.low_width) | (u64::from(lo) & ((1u64 << p.low_width) - 1))) & p.output_mask) ^ p.sign_bias
}

// Zigzag is a bijection, not a hash. It keeps small signed coordinates dense.
#[device]
fn up_id(x: i64, signed: bool) -> u32 {
    if signed { ((x as u32) << 1) ^ ((x as i32 >> 31) as u32) } else { x as u32 }
}
#[device]
fn up_value(x: u32, signed: bool) -> u32 {
    if signed { (x >> 1) ^ 0u32.wrapping_sub(x & 1) } else { x }
}

const MASK: u32 = u32::MAX;
const FRAGMENT_WARPS: usize = 16;

#[device]
fn up_thread() -> (usize, usize) {
    (
        (thread::blockIdx_x() * 256 + thread::threadIdx_x()) as usize,
        (thread::gridDim_x() * 256) as usize,
    )
}

#[device]
unsafe fn up_warp_max(v: u32, dst: *mut u32) {
    let mut value = v;
    let mut step = 16;
    while step != 0 {
        value = value.max(warp::shuffle_xor_u64(value as u64, step) as u32);
        step /= 2;
    }
    if warp::lane_id() == 0 {
        unsafe { DeviceAtomicU32::from_ptr(dst) }.fetch_max(value, Relaxed);
    }
}

/// One reservation for all active lanes. All lanes of the warp participate.
#[device]
unsafe fn up_reserve(active: bool, total: *mut u32) -> u32 {
    let mask = warp::ballot_sync(MASK, active);
    let mut first = 0;
    if warp::lane_id() == 0 && mask != 0 {
        first = unsafe { DeviceAtomicU32::from_ptr(total) }.fetch_add(mask.count_ones(), Relaxed);
    }
    warp::shuffle_u64_sync(MASK, first as u64, 0) as u32 + (mask & warp::lanemask_lt()).count_ones()
}

/// Inclusive warp prefix and total, for enumerating the children of 32 words.
#[device]
fn up_prefix32(value: u32) -> (u32, u32) {
    let lane = warp::lane_id();
    let mut sum = value;
    let mut step = 1;
    while step < 32 {
        let prior = warp::shuffle_u64_sync(MASK, sum as u64, lane.wrapping_sub(step) & 31) as u32;
        if lane >= step { sum += prior; }
        step *= 2;
    }
    (sum - value, warp::shuffle_u64_sync(MASK, sum as u64, 31) as u32)
}

/// Consume compact `(destination group, zero-based source)` pairs. All four
/// representative workers share a source but own disjoint fragment windows.
#[device]
unsafe fn up_take_jobs(
    jobs: *const u64, count: *const u32, offset: *const u64, length: *const u32,
    frag: *const u64, words: usize, bits: *mut u32, warps_per_ref: usize,
) {
    unsafe {
        let warps_per_block = thread::blockDim_x() as usize / 32;
        let warp = thread::blockIdx_x() as usize * warps_per_block + thread::threadIdx_x() as usize / 32;
        let stride = thread::gridDim_x() as usize * warps_per_block;
        let end = *count as usize * warps_per_ref;
        let mut worker = warp;
        while worker < end {
            let job = *jobs.add(worker / warps_per_ref);
            let source = job as u32 as usize;
            let list = frag.add(*offset.add(source) as usize);
            let n = *length.add(source) as usize;
            let dst = bits.add((job >> 32) as usize * words);
            let mut j = warp::lane_id() as usize + (worker % warps_per_ref) * 32;
            while j < n {
                let v = *list.add(j);
                DeviceAtomicU32::from_ptr(dst.add((v >> 32) as usize)).fetch_or(v as u32, Relaxed);
                j += warps_per_ref * 32;
            }
            worker += stride;
        }
    }
}

#[cuda_device::cuda_module]
mod union_project_kernels {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract};

    /// Bound the dense lookup directory by the actual first-column code range.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn up_limits<W: Copy + Word>(v: RadixLayoutView<W, u64>, limits: *mut u64) {
        unsafe {
            if up_thread().0 == 0 {
                let (mut low, mut high) = (u32::MAX, 0u32);
                let mut r = 0;
                while r < v.keys.run_count {
                    let len = run(v.keys, r).keys_len;
                    if len != 0 {
                        let bias = (*v.fields).signed << 31;
                        low = low.min((decode_key(v, r, 0u64, 0) as u32) ^ bias);
                        high = high.max((decode_key(v, r, len - 1, 0) as u32) ^ bias);
                    }
                    r += 1;
                }
                *limits = (u64::from(high) << 32) | u64::from(low);
            }
        }
    }

    /// Record sorted run boundaries by raw source symbol, twice per distinct
    /// prefix and not per candidate. Each half-word has exactly one writer.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn up_directory<W: Copy + Word>(
        v: RadixLayoutView<W, u64>,
        low: u32,
        width: usize,
        dir: *mut u32,
    ) {
        unsafe {
            let (id, stride) = up_thread();
            let mut r = 0;
            while r < v.keys.run_count {
                let n = run(v.keys, r).keys_len as usize;
                let base = dir.add(2 * r as usize * width);
                let mut i = id;
                while i < n {
                    let f = decode_key(v, r, i as u64, 0) as u32;
                    let span = base.add(2 * f.wrapping_sub(low) as usize);
                    if i == 0 || decode_key(v, r, (i - 1) as u64, 0) as u32 != f {
                        *span = i as u32;
                    }
                    if i + 1 == n || decode_key(v, r, (i + 1) as u64, 0) as u32 != f {
                        *span.add(1) = (i + 1) as u32;
                    }
                    i += stride;
                }
                r += 1;
            }
        }
    }

    /// Give each high-fanout edge a warp, including when the delta has too
    /// few rows for one thread per edge to fill the device.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn up_scatter<WD: Copy + Word, WR: Copy + Word>(
        delta: RadixLayoutView<WD, u64>,
        rhs: RadixLayoutView<WR, u64>,
        p: Projection,
        count: *mut u64,
        out: *mut u64,
        capacity: u64,
    ) {
        unsafe {
            let (id, stride) = up_thread();
            let lane = warp::lane_id();
            let mut dr = 0;
            while dr < delta.keys.run_count {
                let n = run(delta.keys, dr).keys_len as usize;
                let mut i = id / 32;
                while i < n {
                    let f = [decode_key(delta, dr, i as u64, p.witness)];
                    let g = decode_key(delta, dr, i as u64, 1 - p.witness) as u32;
                    let (lo, hi, ok) = encode_prefix(rhs, f.as_ptr(), 1);
                    let mut r = 0;
                    while ok && r < rhs.keys.run_count {
                        let (a, b) = interval(run(rhs.keys, r), lo, hi);
                        let len = b - a;
                        if len != 0 {
                            let mut dst = 0;
                            if lane == 0 {
                                dst = DeviceAtomicU64::from_ptr(count).fetch_add(len, Relaxed);
                            }
                            let base = warp::shuffle_u64_sync(MASK, dst, 0);
                            let mut j = u64::from(lane);
                            while j < len && base + j < capacity {
                                let c = decode_key(rhs, r, a + j, 1) as u32;
                                *out.add((base + j) as usize) = up_pack(p, g, c);
                                j += 32;
                            }
                        }
                        r += 1;
                    }
                    i += stride / 32;
                }
                dr += 1;
            }
        }
    }

    /// Direct bag expansion, also used for a count-only systematic sample.
    /// Lanes probe adjacent roots independently, share one reservation, and
    /// cooperate on the long child spans so their writes remain coalesced.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn up_direct<WD: Copy + Word, WR: Copy + Word>(
        delta: RadixLayoutView<WD, u64>,
        rhs: RadixLayoutView<WR, u64>,
        p: Projection,
        n: usize,
        draws: usize,
        count: *mut u64,
        out: *mut u64,
        capacity: u64,
        dir: *const u64,
        low: u32,
        width: usize,
    ) {
        unsafe {
            let (id, stride) = up_thread();
            let lane = warp::lane_id();
            let mut window = id - lane as usize;
            while window < draws {
                let i = window + lane as usize;
                let mut fits = false;
                let mut g = 0u32;
                let mut prefix = [0i64];
                if i < draws {
                    let mut pos = if draws == n {
                        i
                    } else {
                        (i * n + n / 2) / draws
                    };
                    let mut r = 0;
                    while pos >= run(delta.keys, r).keys_len as usize {
                        pos -= run(delta.keys, r).keys_len as usize;
                        r += 1;
                    }
                    prefix[0] = decode_key(delta, r, pos as u64, p.witness);
                    g = decode_key(delta, r, pos as u64, 1 - p.witness) as u32;
                    fits = true;
                }
                let (lo, hi, enc) = encode_prefix(rhs, prefix.as_ptr(), 1);
                let mut r = 0;
                while r < rhs.keys.run_count {
                    let (a, b) = if !dir.is_null() {
                        let index = (prefix[0] as u32).wrapping_sub(low) as usize;
                        let span = if fits && index < width {
                            *dir.add(r as usize * width + index)
                        } else {
                            0
                        };
                        (u64::from(span as u32), span >> 32)
                    } else if fits && enc {
                        interval(run(rhs.keys, r), lo, hi)
                    } else {
                        (0, 0)
                    };
                    if warp::ballot_sync(MASK, a != b) == 0 {
                        r += 1;
                        continue;
                    }
                    let len = b - a;
                    let mut sum = len;
                    let mut step = 1;
                    while step < 32 {
                        let prev = warp::shuffle_u64_sync(MASK, sum, lane.wrapping_sub(step) & 31);
                        if lane >= step {
                            sum += prev;
                        }
                        step *= 2;
                    }
                    let total = warp::shuffle_u64_sync(MASK, sum, 31);
                    let mut start = 0;
                    if lane == 0 && total != 0 {
                        start = DeviceAtomicU64::from_ptr(count).fetch_add(total, Relaxed);
                    }
                    let base = warp::shuffle_u64_sync(MASK, start, 0) + sum - len;
                    if capacity != 0 {
                        if len <= 8 {
                            let mut j = 0;
                            while j < len && base + j < capacity {
                                let c = decode_key(rhs, r, a + j, 1) as u32;
                                *out.add((base + j) as usize) = up_pack(p, g, c);
                                j += 1;
                            }
                        }
                        let mut todo = warp::ballot_sync(MASK, len > 8 && base < capacity);
                        while todo != 0 {
                            let owner = todo.trailing_zeros();
                            let first = warp::shuffle_u64_sync(MASK, a, owner);
                            let begin = warp::shuffle_u64_sync(MASK, base, owner);
                            let end =
                                warp::shuffle_u64_sync(MASK, len, owner).min(capacity - begin);
                            let head = warp::shuffle_u64_sync(MASK, u64::from(g), owner);
                            let mut j = u64::from(lane);
                            while j < end {
                                let c = decode_key(rhs, r, first + j, 1) as u32;
                                *out.add((begin + j) as usize) = up_pack(p, head as u32, c);
                                j += 32;
                            }
                            todo &= todo - 1;
                        }
                    }
                    r += 1;
                }
                window += stride;
            }
        }
    }

    /// Concatenate delta runs without assuming one run or a physical u64 key.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn up_delta<W: Copy + Word>(
        v: RadixLayoutView<W, u64>,
        p: Projection,
        edge: *mut u64,
        maxima: *mut u32,
    ) {
        unsafe {
            let (id, stride) = up_thread();
            let (mut first, mut mg, mut mf) = (0usize, 0u32, 0u32);
            let mut r = 0;
            while r < v.keys.run_count {
                let len = run(v.keys, r).keys_len as usize;
                let mut i = id;
                while i < len {
                    let f = up_id(decode_key(v, r, i as u64, p.witness), (*v.fields.add(p.witness as usize)).signed != 0);
                    let g = up_id(decode_key(v, r, i as u64, 1 - p.witness), p.group_signed);
                    *edge.add(first + i) = (g as u64) << 32 | f as u64;
                    mg = mg.max(g);
                    mf = mf.max(f);
                    i += stride;
                }
                first += len;
                r += 1;
            }
            up_warp_max(mf, maxima);
            up_warp_max(mg, maxima.add(1));
        }
    }

    /// Compact the distinct witness symbols that need a lookup.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn up_sources(edge: *const u64, n: usize, map: *mut u32, list: *mut u32, count: *mut u32) {
        unsafe {
            let (id, stride) = up_thread();
            let lane = warp::lane_id() as usize;
            let mut w = id - lane;
            while w < n {
                let i = w + lane;
                let mut fresh = false;
                let mut f = 0;
                if i < n {
                    f = *edge.add(i) as u32;
                    fresh =
                        DeviceAtomicU32::from_ptr(map.add(f as usize)).fetch_or(1, Relaxed) == 0;
                }
                let pos = up_reserve(fresh, count);
                if fresh {
                    *list.add(pos as usize) = f;
                }
                w += stride;
            }
        }
    }

    /// Locate every queried prefix in every run, reserving child storage per witness.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn up_ranges<W: Copy + Word>(
        v: RadixLayoutView<W, u64>,
        sources: *const u32,
        n: usize,
        map: *mut u32,
        spans: *mut u64,
        offset: *mut u64,
        total: *mut u64,
        maxword: *mut u32,
    ) {
        unsafe {
            let (mut i, stride) = up_thread();
            let mut peak = 0;
            while i < n {
                let f = *sources.add(i);
                let signed = (*v.fields).signed != 0;
                let prefix = [if signed { i64::from(up_value(f, true) as i32) } else { i64::from(f) }];
                let (lo, hi, ok) = encode_prefix(v, prefix.as_ptr(), 1);
                let mut sum = 0;
                let mut r = 0;
                while r < v.keys.run_count {
                    let (a, b) = if ok {
                        interval(run(v.keys, r), lo, hi)
                    } else {
                        (0, 0)
                    };
                    let p = spans.add(2 * (i * v.keys.run_count as usize + r as usize));
                    *p = a;
                    *p.add(1) = b;
                    if b > a {
                        let signed = (*v.fields.add(1)).signed != 0;
                        let last = up_id(decode_key(v, r, b - 1, 1), signed);
                        // Zigzag's maximum over a sorted interval is at an endpoint.
                        let high = if signed { last.max(up_id(decode_key(v, r, a, 1), true)) } else { last };
                        peak = peak.max(high >> 5);
                        sum += b - a;
                    }
                    r += 1;
                }
                *map.add(f as usize) = if sum == 0 { 0 } else { i as u32 + 1 };
                *offset.add(i) = DeviceAtomicU64::from_ptr(total).fetch_add(sum, Relaxed);
                i += stride;
            }
            up_warp_max(peak, maxword);
        }
    }

    /// Rewrite source IDs and mark the head groups with a nonempty range.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn up_live_edges(edge: *mut u64, n: usize, fmap: *const u32, gmap: *mut u32) {
        unsafe {
            let (mut i, stride) = up_thread();
            while i < n {
                let e = *edge.add(i);
                let f = *fmap.add(e as u32 as usize);
                let g = (e >> 32) as u32;
                *edge.add(i) = (u64::from(g) << 32) | u64::from(f);
                if f != 0 {
                    DeviceAtomicU32::from_ptr(gmap.add(g as usize)).store(1, Relaxed);
                }
                i += stride;
            }
        }
    }

    /// Encode windows of sorted children into sparse `(word, bits)` fragments.
    #[kernel]
    #[launch_bounds(128)]
    #[launch_contract(domain = 1, block = (128, 1, 1))]
    pub fn up_fragments<W: Copy + Word>(
        v: RadixLayoutView<W, u64>,
        spans: *const u64,
        n: usize,
        offset: *const u64,
        length: *mut u32,
        frag: *mut u64,
        wmap: *mut u32,
    ) {
        unsafe {
            // Four 128-thread CTAs per span; warp j owns windows j, j+16, ...
            let warp = thread::blockIdx_x() as usize * 4 + thread::threadIdx_x() as usize / 32;
            let lane = warp::lane_id();
            let mut span = warp / FRAGMENT_WARPS;
            while span < n {
                let parent = span / v.keys.run_count as usize;
                let r = (span % v.keys.run_count as usize) as u32;
                let mut a = *spans.add(span * 2) + ((warp % FRAGMENT_WARPS) * 32) as u64;
                let b = *spans.add(span * 2 + 1);
                while a < b {
                    let valid = a + u64::from(lane) < b;
                    let c = if valid {
                        up_id(decode_key(v, r, a + u64::from(lane), 1), (*v.fields.add(1)).signed != 0)
                    } else {
                        0
                    };
                    let word = c >> 5;
                    let mut mask = 1u32 << (c & 31);
                    // Segmented inclusive OR; only the last lane of a raw word
                    // in this window appends. Other windows/runs need not be
                    // stitched: their masks are combined in the final union.
                    let mut step = 1;
                    while step < 32 {
                        let from = lane.wrapping_sub(step) & 31;
                        let prev = warp::shuffle_u64_sync(
                            MASK,
                            (u64::from(word) << 32) | u64::from(mask),
                            from,
                        );
                        if lane >= step && prev >> 32 == u64::from(word) {
                            mask |= prev as u32;
                        }
                        step *= 2;
                    }
                    let next = warp::shuffle_u64_sync(MASK, word as u64, (lane + 1) & 31) as u32;
                    let end = valid && (lane == 31 || a + u64::from(lane) + 1 == b || next != word);
                    let at = up_reserve(end, length.add(parent));
                    if end {
                        *frag.add((*offset.add(parent) + u64::from(at)) as usize) =
                            (u64::from(word) << 32) | u64::from(mask);
                        DeviceAtomicU32::from_ptr(wmap.add(word as usize)).store(1, Relaxed);
                    }
                    a += (32 * FRAGMENT_WARPS) as u64;
                }
                span += thread::gridDim_x() as usize * 4 / FRAGMENT_WARPS;
            }
        }
    }

    /// Mark → dense positive ID. Zero remains the absent sentinel in `map`.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn up_rank(map: *mut u32, n: usize, decode: *mut u32, count: *mut u32) {
        unsafe {
            let (id, stride) = up_thread();
            let lane = warp::lane_id() as usize;
            let mut w = id - lane;
            while w < n {
                let i = w + lane;
                let active = i < n && *map.add(i) != 0;
                let pos = up_reserve(active, count);
                if active {
                    *map.add(i) = pos + 1;
                    *decode.add(pos as usize) = i as u32;
                }
                w += stride;
            }
        }
    }

    /// Replace raw word numbers with the compact per-invocation word IDs.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn up_remap_fragments(
        offset: *const u64,
        length: *const u32,
        n: usize,
        frag: *mut u64,
        wmap: *const u32,
    ) {
        unsafe {
            let (id, stride) = up_thread();
            let mut f = id / 32;
            while f < n {
                let p = frag.add(*offset.add(f) as usize);
                let mut j = warp::lane_id();
                let end = *length.add(f);
                while j < end {
                    let v = *p.add(j as usize);
                    *p.add(j as usize) =
                        (u64::from(*wmap.add((v >> 32) as usize) - 1) << 32) | u64::from(v as u32);
                    j += 32;
                }
                f += stride / 32;
            }
        }
    }

    /// One lane per edge, one ballot reservation for the live stripe references.
    #[kernel] #[launch_bounds(128)] #[launch_contract(domain=1,block=(128,1,1))]
    pub fn up_edge_jobs(edge: *const u64, n: usize, gmap: *const u32, first: usize,
                        groups: usize, jobs: *mut u64, count: *mut u32) {
        unsafe {
            let i = thread::blockIdx_x() as usize * 128 + thread::threadIdx_x() as usize;
            let e = if i < n { *edge.add(i) } else { 0 };
            let source = e as u32;
            let g = if source != 0 { (*gmap.add((e >> 32) as usize) - 1) as usize } else { 0 };
            let live = source != 0 && g >= first && g - first < groups;
            let pos = up_reserve(live, count);
            if live {
                *jobs.add(pos as usize) = (((g - first) as u64) << 32) | u64::from(source - 1);
            }
        }
    }

    /// One lane per membership word: enumerate its set bits into a single
    /// warp-sized reservation, preserving representative (not catalog) IDs.
    #[kernel] #[launch_bounds(128)] #[launch_contract(domain=1,block=(128,1,1))]
    pub fn up_rep_jobs(members: *const u32, catalog: *const u32, reps: usize, width: usize,
                       jobs: *mut u64, count: *mut u32) {
        unsafe {
            let i = thread::blockIdx_x() as usize * 128 + thread::threadIdx_x() as usize;
            let rep = i / width;
            let col = i % width;
            let mut v = if rep < reps { *members.add(*catalog.add(rep) as usize * width + col) } else { 0 };
            let (prefix, total) = up_prefix32(v.count_ones());
            if total == 0 { return; }
            let base = if warp::lane_id() == 0 {
                DeviceAtomicU32::from_ptr(count).fetch_add(total, Relaxed)
            } else { 0 };
            let mut at = warp::shuffle_u64_sync(MASK, u64::from(base), 0) as usize + prefix as usize;
            while v != 0 {
                let source = col * 32 + v.trailing_zeros() as usize;
                *jobs.add(at) = ((rep as u64) << 32) | source as u64;
                at += 1;
                v &= v - 1;
            }
        }
    }

    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn up_take_edge_jobs(jobs: *const u64, count: *const u32, offset: *const u64,
                             length: *const u32, frag: *const u64, words: usize, bits: *mut u32) {
        unsafe { up_take_jobs(jobs, count, offset, length, frag, words, bits, 1) }
    }

    #[kernel] #[launch_bounds(128)] #[launch_contract(domain=1,block=(128,1,1))]
    pub fn up_take_rep_jobs(jobs: *const u64, count: *const u32, offset: *const u64,
                            length: *const u32, frag: *const u64, words: usize, bits: *mut u32) {
        unsafe { up_take_jobs(jobs, count, offset, length, frag, words, bits, 4) }
    }

    /// Atomic bitwise union of source fragments into one stripe of groups.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn up_union(
        edge: *const u64,
        n: usize,
        gmap: *const u32,
        offset: *const u64,
        length: *const u32,
        frag: *const u64,
        first: usize,
        groups: usize,
        words: usize,
        bits: *mut u32,
    ) {
        unsafe {
            let (id, stride) = up_thread();
            let mut i = id / 32;
            while i < n {
                let e = *edge.add(i);
                let source = e as u32;
                if source != 0 {
                    let g = (*gmap.add((e >> 32) as usize) - 1) as usize;
                    if g >= first && g - first < groups {
                        let f = (source - 1) as usize;
                        let p = frag.add(*offset.add(f) as usize);
                        let end = *length.add(f);
                        let dst = bits.add((g - first) * words);
                        let mut j = warp::lane_id();
                        while j < end {
                            let v = *p.add(j as usize);
                            DeviceAtomicU32::from_ptr(dst.add((v >> 32) as usize))
                                .fetch_or(v as u32, Relaxed);
                            j += 32;
                        }
                    }
                }
                i += stride / 32;
            }
        }
    }

    // A row is the exact set of live source IDs of one compact head group.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn up_members(edge: *const u64, n: usize, gmap: *const u32, width: usize, members: *mut u32) {
        unsafe {
            let (mut i, step) = up_thread();
            while i < n {
                let e = *edge.add(i);
                if e as u32 != 0 {
                    let f = (e as u32 - 1) as usize;
                    let g = (*gmap.add((e >> 32) as usize) - 1) as usize;
                    DeviceAtomicU32::from_ptr(members.add(g * width + f / 32)).fetch_or(1 << (f % 32), Relaxed);
                }
                i += step;
            }
        }
    }

    /// Hashes choose a probe sequence only. Every reuse compares all membership
    /// words; only the thread that wins a vacant slot allocates a union ID.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn up_intern(members: *const u32, groups: usize, width: usize, table: *mut u32, mask: usize,
                     owner: *mut u32, rank: *mut u32, catalog: *mut u32, count: *mut u32) {
        unsafe {
            let (id, stride) = up_thread();
            let lane = warp::lane_id();
            let mut g = id / 32;
            while g < groups {
                let row = members.add(g * width);
                let mut h = 0x811c9dc5u32;
                let mut k = lane as usize;
                while k < width {
                    h = (h ^ *row.add(k)).wrapping_mul(0x01000193);
                    k += 32;
                }
                // Mix lane index before reducing so permuting words across lanes
                // does not produce identical probe sequences.
                h = h.wrapping_mul(0x85ebca6b).rotate_left(lane);
                let mut step = 16;
                while step != 0 {
                    h ^= warp::shuffle_xor_u64(h as u64, step) as u32;
                    step /= 2;
                }
                let mut slot = h as usize & mask;
                loop {
                    let mut other = 0;
                    if lane == 0 {
                        let cell = DeviceAtomicU32::from_ptr(table.add(slot));
                        other = cell.load(Relaxed);
                        if other == 0 {
                            if let Err(old) = cell.compare_exchange(0, g as u32 + 1, Relaxed, Relaxed) {
                                other = old;
                            }
                        }
                    }
                    other = warp::shuffle_u64_sync(MASK, other as u64, 0) as u32;
                    if other == 0 {
                        if lane == 0 {
                            let r = DeviceAtomicU32::from_ptr(count).fetch_add(1, Relaxed);
                            *rank.add(g) = r;
                            *catalog.add(r as usize) = g as u32;
                            *owner.add(g) = g as u32;
                        }
                        break;
                    }
                    let rep = (other - 1) as usize;
                    let proposed = members.add(rep * width);
                    let mut equal = true;
                    let mut k = lane as usize;
                    while k < width && equal {
                        equal = *row.add(k) == *proposed.add(k);
                        k += 32;
                    }
                    if warp::ballot_sync(MASK, !equal) == 0 {
                        if lane == 0 { *owner.add(g) = rep as u32; }
                        break;
                    }
                    slot = (slot + 1) & mask;
                }
                g += stride / 32;
            }
        }
    }

    /// Each warp handles 32 possible sources of one representative group.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn up_union_reps(members: *const u32, catalog: *const u32, reps: usize, width: usize,
                         offset: *const u64, length: *const u32, frag: *const u64, words: usize, bits: *mut u32) {
        unsafe {
            let (id, stride) = up_thread();
            let mut task = id / 32;
            while task < reps * width {
                let r = task / width;
                let k = task % width;
                let g = *catalog.add(r) as usize;
                let mut present = *members.add(g * width + k);
                while present != 0 {
                    let f = k * 32 + present.trailing_zeros() as usize;
                    let list = frag.add(*offset.add(f) as usize);
                    let end = *length.add(f);
                    let mut j = warp::lane_id();
                    while j < end {
                        let v = *list.add(j as usize);
                        DeviceAtomicU32::from_ptr(bits.add(r * words + (v >> 32) as usize)).fetch_or(v as u32, Relaxed);
                        j += 32;
                    }
                    present &= present - 1;
                }
                task += stride / 32;
            }
        }
    }

    /// Two passes over only the representative rows. The first stores one
    /// local offset per 32-word tile; the second materializes compact children.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn up_rep_children(bits: *const u32, reps: usize, words: usize, tiles: usize, counts: *mut u32,
                           tile_offset: *mut u32, bases: *const u64, wdecode: *const u32, children: *mut u32) {
        unsafe {
            let (id, stride) = up_thread();
            let lane = warp::lane_id();
            let mut i = id / 32;
            while i < reps * tiles {
                let r = i / tiles;
                let word = i % tiles * 32 + lane as usize;
                let mut v = if word < words { *bits.add(r * words + word) } else { 0 };
                if warp::ballot_sync(MASK, v != 0) != 0 {
                    let (prefix, total) = up_prefix32(v.count_ones());
                    if children.is_null() {
                        if lane == 0 {
                            *tile_offset.add(i) = DeviceAtomicU32::from_ptr(counts.add(r)).fetch_add(total, Relaxed);
                        }
                    } else if v != 0 {
                        let mut dest = *bases.add(r) as usize + *tile_offset.add(i) as usize + prefix as usize;
                        let page = *wdecode.add(word) << 5;
                        while v != 0 {
                            *children.add(dest) = page | v.trailing_zeros();
                            dest += 1;
                            v &= v - 1;
                        }
                    }
                }
                i += stride / 32;
            }
        }
    }

    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn up_rep_layout(groups: usize, owner: *const u32, rank: *const u32, counts: *const u32,
                         bases: *mut u64, out_off: *mut u64, totals: *mut u64) {
        unsafe {
            let (mut g, stride) = up_thread();
            while g < groups {
                let o = *owner.add(g) as usize;
                let r = *rank.add(o) as usize;
                let n = u64::from(*counts.add(r));
                *out_off.add(g) = DeviceAtomicU64::from_ptr(totals).fetch_add(n, Relaxed);
                if g == o { *bases.add(r) = DeviceAtomicU64::from_ptr(totals.add(1)).fetch_add(n, Relaxed); }
                g += stride;
            }
        }
    }

    /// A group's children are already distinct. Replicate coalesced ranges
    /// rather than scanning another mostly-empty bitmap per output group.
    #[kernel] #[launch_bounds(256)] #[launch_contract(domain=1,block=(256,1,1))]
    pub fn up_copy_groups(groups: usize, owner: *const u32, rank: *const u32, counts: *const u32,
                          bases: *const u64, out_off: *const u64, children: *const u32,
                          gdecode: *const u32, p: Projection, out: *mut u64) {
        unsafe {
            let mut g = thread::blockIdx_x() as usize;
            while g < groups {
                let r = *rank.add(*owner.add(g) as usize) as usize;
                let head = up_value(*gdecode.add(g), p.group_signed);
                let src = children.add(*bases.add(r) as usize);
                let dst = out.add(*out_off.add(g) as usize);
                let mut i = thread::threadIdx_x() as usize;
                let len = *counts.add(r) as usize;
                while i < len {
                    let c = up_value(*src.add(i), p.member_signed);
                    *dst.add(i) = up_pack(p, head, c);
                    i += 256;
                }
                g += thread::gridDim_x() as usize;
            }
        }
    }

    /// Decode each set bit to a canonical candidate, respecting the arena capacity.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn up_publish(
        bits: *const u32,
        n: usize,
        words: usize,
        gdecode: *const u32,
        wdecode: *const u32,
        p: Projection,
        count: *mut u64,
        out: *mut u64,
        capacity: u64,
    ) {
        unsafe {
            let (id, stride) = up_thread();
            let lane = warp::lane_id();
            let mut w = id - lane as usize;
            while w < n {
                let i = w + lane as usize;
                let mut v = if i < n { *bits.add(i) } else { 0 };
                let cnt = v.count_ones();
                let mut sum = cnt;
                let mut s = 1;
                while s < 32 {
                    let a =
                        warp::shuffle_u64_sync(MASK, sum as u64, lane.wrapping_sub(s) & 31) as u32;
                    if lane >= s {
                        sum += a;
                    }
                    s *= 2;
                }
                let rows = warp::shuffle_u64_sync(MASK, sum as u64, 31);
                let mut base = 0;
                if lane == 0 && rows != 0 {
                    base = DeviceAtomicU64::from_ptr(count).fetch_add(rows, Relaxed);
                }
                let mut dest = warp::shuffle_u64_sync(MASK, base, 0) + u64::from(sum - cnt);
                if v != 0 && dest < capacity {
                    let g = up_value(*gdecode.add(i / words), p.group_signed);
                    let page = *wdecode.add(i % words);
                    while v != 0 && dest < capacity {
                        let c = up_value((page << 5) | v.trailing_zeros(), p.member_signed);
                        *out.add(dest as usize) = up_pack(p, g, c);
                        v &= v - 1;
                        dest += 1;
                    }
                }
                w += stride;
            }
        }
    }
}

thread_local! {
    static MODULE: std::cell::RefCell<Option<(std::sync::Arc<cuda_core::CudaContext>, union_project_kernels::LoadedModule)>> =
        const { std::cell::RefCell::new(None) };
}

/// Execute a checked UnionProject over two binary key-only layouts. Returns
/// None before publication if the dense-ID scratch would exceed its budget;
/// the ordinary generated enumeration remains valid on the same snapshots.
/// Both layouts carry full rows, with the RHS witness in physical column zero.
#[allow(clippy::too_many_arguments)]
pub fn execute<KD: ScalarTuple, WD: Word + 'static, KR: ScalarTuple, WR: Word + 'static>(
    arena: &mut CandidateArena,
    delta: &RadixLayout<KD, (), WD, u64, ()>,
    rhs: &RadixLayout<KR, (), WR, u64, ()>,
    witness_second: bool,
    group_first: bool,
) -> crate::Result<Option<CandidateBatch>> {
    use cuda_core::LaunchConfig1D;
    let pool = arena.pool().clone();
    let stream = pool.stream();
    delta.validate(KD::COLUMNS)?;
    rhs.validate(KR::COLUMNS)?;
    let ([d0, d1], [via, member]) = (KD::COLUMNS, KR::COLUMNS) else {
        return Err("UnionProject requires two binary layouts".into());
    };
    let (witness, group) = if witness_second { (d1, d0) } else { (d0, d1) };
    if [witness, group, member].iter().any(|col| col.bits() > 32) { return Ok(None); }
    if witness != via {
        return Err("UnionProject coordinate ABI differs from its plan".into());
    }
    let cols = if group_first { [*group, *member] } else { [*member, *group] };
    let pack = Packing::with_columns(&cols)?;
    let sign = |col: crate::Column| if col.is_signed() { 1u64 << (col.bits() - 1) } else { 0 };
    let p = Projection {
        witness: u32::from(witness_second), group_first,
        group_signed: group.is_signed(), member_signed: member.is_signed(),
        low_width: cols[1].bits(), output_mask: u64::MAX >> (64 - pack.bits()),
        sign_bias: (sign(cols[0]) << cols[1].bits()) | sign(cols[1]),
    };
    let owners = || vec![delta.owner(), rhs.owner()];
    let n = delta.len();
    if n > u32::MAX as usize || rhs.len() > u32::MAX as usize { return Ok(None); }
    let crossover = 2_000_000 + 4 * n as u64;
    let append_raw = arena.has_head_target();
    // A set of binary rows contributes at most one pair per input pair.
    // If even that bound cannot select union, leave the native append kernel
    // on its asynchronous cursor without a pilot or per-hole count readback.
    if append_raw && n as u64 * (rhs.len() as u64) < crossover {
        return Ok(None);
    }
    let empty = |arena: &mut CandidateArena| -> crate::Result<_> {
        let lease = arena.reserve(pack.clone(), 0)?;
        arena.record_count(0);
        unsafe { lease.finish(0, owners()).map(Some) }
    };
    if n == 0 || rhs.len() == 0 {
        return empty(arena);
    }
    let _range = cq_gpu_runtime::nvtx::range(c"union_project");
    MODULE.with(|slot| -> crate::Result<_> {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_none_or(|(ctx, _)| !std::sync::Arc::ptr_eq(ctx, pool.context())) {
            *slot = Some((pool.context().clone(), unsafe { union_project_kernels::load(pool.context())? }));
        }
        let m = &slot.as_ref().expect("module initialized above").1;
        let resident = cq_gpu_runtime::launch_grid_for_kernel(
            m.prepare_up_union(LaunchConfig1D::new(1, 256, 0))?.function(), usize::MAX, 256, 0,
        )?;
        let cfg = |threads: usize| LaunchConfig1D::new(threads.div_ceil(256).clamp(1, resident as usize) as u32, 256, 0);
        let u32s = |n: usize| unsafe { Buffer::<u32>::uninitialized(pool.clone(), n.max(1)) };
        let u64s = |n: usize| unsafe { Buffer::<u64>::uninitialized(pool.clone(), n.max(1)) };
        let zero = |n: usize| Buffer::<u32>::zeroed(pool.clone(), n.max(1));
        let read = |b: &Buffer<u32>| -> crate::Result<Vec<u32>> {
            let h = b.to_host()?;
            crate::record_readback();
            Ok(h)
        };
        let read64 = |b: &Buffer<u64>| -> crate::Result<u64> {
            let h = b.to_host()?;
            crate::record_readback();
            Ok(h[0])
        };

        let dv = delta.view_on(&pool)?;
        let rv = rhs.view_on(&pool)?;
        // This is the weighted bag fanout, not up_ranges' sum over distinct
        // sources. Systematic sampling also skips construction of that index
        // on misses. The complete emitter always validates its count.
        let sample=n.min(1024);
        let estimate=if n<256 {0} else {
            let count=Buffer::<u64>::zeroed(pool.clone(),1)?;
            m.up_direct(stream,&m.prepare_up_direct::<WD,WR>(cfg(sample))?,dv,rv,p,n,sample,count.ptr(),std::ptr::null_mut(),0,std::ptr::null(),0,0)?;
            (u128::from(read64(&count)?)*n as u128/sample as u128) as u64
        };
        // Same rule for both orientations/datasets. The fixed cost buys the
        // word index; the linear term avoids it on large, mostly missing
        // deltas. See the paired per-round cost table in join-lab/DESIGN.md.
        if estimate < crossover {
            if append_raw {
                return Ok(None);
            }
            let mut cap=arena.capacity_hint(&pack).max(if n<256{n}else{0});
            // Amortize a run-local prefix directory when the driver is large
            // relative to the lookup and not ordered by the probe prefix.
            let mut directory=None;
            let(mut low, mut width)=(0,0);
            if witness_second && n>=65536 && rhs.len()<=n {
                let extent=u64s(1)?;
                m.up_limits(stream,&m.prepare_up_limits::<WR>(cfg(1))?,rv,extent.ptr())?;
                let value=read64(&extent)?;
                low=value as u32;
                width=(value>>32) as usize+1-low as usize;
                low ^= u32::from(via.is_signed()) << 31;
                let entries=width.saturating_mul(rhs.run_count());
                if entries<=1<<24 {
                    let dir=Buffer::<u64>::zeroed(pool.clone(),entries)?;
                    m.up_directory(stream,&m.prepare_up_directory::<WR>(cfg(rhs.len()))?,rv,low,width,dir.ptr().cast::<u32>())?;
                    directory=Some(dir);
                }
            }
            loop {
                let lease=arena.reserve(pack.clone(),cap)?;
                let count=Buffer::<u64>::zeroed(pool.clone(),1)?;
                if !witness_second && (n<65536 || estimate>32*n as u64) {
                    m.up_scatter(stream,&m.prepare_up_scatter::<WD,WR>(cfg(n*32))?,dv,rv,p,count.ptr(),lease.buffer().ptr(),lease.capacity() as u64)?;
                } else {
                    m.up_direct(stream,&m.prepare_up_direct::<WD,WR>(cfg(n))?,dv,rv,p,n,n,count.ptr(),lease.buffer().ptr(),lease.capacity() as u64,directory.as_ref().map_or(std::ptr::null(),|buf|buf.ptr()),low,width)?;
                }
                let rows=usize::try_from(read64(&count)?)?;
                if rows>u32::MAX as usize{break;}
                if rows<=lease.capacity(){
                    arena.record_count(rows);
                    if crate::trace_kernels() {
                        eprintln!("gpu_union_project\tmethod=raw\tdriver={n}\testimate={estimate}\tcandidates={rows}");
                    }
                    return unsafe{lease.finish(rows,owners()).map(Some)};
                }
                // With skewed input the sample can miss a hot range. Once
                // the exact count is available, don't grow a duplicate bag
                // just to throw it away at admission.
                if rows as u64>=crossover {break;}
                cap=rows;
            }
        }
        let edges = u64s(n)?;
        let maxima = zero(2)?;
        m.up_delta(stream, &m.prepare_up_delta::<WD>(cfg(n))?, dv, p, edges.ptr(), maxima.ptr())?;
        let max = read(&maxima)?;
        let nf = max[0] as usize + 1;
        let ng = max[1] as usize + 1;
        const MAX_WORDS: usize = 1 << 26; // 256 MiB per dense scratch array
        const MAX_JOBS: usize = MAX_WORDS / 2; // one u64 per possible reference
        if nf > MAX_WORDS || ng > MAX_WORDS { return Ok(None); }
        let fmap = zero(nf)?;
        let src = u32s(n.min(nf))?;
        let nsrc = zero(1)?;
        m.up_sources(stream, &m.prepare_up_sources(cfg(n))?, edges.ptr(), n, fmap.ptr(), src.ptr(), nsrc.ptr())?;

        let sources = read(&nsrc)?[0] as usize;
        let segments = sources * rhs.run_count();
        let spans = u64s(2 * segments)?;
        let off = u64s(sources)?;
        let total = Buffer::<u64>::zeroed(pool.clone(), 1)?;
        let maxword = zero(1)?;
        m.up_ranges(stream, &m.prepare_up_ranges::<WR>(cfg(sources))?, rv, src.ptr(), sources,
                    fmap.ptr(), spans.ptr(), off.ptr(), total.ptr(), maxword.ptr())?;
        let gmap = zero(ng)?;
        m.up_live_edges(stream, &m.prepare_up_live_edges(cfg(n))?, edges.ptr(), n, fmap.ptr(), gmap.ptr())?;
        let children = read64(&total)? as usize;
        if children == 0 {
            return empty(arena);
        }

        // A fragment covers at least one physical child; the range reservation
        // sized by physical children is therefore sufficient across all runs.
        let nw = read(&maxword)?[0] as usize + 1;
        if nw > MAX_WORDS { return Ok(None); }
        let frag = u64s(children)?;
        let length = zero(sources)?;
        let wmap = zero(nw)?;
        // Four CTAs form a complete cohort. Only a span count beyond CUDA's
        // maximum grid-x needs the kernel's grid-stride continuation.
        let blocks_per_span = FRAGMENT_WARPS / 4;
        let span_blocks = segments.min(i32::MAX as usize / blocks_per_span) * blocks_per_span;
        let fragment_cfg = LaunchConfig1D::new(span_blocks as u32, 128, 0);
        m.up_fragments(stream, &m.prepare_up_fragments::<WR>(fragment_cfg)?, rv, spans.ptr(), segments,
                       off.ptr(), length.ptr(), frag.ptr(), wmap.ptr())?;
        let nc = zero(2)?;
        let gdecode = u32s(n.min(ng))?;
        let wdecode = u32s(nw)?;
        m.up_rank(stream, &m.prepare_up_rank(cfg(ng))?, gmap.ptr(), ng, gdecode.ptr(), nc.ptr())?;
        m.up_rank(stream, &m.prepare_up_rank(cfg(nw))?, wmap.ptr(), nw, wdecode.ptr(), unsafe { nc.ptr().add(1) })?;
        m.up_remap_fragments(stream, &m.prepare_up_remap_fragments(cfg(sources * 32))?, off.ptr(), length.ptr(),
                             sources, frag.ptr(), wmap.ptr())?;
        let nc = read(&nc)?;
        let (groups, words) = (nc[0] as usize, nc[1] as usize);
        debug_assert!(groups > 0 && words > 0);

        // Interning pays for a second matrix (group × source) to eliminate
        // repeated fragment visits and publishes. Bound both matrices, as in
        // the ordinary striped route; fall back if lists turn out dissimilar.
        let width = sources.div_ceil(32);
        if estimate >= 50_000_000
            && groups.saturating_mul(width) <= MAX_WORDS
        {
            let members = zero(groups * width)?;
            m.up_members(stream, &m.prepare_up_members(cfg(n))?, edges.ptr(), n, gmap.ptr(), width, members.ptr())?;
            let slots = (groups * 2).next_power_of_two();
            let table = zero(slots)?;
            let owner = u32s(groups)?;
            let rank = u32s(groups)?;
            let catalog = u32s(groups)?;
            let count = zero(1)?;
            m.up_intern(stream, &m.prepare_up_intern(cfg(groups * 32))?, members.ptr(), groups, width,
                        table.ptr(), slots - 1, owner.ptr(), rank.ptr(), catalog.ptr(), count.ptr())?;
            let reps = read(&count)?[0] as usize;
            if reps * 2 <= groups && reps.saturating_mul(words) <= MAX_WORDS {
                let bits = zero(reps * words)?;
                let membership_words = reps * width;
                if membership_words <= MAX_JOBS / 32 {
                    let jobs = u64s(membership_words * 32)?;
                    let count = zero(1)?;
                    let gen_cfg = LaunchConfig1D::new(membership_words.div_ceil(128) as u32, 128, 0);
                    m.up_rep_jobs(stream, &m.prepare_up_rep_jobs(gen_cfg)?, members.ptr(), catalog.ptr(),
                                  reps, width, jobs.ptr(), count.ptr())?;
                    let take_cfg = LaunchConfig1D::new(resident, 128, 0);
                    m.up_take_rep_jobs(stream, &m.prepare_up_take_rep_jobs(take_cfg)?, jobs.ptr(), count.ptr(),
                                       off.ptr(), length.ptr(), frag.ptr(), words, bits.ptr())?;
                } else {
                    m.up_union_reps(stream, &m.prepare_up_union_reps(cfg(membership_words * 32))?, members.ptr(),
                                    catalog.ptr(), reps, width, off.ptr(), length.ptr(), frag.ptr(), words, bits.ptr())?;
                }
                let tiles = words.div_ceil(32);
                let tile_offset = u32s(reps * tiles)?;
                let counts = zero(reps)?;
                let bases = u64s(reps)?;
                let out_off = u64s(groups)?;
                let totals = Buffer::<u64>::zeroed(pool.clone(), 2)?;
                let child_kernel = m.prepare_up_rep_children(cfg(reps * tiles * 32))?;
                m.up_rep_children(stream, &child_kernel, bits.ptr(), reps, words, tiles, counts.ptr(),
                                  tile_offset.ptr(), bases.ptr(), wdecode.ptr(), std::ptr::null_mut())?;
                m.up_rep_layout(stream, &m.prepare_up_rep_layout(cfg(groups))?, groups, owner.ptr(), rank.ptr(),
                                counts.ptr(), bases.ptr(), out_off.ptr(), totals.ptr())?;
                let sizes = totals.to_host()?;
                crate::record_readback();
                let rows = usize::try_from(sizes[0])?;
                if rows > u32::MAX as usize {
                    return Err("UnionProject candidate count exceeds arena limit".into());
                }
                if sizes[1] <= MAX_WORDS as u64 {
                    let child = u32s(sizes[1] as usize)?;
                    m.up_rep_children(stream, &child_kernel, bits.ptr(), reps, words, tiles, counts.ptr(),
                                      tile_offset.ptr(), bases.ptr(), wdecode.ptr(), child.ptr())?;
                    let lease = arena.reserve(pack.clone(), rows)?;
                    m.up_copy_groups(stream, &m.prepare_up_copy_groups(cfg(groups * 256))?, groups, owner.ptr(),
                                     rank.ptr(), counts.ptr(), bases.ptr(), out_off.ptr(), child.ptr(),
                                     gdecode.ptr(), p, lease.buffer().ptr())?;
                    arena.record_count(rows);
                    if crate::trace_kernels() {
                        eprintln!("gpu_union_project\tmethod=interned\tdriver={n}\tsources={sources}\tgroups={groups}\trepresentatives={reps}\tcandidates={rows}");
                    }
                    return unsafe { lease.finish(rows, owners()).map(Some) };
                }
            }
        }

        // Stripe the group dimension at 256 MiB, with a minimum of one group.
        // A capacity miss does not require a second radix lookup. When the
        // bitmap fits in one stripe we need not replay the union either.
        let stripe = ((1usize << 26) / words).max(1).min(groups);
        let bits = u32s(stripe * words)?;
        // Allocate once across stripes/retries; do not add a host count gate.
        let compact = if n <= MAX_JOBS {
            let gen_cfg = LaunchConfig1D::new(n.div_ceil(128) as u32, 128, 0);
            // Keep all eight native warps per CTA (6,816 workers on Ada).
            let take_cfg = LaunchConfig1D::new(resident, 256, 0);
            Some((u64s(n)?, u32s(1)?, m.prepare_up_edge_jobs(gen_cfg)?, m.prepare_up_take_edge_jobs(take_cfg)?))
        } else { None };
        let mut cap = arena.capacity_hint(&pack);
        let mut populated = false;
        loop {
            let lease = arena.reserve(pack.clone(), cap)?;
            let rows = Buffer::<u64>::zeroed(pool.clone(), 1)?;
            for first in (0..groups).step_by(stripe) {
                let gs = stripe.min(groups - first);
                let size = gs * words;
                if !populated || stripe < groups {
                    unsafe { cuda_core::simt::memory::memset_d8_async(bits.cu_deviceptr(), 0, 4 * size, stream.cu_stream())?; }
                    if let Some((jobs, count, gen_kernel, take_kernel)) = &compact {
                        unsafe { cuda_core::simt::memory::memset_d8_async(count.cu_deviceptr(), 0, 4, stream.cu_stream())?; }
                        m.up_edge_jobs(stream, gen_kernel, edges.ptr(), n, gmap.ptr(), first, gs, jobs.ptr(), count.ptr())?;
                        m.up_take_edge_jobs(stream, take_kernel, jobs.ptr(), count.ptr(), off.ptr(), length.ptr(),
                                            frag.ptr(), words, bits.ptr())?;
                    } else {
                        m.up_union(stream, &m.prepare_up_union(cfg(n * 32))?, edges.ptr(), n, gmap.ptr(),
                                   off.ptr(), length.ptr(), frag.ptr(), first, gs, words, bits.ptr())?;
                    }
                }
                m.up_publish(stream, &m.prepare_up_publish(cfg(size))?, bits.ptr(), size, words,
                             unsafe { gdecode.ptr().add(first) }, wdecode.ptr(), p, rows.ptr(),
                             lease.buffer().ptr(), lease.capacity() as u64)?;
            }
            let found = read64(&rows)? as usize;
            if found > u32::MAX as usize {
                return Err("UnionProject candidate count exceeds arena limit".into());
            }
            if found <= lease.capacity() {
                arena.record_count(found);
                if crate::trace_kernels() {
                    eprintln!("gpu_union_project\tmethod=striped\tdriver={n}\tsources={sources}\tgroups={groups}\tcandidates={found}");
                }
                return unsafe { lease.finish(found, owners()).map(Some) };
            }
            cap = found;
            populated = true;
        }
    })
}

#[cfg(test)]
mod kernel_tests {
    use super::union_project_kernels;
    use crate::{Buffer, DeviceRadixRun, Pool, RadixLayout, RadixRunParams, Result, SortedBatch};
    use cuda_core::LaunchConfig1D;

    #[test]
    fn fragments_match_each_parents_window_multiset() -> Result<()> {
        type Layout = RadixLayout<(u16, u16), (), u64, u64, ()>;
        let pool = Pool::new(0, 0, 0)?;
        let pack = Layout::derive_packing()?;
        let params = RadixRunParams {
            ratio: 2, radix_bits: 0, merge_tile_rows: 32, block_threads: 256,
        };
        let mut set = DeviceRadixRun::new_set(pool.clone(), params, pack.bits())?;
        // Each parent has two disjoint runs. Odd children start at 3 so a
        // raw word straddles window boundaries, including the 16-warp wrap.
        let parents: [[Vec<u16>; 2]; 5] = [
            [(0..1091).map(|k| 3 + 2 * k).collect(), (0..65).map(|k| 2 * k).collect()],
            [vec![], vec![4096]],
            [vec![], vec![]],
            [(8000..8032).collect(), vec![]],
            [(200..233).collect(), (400..431).collect()],
        ];
        for r in 0..2 {
            let mut keys: Vec<u64> = Vec::new();
            for (parent, runs) in parents.iter().enumerate() {
                for &member in &runs[r] {
                    keys.push(pack.pack(&[parent as i128, i128::from(member)])?);
                }
            }
            let buf = Buffer::from_host(pool.clone(), &keys)?;
            // SAFETY: rows above are strictly increasing by (parent, member).
            unsafe { set.append_sorted(SortedBatch::from_sorted(buf, keys.len())?)?; }
        }
        let rhs = Layout::publish(&set, pack)?;
        assert_eq!(rhs.run_count(), 2);
        let view = rhs.view_on(&pool)?;
        let n = parents.len();
        let segments = n * rhs.run_count();
        let sources = Buffer::from_host(pool.clone(), &(0..n as u32).collect::<Vec<_>>())?;
        let offsets = Buffer::<u64>::zeroed(pool.clone(), n)?;
        let spans = Buffer::<u64>::zeroed(pool.clone(), 2 * segments)?;
        let fmap = Buffer::<u32>::zeroed(pool.clone(), n)?;
        let total = Buffer::<u64>::zeroed(pool.clone(), 1)?;
        let maxword = Buffer::<u32>::zeroed(pool.clone(), 1)?;
        let m = unsafe { union_project_kernels::load(pool.context())? };
        let stream = pool.stream();
        m.up_ranges(stream, &m.prepare_up_ranges::<u64>(LaunchConfig1D::new(1, 256, 0))?,
            view, sources.ptr(), n, fmap.ptr(), spans.ptr(), offsets.ptr(), total.ptr(), maxword.ptr())?;
        let lengths = Buffer::<u32>::zeroed(pool.clone(), n)?;
        let fragments = Buffer::<u64>::zeroed(pool.clone(), rhs.len())?;
        let words = usize::from(*parents.iter().flatten().flatten().max().unwrap()) / 32 + 1;
        let marked = Buffer::<u32>::zeroed(pool.clone(), words)?;
        // Two spans in flight, then grid-stride across the remaining parents.
        m.up_fragments(stream, &m.prepare_up_fragments::<u64>(LaunchConfig1D::new(8, 128, 0))?,
            view, spans.ptr(), segments, offsets.ptr(), lengths.ptr(), fragments.ptr(), marked.ptr())?;
        let (lengths, offsets, fragments) = (lengths.to_host()?, offsets.to_host()?, fragments.to_host()?);
        let mut expected_marks = vec![0; words];
        for (parent, runs) in parents.iter().enumerate() {
            let mut expected = Vec::new();
            for window in runs.iter().flat_map(|run| run.chunks(32)) {
                for segment in window.chunk_by(|a, b| a / 32 == b / 32) {
                    let word = segment[0] / 32;
                    let mask = segment.iter().fold(0u32, |bits, member| bits | (1 << (member % 32)));
                    expected.push((u64::from(word) << 32) | u64::from(mask));
                    expected_marks[usize::from(word)] = 1;
                }
            }
            assert_eq!(lengths[parent] as usize, expected.len(), "length of parent {parent}");
            let start = offsets[parent] as usize;
            let mut actual = fragments[start..start + expected.len()].to_vec();
            // Reservations are unordered. Don't dedup or OR across windows.
            expected.sort_unstable();
            actual.sort_unstable();
            assert_eq!(actual, expected, "fragments of parent {parent}");
        }
        assert_eq!(marked.to_host()?, expected_marks);
        Ok(())
    }

    #[test]
    fn compact_and_fallback_unions_match_every_bitmap_word() -> Result<()> {
        const SOURCES: usize = 1057;
        const WIDTH: usize = SOURCES.div_ceil(32);
        const WORDS: usize = 557;
        let pool = Pool::new(0, 0, 0)?;
        let stream = pool.stream();
        let m = unsafe { union_project_kernels::load(pool.context())? };
        let cfg = |n: usize, block: u32| LaunchConfig1D::new(n.div_ceil(block as usize) as u32, block, 0);
        let mut offsets = vec![0u64; SOURCES];
        let mut lengths = vec![0u32; SOURCES];
        let mut fragments = Vec::new();
        // A missing window cannot hide behind another source: each source
        // contributes a different bit, and no word repeats within its list.
        let lists = [(0, 523), (2, 127), (31, 128), (32, 129), (63, 1), (96, 32), (1024, 33), (1056, 31)];
        for (bit, (source, len)) in lists.into_iter().enumerate() {
            offsets[source] = fragments.len() as u64;
            lengths[source] = len as u32;
            for j in 0..len {
                let word = (73 * j + source) % WORDS;
                fragments.push(((word as u64) << 32) | (1 << bit));
            }
        }
        let rows: [&[usize]; 6] = [
            &[0, 31, 1056], &[2, 31, 32, 1024], &[32, 63],
            &[0, 31, 1056], &[63, 96, 1024], &[32, 63],
        ];
        let mut expected = vec![0u32; rows.len() * WORDS];
        for (g, sources) in rows.iter().enumerate() {
            for &f in *sources {
                let first = offsets[f] as usize;
                for &fragment in &fragments[first..first + lengths[f] as usize] {
                    expected[g * WORDS + (fragment >> 32) as usize] |= fragment as u32;
                }
            }
        }
        let offsets = Buffer::from_host(pool.clone(), &offsets)?;
        let lengths = Buffer::from_host(pool.clone(), &lengths)?;
        let fragments = Buffer::from_host(pool.clone(), &fragments)?;
        // Raw group IDs are not compact ranks. Group 6 has only failed lookups.
        let group_map = [4u32, 1, 2, 5, 3, 6, 0];
        let mut edges = Vec::new();
        for f in 0..SOURCES {
            if f % 8 == 0 { edges.push(6u64 << 32); }
            for (raw, &rank) in group_map[..6].iter().enumerate() {
                if rows[(rank - 1) as usize].contains(&f) {
                    edges.push(((raw as u64) << 32) | (f as u64 + 1));
                }
            }
        }
        let n = edges.len();
        let edges = Buffer::from_host(pool.clone(), &edges)?;
        let gmap = Buffer::from_host(pool.clone(), &group_map)?;
        let jobs = Buffer::<u64>::zeroed(pool.clone(), n)?;
        for first in (0..rows.len()).step_by(4) {
            let gs = 4.min(rows.len() - first);
            let count = Buffer::<u32>::zeroed(pool.clone(), 1)?;
            let compact = Buffer::<u32>::zeroed(pool.clone(), gs * WORDS)?;
            let fallback = Buffer::<u32>::zeroed(pool.clone(), gs * WORDS)?;
            m.up_edge_jobs(stream, &m.prepare_up_edge_jobs(cfg(n, 128))?,
                edges.ptr(), n, gmap.ptr(), first, gs, jobs.ptr(), count.ptr())?;
            let refs: usize = rows[first..first + gs].iter().map(|row| row.len()).sum();
            assert_eq!(count.to_host()?[0] as usize, refs);
            m.up_take_edge_jobs(stream, &m.prepare_up_take_edge_jobs(cfg(256, 256))?,
                jobs.ptr(), count.ptr(), offsets.ptr(), lengths.ptr(), fragments.ptr(), WORDS, compact.ptr())?;
            m.up_union(stream, &m.prepare_up_union(cfg(256, 256))?,
                edges.ptr(), n, gmap.ptr(), offsets.ptr(), lengths.ptr(), fragments.ptr(),
                first, gs, WORDS, fallback.ptr())?;
            let stripe = &expected[first * WORDS..(first + gs) * WORDS];
            assert_eq!(compact.to_host()?.as_slice(), stripe, "compact stripe {first}");
            assert_eq!(fallback.to_host()?.as_slice(), stripe, "fallback stripe {first}");
        }

        let members = Buffer::<u32>::zeroed(pool.clone(), rows.len() * WIDTH)?;
        m.up_members(stream, &m.prepare_up_members(cfg(n, 256))?, edges.ptr(), n, gmap.ptr(), WIDTH, members.ptr())?;
        // Each distinct row exactly once, in non-rank order. WIDTH=34 puts
        // sparse membership words on both sides of warp and CTA boundaries.
        let representatives = [4u32, 2, 1, 3];
        let reps = representatives.len();
        let catalog = Buffer::from_host(pool.clone(), &representatives)?;
        let count = Buffer::<u32>::zeroed(pool.clone(), 1)?;
        let compact = Buffer::<u32>::zeroed(pool.clone(), reps * WORDS)?;
        let fallback = Buffer::<u32>::zeroed(pool.clone(), reps * WORDS)?;
        m.up_rep_jobs(stream, &m.prepare_up_rep_jobs(cfg(reps * WIDTH, 128))?,
            members.ptr(), catalog.ptr(), reps, WIDTH, jobs.ptr(), count.ptr())?;
        let refs: usize = representatives.iter().map(|&g| rows[g as usize].len()).sum();
        assert_eq!(count.to_host()?[0] as usize, refs);
        m.up_take_rep_jobs(stream, &m.prepare_up_take_rep_jobs(cfg(128, 128))?,
            jobs.ptr(), count.ptr(), offsets.ptr(), lengths.ptr(), fragments.ptr(), WORDS, compact.ptr())?;
        m.up_union_reps(stream, &m.prepare_up_union_reps(cfg(256, 256))?,
            members.ptr(), catalog.ptr(), reps, WIDTH, offsets.ptr(), lengths.ptr(), fragments.ptr(), WORDS, fallback.ptr())?;
        let (compact, fallback) = (compact.to_host()?, fallback.to_host()?);
        for (r, g) in representatives.into_iter().enumerate() {
            let row = &expected[g as usize * WORDS..(g as usize + 1) * WORDS];
            assert_eq!(&compact[r * WORDS..(r + 1) * WORDS], row, "compact representative {r}");
            assert_eq!(&fallback[r * WORDS..(r + 1) * WORDS], row, "fallback representative {r}");
        }
        Ok(())
    }
}
