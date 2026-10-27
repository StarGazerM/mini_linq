// Included after common.metal, which supplies Row and the Metal namespace.
// All comparisons use only key. The A-before-B tie rule preserves source order
// within each run and places the older run before newly appended equal keys.

// Return A's coordinate on merged diagonal d. A valid partition satisfies
// A[i-1] <= B[j] and B[j-1] < A[i]. Advancing on equality is essential:
// partitioning on payloads or using the opposite tie rule is not stable.
inline uint maintain_path_device(
    const device Row* a, uint na, const device Row* b, uint nb, uint d)
{
    uint lo = d > nb ? d - nb : 0u;
    uint hi = min(d, na);
    while (lo < hi) {
        const uint i = lo + (hi - lo) / 2u;
        const uint j = d - i;
        if (j > 0u && b[j - 1u].key >= a[i].key) {
            lo = i + 1u;
        } else {
            hi = i;
        }
    }
    return lo;
}

inline uint maintain_path_shared(
    const threadgroup Row* a, uint na,
    const threadgroup Row* b, uint nb, uint d)
{
    uint lo = d > nb ? d - nb : 0u;
    uint hi = min(d, na);
    while (lo < hi) {
        const uint i = lo + (hi - lo) / 2u;
        const uint j = d - i;
        if (j > 0u && b[j - 1u].key >= a[i].key) {
            lo = i + 1u;
        } else {
            hi = i;
        }
    }
    return lo;
}

// Baseline: every thread searches a global merge-path diagonal, then produces
// items consecutive rows directly in global memory. No threadgroup staging.
// Logical streaming traffic: 16*(na+nb) bytes, plus binary-search key reads.
kernel void maintain_merge_direct(
    const device Row* a [[buffer(0)]],
    const device Row* b [[buffer(1)]],
    device Row* out [[buffer(2)]],
    constant uint* args [[buffer(3)]],
    uint gid [[thread_position_in_grid]])
{
    const uint na = args[0];
    const uint nb = args[1];
    const uint items = args[2];
    const uint n = na + nb;
    // Host accepts only 4/8 items and 256-thread groups, so even padded
    // thread starts fit u32 when n <= UINT_MAX.
    const uint d = gid * items;
    if (d >= n) {
        return;
    }
    uint i = maintain_path_device(a, na, b, nb, d);
    uint j = d - i;
    const uint count = min(items, n - d);
    for (uint k = 0u; k < count; ++k) {
        if (i < na && (j == nb || a[i].key <= b[j].key)) {
            out[d + k] = a[i++];
        } else {
            out[d + k] = b[j++];
        }
    }
}

