// Calibration kernels. All threadgroups are 256 wide (8 SIMD-groups of 32).
#include <metal_stdlib>
using namespace metal;

#define TG 256u
#define ITEMS 8u            // elements per thread in scan / partition / merge tiles
#define TILE (TG * ITEMS)   // 2048

// ---------------------------------------------------------------------------
// helpers

// Sum `v` over the threadgroup; result valid in all threads. `scratch` >= 8.
inline uint tg_sum(uint v, uint simd_lane, uint simd_id, threadgroup uint* scratch) {
    uint s = simd_sum(v);
    if (simd_lane == 0) scratch[simd_id] = s;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint t = simd_lane < 8 ? scratch[simd_lane] : 0u;
    t = simd_sum(t);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    return t;
}

// Exclusive prefix of `v` across the 256 threads; `total` receives the sum.
// `scratch` must hold >= 9 uints; two barriers inside, one more at the end so
// the caller can reuse `scratch` immediately.
inline uint tg_exclusive_scan(uint v, uint simd_lane, uint simd_id,
                              threadgroup uint* scratch, thread uint& total) {
    uint incl = simd_prefix_inclusive_sum(v);
    if (simd_lane == 31) scratch[simd_id] = incl;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (simd_id == 0) {
        uint t = simd_lane < 8 ? scratch[simd_lane] : 0u;
        uint e = simd_prefix_exclusive_sum(t);
        uint tot = simd_sum(t);
        if (simd_lane < 8) scratch[simd_lane] = e;
        if (simd_lane == 0) scratch[8] = tot;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint r = scratch[simd_id] + incl - v;
    total = scratch[8];
    threadgroup_barrier(mem_flags::mem_threadgroup);
    return r;
}

// ---------------------------------------------------------------------------
// streaming

kernel void seq_read(const device uint4* in [[buffer(0)]],
                     device atomic_uint* out [[buffer(1)]],
                     constant uint& n4 [[buffer(2)]],
                     uint gid [[thread_position_in_grid]],
                     uint gsize [[threads_per_grid]],
                     uint lid [[thread_index_in_threadgroup]],
                     uint simd_lane [[thread_index_in_simdgroup]],
                     uint simd_id [[simdgroup_index_in_threadgroup]]) {
    threadgroup atomic_uint tg_acc;
    if (lid == 0) atomic_store_explicit(&tg_acc, 0u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint s = 0;
    for (uint i = gid; i < n4; i += gsize) {
        uint4 v = in[i];
        s += v.x + v.y + v.z + v.w;
    }
    s = simd_sum(s);
    if (simd_lane == 0) atomic_fetch_add_explicit(&tg_acc, s, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (lid == 0) atomic_fetch_add_explicit(out, atomic_load_explicit(&tg_acc, memory_order_relaxed), memory_order_relaxed);
}

kernel void seq_write(device uint4* out [[buffer(0)]],
                      constant uint& n4 [[buffer(1)]],
                      constant uint& val [[buffer(2)]],
                      uint gid [[thread_position_in_grid]],
                      uint gsize [[threads_per_grid]]) {
    uint4 v = uint4(val, val + 1, val + 2, val + 3);
    for (uint i = gid; i < n4; i += gsize) out[i] = v;
}

kernel void seq_copy(const device uint4* in [[buffer(0)]],
                     device uint4* out [[buffer(1)]],
                     constant uint& n4 [[buffer(2)]],
                     uint gid [[thread_position_in_grid]],
                     uint gsize [[threads_per_grid]]) {
    for (uint i = gid; i < n4; i += gsize) out[i] = in[i];
}

// ---------------------------------------------------------------------------
// random line access. `idx` holds precomputed line numbers; a line is 32 uints.

kernel void gather_line(const device uint* table [[buffer(0)]],
                        const device uint* idx [[buffer(1)]],
                        device atomic_uint* out [[buffer(2)]],
                        constant uint& n [[buffer(3)]],
                        uint gid [[thread_position_in_grid]],
                        uint lid [[thread_index_in_threadgroup]],
                        uint simd_lane [[thread_index_in_simdgroup]],
                        uint simd_id [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint scratch[8];
    uint s = 0;
    if (gid < n) s = table[idx[gid] * 32u + (gid & 31u)];
    s = tg_sum(s, simd_lane, simd_id, scratch);
    if (lid == 0) atomic_fetch_add_explicit(out, s, memory_order_relaxed);
}

// One SIMD-group per 32 lines: for each line the 32 lanes load its 32 words
// coalesced. Same thread count and line list as gather_line.
kernel void gather_coop(const device uint* table [[buffer(0)]],
                        const device uint* idx [[buffer(1)]],
                        device atomic_uint* out [[buffer(2)]],
                        constant uint& nlines [[buffer(3)]],
                        uint gid [[thread_position_in_grid]],
                        uint lid [[thread_index_in_threadgroup]],
                        uint simd_lane [[thread_index_in_simdgroup]],
                        uint simd_id [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint scratch[8];
    uint first = (gid >> 5) * 32u;
    uint s = 0;
    for (uint l = first; l < min(first + 32u, nlines); l++) s += table[idx[l] * 32u + simd_lane];
    s = tg_sum(s, simd_lane, simd_id, scratch);
    if (lid == 0) atomic_fetch_add_explicit(out, s, memory_order_relaxed);
}

kernel void scatter_line(device uint* table [[buffer(0)]],
                         const device uint* idx [[buffer(1)]],
                         constant uint& n [[buffer(2)]],
                         uint gid [[thread_position_in_grid]]) {
    if (gid < n) table[idx[gid] * 32u + (gid & 31u)] = gid;
}

kernel void rmw_atomic(device atomic_uint* table [[buffer(0)]],
                       const device uint* idx [[buffer(1)]],
                       constant uint& n [[buffer(2)]],
                       uint gid [[thread_position_in_grid]]) {
    if (gid < n) atomic_fetch_add_explicit(&table[idx[gid] * 32u + (gid & 31u)], 1u, memory_order_relaxed);
}

// ---------------------------------------------------------------------------
// histogram, threadgroup-privatised in 32 KiB of threadgroup memory, flushed
// with one device atomic per non-zero bin per threadgroup.
//   bins <= 8192 : u32 counters, one pass.
//   bins >  8192 : two 16-bit counters per word (16384 bins per slice), one
//                  pass over the threadgroup's key chunk per slice; the host
//                  keeps per_tg < 65536 so a counter cannot overflow.

kernel void hist_tg(const device uint* keys [[buffer(0)]],
                    device atomic_uint* hist [[buffer(1)]],
                    constant uint& n [[buffer(2)]],
                    constant uint& bins [[buffer(3)]],
                    constant uint& per_tg [[buffer(4)]],
                    uint tg_id [[threadgroup_position_in_grid]],
                    uint lid [[thread_index_in_threadgroup]]) {
    threadgroup atomic_uint h[8192];
    uint begin = tg_id * per_tg;
    uint end = min(n, begin + per_tg);
    uint mask = bins - 1u;
    if (bins <= 8192u) {
        for (uint b = lid; b < bins; b += TG) atomic_store_explicit(&h[b], 0u, memory_order_relaxed);
        threadgroup_barrier(mem_flags::mem_threadgroup);
        for (uint i = begin + lid; i < end; i += TG)
            atomic_fetch_add_explicit(&h[keys[i] & mask], 1u, memory_order_relaxed);
        threadgroup_barrier(mem_flags::mem_threadgroup);
        for (uint b = lid; b < bins; b += TG) {
            uint v = atomic_load_explicit(&h[b], memory_order_relaxed);
            if (v != 0) atomic_fetch_add_explicit(&hist[b], v, memory_order_relaxed);
        }
        return;
    }
    uint passes = bins / 16384u;
    for (uint p = 0; p < passes; p++) {
        for (uint w = lid; w < 8192u; w += TG) atomic_store_explicit(&h[w], 0u, memory_order_relaxed);
        threadgroup_barrier(mem_flags::mem_threadgroup);
        for (uint i = begin + lid; i < end; i += TG) {
            uint k = keys[i] & mask;
            if ((k >> 14) == p) atomic_fetch_add_explicit(&h[(k & 16383u) >> 1], (k & 1u) ? 65536u : 1u, memory_order_relaxed);
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
        for (uint w = lid; w < 8192u; w += TG) {
            uint v = atomic_load_explicit(&h[w], memory_order_relaxed);
            uint lo = v & 0xFFFFu, hi = v >> 16;
            if (lo != 0) atomic_fetch_add_explicit(&hist[p * 16384u + 2u * w], lo, memory_order_relaxed);
            if (hi != 0) atomic_fetch_add_explicit(&hist[p * 16384u + 2u * w + 1u], hi, memory_order_relaxed);
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
}

// ---------------------------------------------------------------------------
// reduce-then-scan exclusive scan over `n` uints in blocks of TILE, three
// dispatches: block sums; exclusive scan of the block sums in one threadgroup;
// per-block scan with the block prefix.

inline uint load_masked(const device uint* in, uint i, uint n, uint mask) {
    return i < n ? (in[i] & mask) : 0u;
}

kernel void scan_reduce(const device uint* in [[buffer(0)]],
                        device uint* block_sums [[buffer(1)]],
                        constant uint& n [[buffer(2)]],
                        constant uint& mask [[buffer(3)]],
                        uint tg_id [[threadgroup_position_in_grid]],
                        uint lid [[thread_index_in_threadgroup]],
                        uint simd_lane [[thread_index_in_simdgroup]],
                        uint simd_id [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint scratch[8];
    uint base = tg_id * TILE;
    uint s = 0;
    for (uint k = 0; k < ITEMS; k++) s += load_masked(in, base + k * TG + lid, n, mask);
    s = tg_sum(s, simd_lane, simd_id, scratch);
    if (lid == 0) block_sums[tg_id] = s;
}

// Single threadgroup: in-place exclusive scan of `nblocks` block sums.
kernel void scan_blocksums(device uint* block_sums [[buffer(0)]],
                           constant uint& nblocks [[buffer(1)]],
                           uint lid [[thread_index_in_threadgroup]],
                           uint simd_lane [[thread_index_in_simdgroup]],
                           uint simd_id [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint scratch[9];
    uint carry = 0;
    for (uint base = 0; base < nblocks; base += TILE) {
        uint b = base + lid * ITEMS;
        uint v[ITEMS];
        uint t = 0;
        for (uint k = 0; k < ITEMS; k++) { v[k] = b + k < nblocks ? block_sums[b + k] : 0u; t += v[k]; }
        uint total;
        uint ex = tg_exclusive_scan(t, simd_lane, simd_id, scratch, total) + carry;
        for (uint k = 0; k < ITEMS; k++) {
            if (b + k < nblocks) block_sums[b + k] = ex;
            ex += v[k];
        }
        carry += total;
    }
}

kernel void scan_downsweep(const device uint* in [[buffer(0)]],
                           device uint* out [[buffer(1)]],
                           const device uint* block_prefix [[buffer(2)]],
                           constant uint& n [[buffer(3)]],
                           constant uint& mask [[buffer(4)]],
                           uint tg_id [[threadgroup_position_in_grid]],
                           uint lid [[thread_index_in_threadgroup]],
                           uint simd_lane [[thread_index_in_simdgroup]],
                           uint simd_id [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint scratch[9];
    uint pre = block_prefix[tg_id];
    // thread-local: ITEMS consecutive elements
    uint base = tg_id * TILE + lid * ITEMS;
    uint v[ITEMS];
    uint t = 0;
    for (uint k = 0; k < ITEMS; k++) { v[k] = load_masked(in, base + k, n, mask); t += v[k]; }
    uint total;
    uint ex = tg_exclusive_scan(t, simd_lane, simd_id, scratch, total) + pre;
    for (uint k = 0; k < ITEMS; k++) {
        if (base + k < n) out[base + k] = ex;
        ex += v[k];
    }
}

// ---------------------------------------------------------------------------
// single-pass decoupled look-back scan (feasibility probe).
// status word: bits 31..30 flag (0 none, 1 aggregate, 2 inclusive prefix),
// bits 29..0 value. Inputs are masked so sums fit 30 bits.

#define LB_FLAG_A 0x40000000u
#define LB_FLAG_P 0x80000000u
#define LB_VAL    0x3FFFFFFFu

kernel void lookback_scan(const device uint* in [[buffer(0)]],
                          device uint* out [[buffer(1)]],
                          device atomic_uint* status [[buffer(2)]],
                          device atomic_uint* ticket [[buffer(3)]],
                          device atomic_uint* failed [[buffer(4)]],
                          constant uint& n [[buffer(5)]],
                          constant uint& mask [[buffer(6)]],
                          constant uint& cap [[buffer(7)]],
                          uint lid [[thread_index_in_threadgroup]],
                          uint simd_lane [[thread_index_in_simdgroup]],
                          uint simd_id [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint scratch[9];
    threadgroup uint bcast[2]; // [0] tile ticket, [1] exclusive prefix
    if (lid == 0) bcast[0] = atomic_fetch_add_explicit(ticket, 1u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint tile = bcast[0];
    uint base = tile * TILE + lid * ITEMS;
    uint v[ITEMS];
    uint t = 0;
    for (uint k = 0; k < ITEMS; k++) { v[k] = load_masked(in, base + k, n, mask); t += v[k]; }
    uint total;
    uint ex = tg_exclusive_scan(t, simd_lane, simd_id, scratch, total);
    if (lid == 0) {
        uint prefix = 0;
        if (tile == 0) {
            atomic_store_explicit(&status[0], LB_FLAG_P | total, memory_order_relaxed);
        } else {
            atomic_store_explicit(&status[tile], LB_FLAG_A | total, memory_order_relaxed);
            uint pred = tile - 1;
            uint iters = 0;
            while (true) {
                uint s = atomic_load_explicit(&status[pred], memory_order_relaxed);
                uint f = s >> 30;
                if (f == 0) {
                    if (++iters > cap) { atomic_store_explicit(failed, 1u, memory_order_relaxed); break; }
                    continue;
                }
                prefix += s & LB_VAL;
                if (f == 2) break;
                pred--;
            }
            atomic_store_explicit(&status[tile], LB_FLAG_P | ((prefix + total) & LB_VAL), memory_order_relaxed);
        }
        bcast[1] = prefix;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    ex += bcast[1];
    for (uint k = 0; k < ITEMS; k++) {
        if (base + k < n) out[base + k] = ex;
        ex += v[k];
    }
}

// ---------------------------------------------------------------------------
// one 8-bit radix partition pass over (key, payload) pairs, tile = 2048.
// counts layout is bin-major: counts[bin * ntiles + tile].

kernel void part_hist(const device uint* keys [[buffer(0)]],
                      device uint* counts [[buffer(1)]],
                      constant uint& n [[buffer(2)]],
                      constant uint& ntiles [[buffer(3)]],
                      constant uint& shift [[buffer(4)]],
                      uint tg_id [[threadgroup_position_in_grid]],
                      uint lid [[thread_index_in_threadgroup]]) {
    threadgroup atomic_uint h[256];
    atomic_store_explicit(&h[lid], 0u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint base = tg_id * TILE;
    for (uint k = 0; k < ITEMS; k++) {
        uint i = base + k * TG + lid;
        if (i < n) atomic_fetch_add_explicit(&h[(keys[i] >> shift) & 255u], 1u, memory_order_relaxed);
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    counts[lid * ntiles + tg_id] = atomic_load_explicit(&h[lid], memory_order_relaxed);
}

kernel void part_scatter(const device uint* keys [[buffer(0)]],
                         const device uint* vals [[buffer(1)]],
                         device uint* out_keys [[buffer(2)]],
                         device uint* out_vals [[buffer(3)]],
                         const device uint* offsets [[buffer(4)]],
                         constant uint& n [[buffer(5)]],
                         constant uint& ntiles [[buffer(6)]],
                         constant uint& shift [[buffer(7)]],
                         uint tg_id [[threadgroup_position_in_grid]],
                         uint lid [[thread_index_in_threadgroup]],
                         uint simd_lane [[thread_index_in_simdgroup]],
                         uint simd_id [[simdgroup_index_in_threadgroup]]) {
    threadgroup atomic_uint h[256];
    threadgroup uint start[256];
    threadgroup uint tk[TILE];
    threadgroup uint tv[TILE];
    threadgroup uint scratch[9];
    atomic_store_explicit(&h[lid], 0u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint base = tg_id * TILE;
    uint k[ITEMS], v[ITEMS], r[ITEMS];
    for (uint j = 0; j < ITEMS; j++) {
        uint i = base + j * TG + lid;
        if (i < n) {
            k[j] = keys[i];
            v[j] = vals[i];
            r[j] = atomic_fetch_add_explicit(&h[(k[j] >> shift) & 255u], 1u, memory_order_relaxed);
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint cnt = atomic_load_explicit(&h[lid], memory_order_relaxed);
    uint total;
    start[lid] = tg_exclusive_scan(cnt, simd_lane, simd_id, scratch, total);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint j = 0; j < ITEMS; j++) {
        uint i = base + j * TG + lid;
        if (i < n) {
            uint p = start[(k[j] >> shift) & 255u] + r[j];
            tk[p] = k[j];
            tv[p] = v[j];
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint lim = min(TILE, n - base);
    for (uint p = lid; p < lim; p += TG) {
        uint key = tk[p];
        uint bin = (key >> shift) & 255u;
        uint dst = offsets[bin * ntiles + tg_id] + (p - start[bin]);
        out_keys[dst] = key;
        out_vals[dst] = tv[p];
    }
}

// ---------------------------------------------------------------------------
// merge path: merge sorted a[na] and b[nb] into out, tile = 2048 outputs.

inline uint merge_path_search(const device uint* a, const device uint* b, uint na, uint nb, uint diag) {
    uint lo = diag > nb ? diag - nb : 0u;
    uint hi = min(diag, na);
    while (lo < hi) {
        uint mid = (lo + hi) >> 1;
        if (a[mid] < b[diag - 1 - mid]) lo = mid + 1; else hi = mid;
    }
    return lo;
}

inline uint merge_path_search_tg(const threadgroup uint* a, const threadgroup uint* b, uint na, uint nb, uint diag) {
    uint lo = diag > nb ? diag - nb : 0u;
    uint hi = min(diag, na);
    while (lo < hi) {
        uint mid = (lo + hi) >> 1;
        if (a[mid] < b[diag - 1 - mid]) lo = mid + 1; else hi = mid;
    }
    return lo;
}

kernel void merge_path(const device uint* a [[buffer(0)]],
                       const device uint* b [[buffer(1)]],
                       device uint* out [[buffer(2)]],
                       constant uint& na [[buffer(3)]],
                       constant uint& nb [[buffer(4)]],
                       uint tg_id [[threadgroup_position_in_grid]],
                       uint lid [[thread_index_in_threadgroup]]) {
    threadgroup uint s[TILE];
    threadgroup uint bounds[2];
    uint total = na + nb;
    uint d0 = tg_id * TILE;
    uint d1 = min(total, d0 + TILE);
    if (lid < 2) bounds[lid] = merge_path_search(a, b, na, nb, lid == 0 ? d0 : d1);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint a0 = bounds[0], a1 = bounds[1];
    uint b0 = d0 - a0, b1 = d1 - a1;
    uint la = a1 - a0, lb = b1 - b0;
    for (uint i = lid; i < la; i += TG) s[i] = a[a0 + i];
    for (uint i = lid; i < lb; i += TG) s[la + i] = b[b0 + i];
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint ld = lid * ITEMS;
    uint n_out = la + lb;
    if (ld < n_out) {
        uint ai = merge_path_search_tg(s, s + la, la, lb, ld);
        uint bi = ld - ai;
        uint lim = min(ld + ITEMS, n_out);
        for (uint o = ld; o < lim; o++) {
            bool take_a = (bi >= lb) || (ai < la && s[ai] <= s[la + bi]);
            uint val = take_a ? s[ai] : s[la + bi];
            if (take_a) ai++; else bi++;
            out[d0 + o] = val;
        }
    }
}

// ---------------------------------------------------------------------------

kernel void empty(uint gid [[thread_position_in_grid]]) {}
