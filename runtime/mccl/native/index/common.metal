#include <metal_stdlib>
using namespace metal;
struct Row { uint key; uint value; };

kernel void copy_rows(const device uint4* src [[buffer(0)]], device uint4* dst [[buffer(1)]],
    constant uint* args [[buffer(2)]], uint tid [[thread_position_in_grid]], uint stride [[threads_per_grid]]) {
    uint n = args[0];
    for (uint i = tid; i < n / 2; i += stride) dst[i] = src[i];
    if ((n & 1) && tid == 0) {
        ((device uint2*)dst)[n - 1] = ((const device uint2*)src)[n - 1];
    }
}

inline uint group_prefix(uint v, uint lane, uint simd, threadgroup uint* scratch, thread uint& total) {
    uint p = simd_prefix_exclusive_sum(v);
    uint s = simd_sum(v);
    if (lane == 0) scratch[simd] = s;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (simd == 0) {
        uint x = lane < 8 ? scratch[lane] : 0;
        uint prefix = simd_prefix_exclusive_sum(x);
        uint sum = simd_sum(x);
        if (lane < 8) scratch[lane] = prefix;
        if (lane == 0) scratch[8] = sum;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    p += scratch[simd];
    total = scratch[8];
    threadgroup_barrier(mem_flags::mem_threadgroup);
    return p;
}

kernel void group_count(const device Row* rows [[buffer(0)]], device uint* counts [[buffer(1)]],
    constant uint* args [[buffer(2)]], uint tg [[threadgroup_position_in_grid]],
    uint lid [[thread_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]], uint simd [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint scratch[9];
    uint n = args[0], value = 0;
    for (uint i = tg * 2048 + lid; i < min(n, (tg + 1) * 2048); i += 256)
        value += (i == 0 || rows[i - 1].key != rows[i].key);
    uint total;
    group_prefix(value, lane, simd, scratch, total);
    if (lid == 0) counts[tg] = total;
}

// Each lane owns a contiguous interval. The single metadata scan touches only
// one count per 2048 rows; it does not serialize the row stream.
kernel void group_scan(const device uint* counts [[buffer(0)]], device uint* offsets [[buffer(1)]],
    device uint* total_out [[buffer(2)]], device uint* starts [[buffer(3)]],
    constant uint* args [[buffer(4)]], uint lid [[thread_index_in_threadgroup]],
    uint lane [[thread_index_in_simdgroup]], uint simd [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint scratch[9];
    uint tiles = args[0], chunk = (tiles + 255) / 256;
    uint begin = min(tiles, lid * chunk), end = min(tiles, begin + chunk), sum = 0;
    for (uint i = begin; i < end; ++i) sum += counts[i];
    uint total;
    uint prefix = group_prefix(sum, lane, simd, scratch, total);
    for (uint i = begin; i < end; ++i) { offsets[i] = prefix; prefix += counts[i]; }
    if (lid == 0) { total_out[0] = total; starts[total] = args[1]; }
}

kernel void group_scatter(const device Row* rows [[buffer(0)]], const device uint* offsets [[buffer(1)]],
    device uint* keys [[buffer(2)]], device uint* starts [[buffer(3)]], constant uint* args [[buffer(4)]],
    uint tg [[threadgroup_position_in_grid]], uint lid [[thread_index_in_threadgroup]],
    uint lane [[thread_index_in_simdgroup]], uint simd [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint scratch[9];
    uint n = args[0], base = offsets[tg];
    for (uint stripe = 0; stripe < 8; ++stripe) {
        uint i = tg * 2048 + stripe * 256 + lid;
        bool head = i < n && (i == 0 || rows[i - 1].key != rows[i].key);
        uint total;
        uint rank = group_prefix(uint(head), lane, simd, scratch, total);
        if (head) { keys[base + rank] = rows[i].key; starts[base + rank] = i; }
        base += total;
    }
}
