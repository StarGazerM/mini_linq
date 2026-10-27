// Stable 4-bit LSD radix stages. A tile has 256 threads and 4 or 8 rows/thread.
// Row and metal_stdlib are supplied by common.metal.

template<uint Items>
inline void sort_histogram_impl(
    device const Row* input,
    device uint* histogram,
    constant uint* args,
    uint tile, uint tid, uint lane, uint subgroup,
    threadgroup uint* subgroup_counts)
{
    const uint n = args[0], tiles = args[1], shift = args[2];
    const uint begin = tile * (256u * Items);
    uint counts[16] = {};
    for (uint item = 0; item < Items; ++item) {
        const uint index = begin + item * 256u + tid;
        if (index < n) {
            ++counts[(input[index].key >> shift) & 15u];
        }
    }
    for (uint digit = 0; digit < 16; ++digit) {
        const uint count = simd_sum(counts[digit]);
        if (lane == 0) {
            subgroup_counts[digit * 8u + subgroup] = count;
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (tid < 16) {
        uint count = 0;
        for (uint group = 0; group < 8; ++group) {
            count += subgroup_counts[tid * 8u + group];
        }
        // One flat exclusive scan supplies both earlier digits' totals and
        // earlier tiles' counts for this digit. No CPU offsets are involved.
        histogram[tid * tiles + tile] = count;
    }
}

#define SORT_HISTOGRAM_KERNEL(ITEMS) \
kernel void sort_histogram##ITEMS( \
    device const Row* input [[buffer(0)]], \
    device uint* histogram [[buffer(1)]], \
    constant uint* args [[buffer(2)]], \
    uint tile [[threadgroup_position_in_grid]], \
    uint tid [[thread_index_in_threadgroup]], \
    uint lane [[thread_index_in_simdgroup]], \
    uint subgroup [[simdgroup_index_in_threadgroup]]) \
{ \
    threadgroup uint subgroup_counts[16 * 8]; \
    sort_histogram_impl<ITEMS>(input, histogram, args, tile, tid, lane, subgroup, subgroup_counts); \
}

SORT_HISTOGRAM_KERNEL(4)
SORT_HISTOGRAM_KERNEL(8)
#undef SORT_HISTOGRAM_KERNEL

// Exclusive scan of 1024 consecutive uints, four consecutive values/thread.
// Out-of-range values are zero. Recursively scanning block_sums, then adding
// their exclusive offsets, yields a full arbitrary-length exclusive scan.
kernel void sort_scan_blocks(
    device uint* data [[buffer(0)]],
    device uint* block_sums [[buffer(1)]],
    constant uint* args [[buffer(2)]],
    uint block [[threadgroup_position_in_grid]],
    uint tid [[thread_index_in_threadgroup]],
    uint lane [[thread_index_in_simdgroup]],
    uint subgroup [[simdgroup_index_in_threadgroup]])
{
    threadgroup uint offsets[9];
    const uint begin = block * 1024u + tid * 4u;
    uint values[4];
    uint sum = 0;
    for (uint i = 0; i < 4; ++i) {
        values[i] = begin + i < args[0] ? data[begin + i] : 0u;
        sum += values[i];
    }
    const uint within_subgroup = simd_prefix_exclusive_sum(sum);
    const uint subgroup_sum = simd_sum(sum);
    if (lane == 0) {
        offsets[subgroup] = subgroup_sum;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (subgroup == 0) {
        const uint count = lane < 8 ? offsets[lane] : 0u;
        const uint prefix = simd_prefix_exclusive_sum(count);
        const uint total = simd_sum(count);
        if (lane < 8) {
            offsets[lane] = prefix;
        }
        if (lane == 0) {
            offsets[8] = total;
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint prefix = offsets[subgroup] + within_subgroup;
    for (uint i = 0; i < 4; ++i) {
        if (begin + i < args[0]) {
            data[begin + i] = prefix;
        }
        prefix += values[i];
    }
    if (tid == 0) {
        block_sums[block] = offsets[8];
    }
}

kernel void sort_add_block_offsets(
    device uint* data [[buffer(0)]],
    device const uint* block_offsets [[buffer(1)]],
    constant uint* args [[buffer(2)]],
    uint block [[threadgroup_position_in_grid]],
    uint tid [[thread_index_in_threadgroup]])
{
    threadgroup uint offset;
    if (tid == 0) {
        offset = block_offsets[block];
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint i = 0; i < 4; ++i) {
        const uint index = block * 1024u + i * 256u + tid;
        if (index < args[0]) {
            data[index] += offset;
        }
    }
}

template<uint Items, uint Bits = 4>
inline void sort_scatter_impl(
    device const Row* input,
    device Row* output,
    device const uint* histogram,
    constant uint* args,
    uint tile, uint tid, uint lane, uint subgroup,
    threadgroup Row* ordered,
    threadgroup uint* partition_offsets,
    threadgroup uint* digit_starts,
    threadgroup uint* digit_bases)
{
    const uint n = args[0], tiles = args[1], shift = args[2];
    const uint begin = tile * (256u * Items);
    const uint mask = (1u << Bits) - 1u;
    Row rows[Items];
    for (uint item = 0; item < Items; ++item) {
        const uint index = begin + item * 256u + tid;
        // Padding has the highest digit and starts after all real rows.
        // Stable partitions keep it after real maximum-key rows.
        rows[item] = index < n ? input[index] : Row{0xffffffffu, 0u};
    }

    // Stable binary partitions create a digit-sorted tile. Ranks follow
    // source position (item, subgroup, lane), never payload values or atomics.
    for (uint bit = shift; bit < shift + Bits; ++bit) {
        uint zero_prefixes[Items];
        for (uint item = 0; item < Items; ++item) {
            const uint zero = ((rows[item].key >> bit) & 1u) ^ 1u;
            zero_prefixes[item] = simd_prefix_exclusive_sum(zero);
            const uint count = simd_sum(zero);
            if (lane == 0) {
                partition_offsets[item * 8u + subgroup] = count;
            }
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
        if (subgroup == 0) {
            uint preceding = 0;
            for (uint chunk = 0; chunk < Items / 4u; ++chunk) {
                const uint index = chunk * 32u + lane;
                const uint count = partition_offsets[index];
                const uint prefix = simd_prefix_exclusive_sum(count);
                const uint total = simd_sum(count);
                partition_offsets[index] = preceding + prefix;
                preceding += total;
            }
            if (lane == 0) {
                partition_offsets[Items * 8u] = preceding;
            }
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
        const uint zeros = partition_offsets[Items * 8u];
        for (uint item = 0; item < Items; ++item) {
            const uint position = item * 256u + tid;
            const uint zero_rank = partition_offsets[item * 8u + subgroup] + zero_prefixes[item];
            const bool is_zero = ((rows[item].key >> bit) & 1u) == 0;
            const uint rank = is_zero ? zero_rank : zeros + position - zero_rank;
            ordered[rank] = rows[item];
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
        for (uint item = 0; item < Items; ++item) {
            rows[item] = ordered[item * 256u + tid];
        }
        // Next iteration's rank-scan barriers also finish these reads before
        // the next write to ordered. No extra shared-array copy is necessary.
    }

    if (tid < (1u << Bits)) {
        digit_bases[tid] = histogram[tid * tiles + tile];
    }
    for (uint item = 0; item < Items; ++item) {
        const uint position = item * 256u + tid;
        if (begin + position < n) {
            const uint digit = (rows[item].key >> shift) & mask;
            if (position == 0 || ((ordered[position - 1].key >> shift) & mask) != digit) {
                digit_starts[digit] = position;
            }
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    // Adjacent lanes now write adjacent positions within a digit run, rather
    // than issuing one scattered store per arbitrary unsorted input row.
    for (uint item = 0; item < Items; ++item) {
        const uint position = item * 256u + tid;
        if (begin + position < n) {
            const uint digit = (rows[item].key >> shift) & mask;
            output[digit_bases[digit] + position - digit_starts[digit]] = rows[item];
        }
    }
}

#define SORT_SCATTER_KERNEL(ITEMS) \
kernel void sort_scatter##ITEMS( \
    device const Row* input [[buffer(0)]], \
    device Row* output [[buffer(1)]], \
    device const uint* histogram [[buffer(2)]], \
    constant uint* args [[buffer(3)]], \
    uint tile [[threadgroup_position_in_grid]], \
    uint tid [[thread_index_in_threadgroup]], \
    uint lane [[thread_index_in_simdgroup]], \
    uint subgroup [[simdgroup_index_in_threadgroup]]) \
{ \
    threadgroup Row ordered[256 * ITEMS]; \
    threadgroup uint partition_offsets[8 * ITEMS + 1]; \
    threadgroup uint digit_starts[16]; \
    threadgroup uint digit_bases[16]; \
    sort_scatter_impl<ITEMS>(input, output, histogram, args, tile, tid, lane, subgroup, \
                           ordered, partition_offsets, digit_starts, digit_bases); \
}

SORT_SCATTER_KERNEL(4)
SORT_SCATTER_KERNEL(8)
#undef SORT_SCATTER_KERNEL

// Eight-bit histogram counters are private to each SIMD group. Uniform
// SIMD digits contribute one atomic count, avoiding duplicate-key contention.
template<uint Items>
inline void sort_histogram_byte_impl(const device Row* input, device uint* histogram,
    constant uint* args, uint tile, uint tid, uint lane, uint subgroup,
    threadgroup atomic_uint* counts)
{
    for (uint i = tid; i < 2048; i += 256)
        atomic_store_explicit(&counts[i], 0u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint item = 0; item < Items; ++item) {
        const uint index = tile * 256u * Items + item * 256u + tid;
        const uint digit = index < args[0] ? ((input[index].key >> args[2]) & 255u) : 256u;
        if (simd_all(digit == simd_broadcast_first(digit))) {
            if (lane == 0 && digit < 256)
                atomic_fetch_add_explicit(&counts[subgroup * 256 + digit], 32u, memory_order_relaxed);
        } else if (digit < 256) {
            atomic_fetch_add_explicit(&counts[subgroup * 256 + digit], 1u, memory_order_relaxed);
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint total = 0;
    for (uint group = 0; group < 8; ++group)
        total += atomic_load_explicit(&counts[group * 256 + tid], memory_order_relaxed);
    histogram[tid * args[1] + tile] = total;
}

#define SORT_BYTE_KERNELS(ITEMS) \
kernel void sort_histogram_byte##ITEMS( \
    const device Row* input [[buffer(0)]], device uint* histogram [[buffer(1)]], \
    constant uint* args [[buffer(2)]], uint tile [[threadgroup_position_in_grid]], \
    uint tid [[thread_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]], \
    uint subgroup [[simdgroup_index_in_threadgroup]]) \
{ \
    threadgroup atomic_uint counts[2048]; \
    sort_histogram_byte_impl<ITEMS>(input, histogram, args, tile, tid, lane, subgroup, counts); \
} \
kernel void sort_scatter_byte##ITEMS( \
    const device Row* input [[buffer(0)]], device Row* output [[buffer(1)]], \
    const device uint* histogram [[buffer(2)]], constant uint* args [[buffer(3)]], \
    uint tile [[threadgroup_position_in_grid]], uint tid [[thread_index_in_threadgroup]], \
    uint lane [[thread_index_in_simdgroup]], uint subgroup [[simdgroup_index_in_threadgroup]]) \
{ \
    threadgroup Row ordered[256 * ITEMS]; \
    threadgroup uint partition_offsets[8 * ITEMS + 1]; \
    threadgroup uint digit_starts[256]; \
    threadgroup uint digit_bases[256]; \
    sort_scatter_impl<ITEMS, 8>(input, output, histogram, args, tile, tid, lane, subgroup, \
        ordered, partition_offsets, digit_starts, digit_bases); \
}
SORT_BYTE_KERNELS(4)
SORT_BYTE_KERNELS(8)
#undef SORT_BYTE_KERNELS