// Global partitions delimit one output tile. Cooperative striped loads fetch
// two contiguous source spans, local merge paths produce short consecutive
// chunks, and striped stores make global output writes coalesced as well.
inline void maintain_tiled_body(
    const device Row* a, const device Row* b, device Row* out,
    constant uint* args, uint tid, uint group, uint items,
    threadgroup Row* input_tile, threadgroup Row* output_tile,
    const device uint* partitions, bool prepartitioned)
{
    const uint na = args[0];
    const uint nb = args[1];
    const uint n = na + nb;
    const uint tile_size = 256u * items;
    const uint d = group * tile_size;
    const uint count = min(tile_size, n - d);

    // Reuse two output slots as partition metadata until all source loads
    // finish. This keeps total threadgroup memory at exactly 16/32 KiB,
    // rather than exceeding the hardware's 32 KiB limit with extra scalars.
    uint a0, b0, ca, cb;
    if (prepartitioned) {
        a0 = partitions[group];
        b0 = d - a0;
        ca = partitions[group + 1] - a0;
        cb = count - ca;
    } else {
        if (tid == 0u) {
            const uint first = maintain_path_device(a, na, b, nb, d);
            const uint last = maintain_path_device(a, na, b, nb, d + count);
            output_tile[0].key = first;
            output_tile[0].value = d - first;
            output_tile[1].key = last - first;
            output_tile[1].value = count - (last - first);
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
        a0 = output_tile[0].key;
        b0 = output_tile[0].value;
        ca = output_tile[1].key;
        cb = output_tile[1].value;
    }

    for (uint k = tid; k < count; k += 256u) {
        if (k < ca) {
            input_tile[k] = a[a0 + k];
        } else {
            input_tile[k] = b[b0 + k - ca];
        }
    }
    // All threads have copied metadata to private scalars before reuse.
    threadgroup_barrier(mem_flags::mem_threadgroup);

    const uint local_d = tid * items;
    if (local_d < count) {
        uint i = maintain_path_shared(input_tile, ca, input_tile + ca, cb, local_d);
        uint j = local_d - i;
        const uint local_count = min(items, count - local_d);
        for (uint k = 0u; k < local_count; ++k) {
            if (i < ca && (j == cb || input_tile[i].key <= input_tile[ca + j].key)) {
                output_tile[local_d + k] = input_tile[i++];
            } else {
                output_tile[local_d + k] = input_tile[ca + j++];
            }
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint k = tid; k < count; k += 256u) {
        out[d + k] = output_tile[k];
    }
}

kernel void maintain_merge_tiled_4(
    const device Row* a [[buffer(0)]],
    const device Row* b [[buffer(1)]],
    device Row* out [[buffer(2)]],
    constant uint* args [[buffer(3)]],
    uint tid [[thread_index_in_threadgroup]],
    uint group [[threadgroup_position_in_grid]])
{
    threadgroup Row input_tile[1024];
    threadgroup Row output_tile[1024];
    maintain_tiled_body(a, b, out, args, tid, group, 4u, input_tile, output_tile, nullptr, false);
}

kernel void maintain_merge_tiled_8(
    const device Row* a [[buffer(0)]],
    const device Row* b [[buffer(1)]],
    device Row* out [[buffer(2)]],
    constant uint* args [[buffer(3)]],
    uint tid [[thread_index_in_threadgroup]],
    uint group [[threadgroup_position_in_grid]])
{
    threadgroup Row input_tile[2048];
    threadgroup Row output_tile[2048];
    maintain_tiled_body(a, b, out, args, tid, group, 8u, input_tile, output_tile, nullptr, false);
}

kernel void maintain_partition(
    const device Row* a [[buffer(0)]], const device Row* b [[buffer(1)]],
    device uint* partitions [[buffer(2)]], constant uint* args [[buffer(3)]],
    uint gid [[thread_position_in_grid]])
{
    const uint n = args[0] + args[1];
    const uint tile = 256u * args[2];
    const uint tiles = n / tile + uint(n % tile != 0);
    if (gid <= tiles) {
        const uint diagonal = gid == tiles ? n : gid * tile;
        partitions[gid] = maintain_path_device(a, args[0], b, args[1], diagonal);
    }
}

#define MAINTAIN_PARTITIONED(ITEMS) \
kernel void maintain_merge_partitioned_##ITEMS( \
    const device Row* a [[buffer(0)]], const device Row* b [[buffer(1)]], \
    device Row* out [[buffer(2)]], const device uint* partitions [[buffer(3)]], \
    constant uint* args [[buffer(4)]], uint tid [[thread_index_in_threadgroup]], \
    uint group [[threadgroup_position_in_grid]]) \
{ \
    threadgroup Row input_tile[256 * ITEMS]; \
    threadgroup Row output_tile[256 * ITEMS]; \
    maintain_tiled_body(a, b, out, args, tid, group, ITEMS, input_tile, output_tile, partitions, true); \
}
MAINTAIN_PARTITIONED(4)
MAINTAIN_PARTITIONED(8)
#undef MAINTAIN_PARTITIONED

// Exact radix-12 directory, NOT RadixSpline: each entry is an exact lower
// bound, not a fitted prediction or bounded-error approximation. Parallel
// lower bounds avoid an extra full scan solely to construct small metadata.
kernel void maintain_directory(
    const device Row* rows [[buffer(0)]],
    device uint* dir [[buffer(1)]],
    constant uint* args [[buffer(2)]],
    uint p [[thread_position_in_grid]])
{
    if (p > 4096u) {
        return;
    }
    const uint n = args[0];
    if (p == 4096u) {
        dir[p] = n; // Do not compute 4096 << 20: that would wrap to zero.
        return;
    }
    if (p == 0u) {
        dir[p] = 0u;
        return;
    }
    const uint key = p << 20u;
    uint lo = 0u;
    uint hi = n;
    while (lo < hi) {
        const uint mid = lo + (hi - lo) / 2u;
        if (rows[mid].key < key) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    dir[p] = lo;
}

kernel void maintain_lookup(
    const device Row* rows [[buffer(0)]],
    const device uint* dir [[buffer(1)]],
    const device uint* queries [[buffer(2)]],
    device uint2* out [[buffer(3)]],
    constant uint* args [[buffer(4)]],
    uint q [[thread_position_in_grid]])
{
    const uint n = args[0];
    if (q >= args[1]) {
        return;
    }
    const uint key = queries[q];
    const uint p = key >> 20u;
    const uint end = dir[p + 1u];
    uint lo = dir[p];
    uint hi = end;
    while (lo < hi) {
        const uint mid = lo + (hi - lo) / 2u;
        if (rows[mid].key < key) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    const uint begin = lo;
    if (begin == end || rows[begin].key != key) {
        out[q] = uint2(n, n);
        return;
    }
    // Search the entire remaining bucket, not a fixed local window. Compare
    // with <= key instead of lower_bound(key+1), including key == UINT_MAX.
    lo = begin + 1u;
    hi = end;
    while (lo < hi) {
        const uint mid = lo + (hi - lo) / 2u;
        if (rows[mid].key <= key) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    out[q] = uint2(begin, lo);
}
