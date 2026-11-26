// CUDA DeviceRadixRun layout on Metal (GPU-RadixRun, STORAGE §8.2): packed
// keys of `lanes` 64-bit words, ordered most-significant lane last in lane
// numbering. Memory word m of a key holds lane (ms_first ? lanes-1-m : m),
// the byte layout of CUDA's u64/u128 (ms_first = 0) and [u64; L] (= 1).
// Row and metal_stdlib are supplied by common.metal.

constant uint kMaxWordLanes = 8;

inline ulong word_lane(const device ulong* keys, uint row, uint lanes, uint ms_first, uint lane) {
    return keys[ulong(row) * lanes + (ms_first != 0u ? lanes - 1u - lane : lane)];
}

// Stable row order, CUDA RowSortScratch: per active 64-bit lane, least
// significant first, the lane is gathered through the current order into
// {sort key, row} arrays A and B, then stable byte passes ping-pong between
// them. Passes cover the bytes of key - base: base 0 skips bytes constant
// over all rows (OR/AND bounds), as CUB skips trivial digit passes; base
// min sorts only the significant bytes of max - min, when that is fewer.
// The decisions live on the GPU: a plan records, for pass lane * 8 + byte,
// whether it is skipped and whether its source is B; slot kWordPlanGather +
// lane does the same for a lane gather, kWordPlanFinal names the array
// holding the final order and kWordPlanBase + 2 * lane holds the lane base.
constant uint kWordPlanGather = 64u;
constant uint kWordPlanFinal = 72u;
constant uint kWordPlanBase = 73u; // Plan length 89 must match mccl.cpp.
constant uint kWordInB = 1u;
constant uint kWordSkip = 2u;

// Device atomics only when they change the bound: random keys saturate it.
inline void words_fetch_or(device atomic_uint* word, uint bits) {
    const uint current = atomic_load_explicit(word, memory_order_relaxed);
    if ((current | bits) != current) atomic_fetch_or_explicit(word, bits, memory_order_relaxed);
}

inline void words_fetch_and(device atomic_uint* word, uint bits) {
    const uint current = atomic_load_explicit(word, memory_order_relaxed);
    if ((current & bits) != current) atomic_fetch_and_explicit(word, bits, memory_order_relaxed);
}

// 64-bit atomics have no load; a plain read of the bound is a hint only.
inline void words_fetch_min(device atomic_ulong* word, ulong value) {
    if (value < *reinterpret_cast<const device ulong*>(word))
        atomic_min_explicit(word, value, memory_order_relaxed);
}

inline void words_fetch_max(device atomic_ulong* word, ulong value) {
    if (value > *reinterpret_cast<const device ulong*>(word))
        atomic_max_explicit(word, value, memory_order_relaxed);
}

inline ulong words_simd_min(ulong value) {
    for (ushort delta = 16; delta > 0; delta >>= 1)
        value = min(value, as_type<ulong>(simd_shuffle_xor(as_type<uint2>(value), delta)));
    return value;
}

inline ulong words_simd_max(ulong value) {
    for (ushort delta = 16; delta > 0; delta >>= 1)
        value = max(value, as_type<ulong>(simd_shuffle_xor(as_type<uint2>(value), delta)));
    return value;
}

// args: n, lanes, ms_first; tiles of 2048 rows. Identity rows keyed by lane
// 0 into A, and per lane bounds {or lo, or hi, and lo, and hi, min, max}
// (8 uints; host-initialized to 0, 0, ~0, ~0, ~0, 0) over all rows. One
// conditional device atomic per tile and bound: same-address atomics
// serialize.
kernel void words_sort_first(
    const device ulong* keys [[buffer(0)]], device ulong* sort_keys [[buffer(1)]],
    device uint* sort_rows [[buffer(2)]], device atomic_uint* control [[buffer(3)]],
    constant uint* args [[buffer(4)]], uint tile [[threadgroup_position_in_grid]],
    uint tid [[thread_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]],
    uint subgroup [[simdgroup_index_in_threadgroup]])
{
    threadgroup ulong partial[8][4u * kMaxWordLanes];
    const uint n = args[0], lanes = args[1], ms_first = args[2];
    ulong any[kMaxWordLanes], all[kMaxWordLanes], low[kMaxWordLanes], high[kMaxWordLanes];
    for (uint l = 0; l < lanes; ++l) {
        any[l] = high[l] = 0ul;
        all[l] = low[l] = ~0ul;
    }
    for (uint item = 0; item < 8u; ++item) {
        const uint i = tile * 2048u + item * 256u + tid;
        if (i >= n) break;
        for (uint l = 0; l < lanes; ++l) {
            const ulong value = word_lane(keys, i, lanes, ms_first, l);
            if (l == 0u) {
                sort_keys[i] = value;
                sort_rows[i] = i;
            }
            any[l] |= value;
            all[l] &= value;
            low[l] = min(low[l], value);
            high[l] = max(high[l], value);
        }
    }
    for (uint l = 0; l < lanes; ++l) {
        const ulong bounds[4] = {
            ulong(simd_or(uint(any[l]))) | ulong(simd_or(uint(any[l] >> 32))) << 32,
            ulong(simd_and(uint(all[l]))) | ulong(simd_and(uint(all[l] >> 32))) << 32,
            words_simd_min(low[l]), words_simd_max(high[l])};
        if (lane == 0u) {
            for (uint k = 0; k < 4u; ++k) partial[subgroup][4u * l + k] = bounds[k];
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (tid >= 4u * lanes) return;
    const uint kind = tid % 4u;
    ulong bound = partial[0][tid];
    for (uint s = 1; s < 8u; ++s) {
        const ulong other = partial[s][tid];
        bound = kind == 0u ? bound | other : kind == 1u ? bound & other
            : kind == 2u ? min(bound, other) : max(bound, other);
    }
    device atomic_uint* word = control + 8u * (tid / 4u);
    if (kind == 0u) {
        words_fetch_or(word, uint(bound));
        words_fetch_or(word + 1u, uint(bound >> 32));
    } else if (kind == 1u) {
        words_fetch_and(word + 2u, uint(bound));
        words_fetch_and(word + 3u, uint(bound >> 32));
    } else if (kind == 2u) {
        words_fetch_min(reinterpret_cast<device atomic_ulong*>(word + 4u), bound);
    } else {
        words_fetch_max(reinterpret_cast<device atomic_ulong*>(word + 6u), bound);
    }
}

// Sorted bytes of `lane` under the caller's active-bit window.
inline uint words_lane_bytes(uint active_bits, uint lane) {
    return active_bits > lane * 64u ? min(8u, (active_bits - lane * 64u + 7u) / 8u) : 0u;
}

// args: lanes, active_bits. One thread writes the plan from the bounds.
kernel void words_sort_plan(
    const device uint* control [[buffer(0)]], device uint* plan [[buffer(1)]],
    constant uint* args [[buffer(2)]], uint i [[thread_position_in_grid]])
{
    if (i != 0u) return;
    uint in_b = 0u;
    for (uint lane = 0; lane < args[0]; ++lane) {
        const device uint* bounds = control + 8u * lane;
        const ulong varying = ulong(bounds[0] ^ bounds[2]) | (ulong(bounds[1] ^ bounds[3]) << 32);
        const ulong low = ulong(bounds[4]) | ulong(bounds[5]) << 32;
        const ulong span = (ulong(bounds[6]) | ulong(bounds[7]) << 32) - low;
        const uint bytes = words_lane_bytes(args[1], lane);
        const ulong window = bytes == 8u ? ~0ul : (1ul << (8u * bytes)) - 1ul;
        uint raw = 0u;
        for (uint byte = 0; byte < bytes; ++byte) raw += ((varying >> (8u * byte)) & 255ul) != 0ul;
        const uint relative = (64u - clz(span) + 7u) / 8u;
        const bool rebase = relative < raw;
        plan[kWordPlanBase + 2u * lane] = rebase ? uint(low) : 0u;
        plan[kWordPlanBase + 2u * lane + 1u] = rebase ? uint(low >> 32) : 0u;
        plan[kWordPlanGather + lane] = in_b | ((varying & window) != 0ul ? 0u : kWordSkip);
        for (uint byte = 0; byte < 8u; ++byte) {
            const bool sorted = rebase ? byte < relative
                : byte < bytes && ((varying >> (8u * byte)) & 255ul) != 0ul;
            plan[lane * 8u + byte] = in_b | (sorted ? 0u : kWordSkip);
            if (sorted) in_b ^= 1u;
        }
    }
    plan[kWordPlanFinal] = in_b;
}

inline ulong words_plan_base(const device uint* plan, uint pass) {
    const uint slot = kWordPlanBase + 2u * (pass / 8u);
    return ulong(plan[slot]) | ulong(plan[slot + 1u]) << 32;
}

// args: n, lanes, ms_first, lane (>= 1). In place: each row's sort key
// becomes `lane` of its key, in whichever array holds the current order.
kernel void words_sort_gather(
    const device ulong* keys [[buffer(0)]], device ulong* keys_a [[buffer(1)]],
    device uint* rows_a [[buffer(2)]], device ulong* keys_b [[buffer(3)]],
    device uint* rows_b [[buffer(4)]], const device uint* plan [[buffer(5)]],
    constant uint* args [[buffer(6)]], uint i [[thread_position_in_grid]])
{
    const uint step = plan[kWordPlanGather + args[3]];
    if (i >= args[0] || (step & kWordSkip) != 0u) return;
    const bool b = (step & kWordInB) != 0u;
    const uint row = (b ? rows_b : rows_a)[i];
    (b ? keys_b : keys_a)[i] = word_lane(keys, row, args[1], args[2], args[3]);
}

inline uint words_digit(ulong key, ulong origin, uint shift) {
    return uint((key - origin) >> shift) & 255u;
}

// Per-tile byte counts, digit-major for one flat exclusive scan (as
// sort_histogram_byte_impl over 64-bit sort keys less the lane base).
template<uint Items>
inline void words_sort_histogram_impl(const device ulong* keys, device uint* histogram,
    uint n, uint tiles, ulong origin, uint shift, uint tile, uint tid, uint lane, uint subgroup,
    threadgroup atomic_uint* counts)
{
    for (uint i = tid; i < 2048u; i += 256u)
        atomic_store_explicit(&counts[i], 0u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint item = 0; item < Items; ++item) {
        const uint index = tile * 256u * Items + item * 256u + tid;
        const uint digit = index < n ? words_digit(keys[index], origin, shift) : 256u;
        if (simd_all(digit == simd_broadcast_first(digit))) {
            if (lane == 0u && digit < 256u)
                atomic_fetch_add_explicit(&counts[subgroup * 256u + digit], 32u, memory_order_relaxed);
        } else if (digit < 256u) {
            atomic_fetch_add_explicit(&counts[subgroup * 256u + digit], 1u, memory_order_relaxed);
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint total = 0u;
    for (uint group = 0; group < 8u; ++group)
        total += atomic_load_explicit(&counts[group * 256u + tid], memory_order_relaxed);
    histogram[tid * tiles + tile] = total;
}

// One stable byte pass over a tile. Subgroup s owns the tile's rows
// [s * 32 * Items, (s + 1) * 32 * Items), so ranking items in order per
// subgroup is source order. A row's rank among equal digits of its subgroup
// comes from eight ballots (warp multisplit) plus the subgroup's running
// digit count; digit-major offsets over subgroups then place it in a
// digit-sorted staging tile, written out in digit runs.
template<uint Items>
inline void words_sort_scatter_impl(
    const device ulong* in_keys, const device uint* in_rows,
    device ulong* out_keys, device uint* out_rows, const device uint* histogram,
    uint n, uint tiles, ulong origin, uint shift, uint tile, uint tid, uint lane, uint subgroup,
    threadgroup uint* counts, threadgroup ulong* staged, threadgroup uint* starts,
    threadgroup uint* bases, threadgroup uint* scan)
{
    const uint begin = tile * (256u * Items);
    const uint count = min(256u * Items, n - begin);
    for (uint i = tid; i < 2048u; i += 256u) counts[i] = 0u;
    ulong keys[Items];
    uint rows[Items], ranks[Items];
    for (uint item = 0; item < Items; ++item) {
        const uint position = (subgroup * Items + item) * 32u + lane;
        keys[item] = position < count ? in_keys[begin + position] : 0ul;
        rows[item] = position < count ? in_rows[begin + position] : 0u;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    threadgroup uint* mine = counts + subgroup * 256u;
    const uint below = (1u << lane) - 1u;
    for (uint item = 0; item < Items; ++item) {
        const bool active = (subgroup * Items + item) * 32u + lane < count;
        const uint digit = words_digit(keys[item], origin, shift);
        uint peers = uint(static_cast<simd_vote::vote_t>(simd_ballot(active)));
        for (uint bit = 0; bit < 8u; ++bit) {
            const bool set = ((digit >> bit) & 1u) != 0u;
            const uint vote = uint(static_cast<simd_vote::vote_t>(simd_ballot(set)));
            peers &= set ? vote : ~vote;
        }
        const uint before = popcount(peers & below);
        const uint base = active ? mine[digit] : 0u;
        simdgroup_barrier(mem_flags::mem_threadgroup);
        if (active && before == 0u) mine[digit] = base + popcount(peers);
        simdgroup_barrier(mem_flags::mem_threadgroup);
        ranks[item] = base + before;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    // Thread tid owns digit tid: earlier subgroups' counts, the digit's tile
    // start and its global base (the scanned histogram).
    uint total = 0u;
    for (uint s = 0; s < 8u; ++s) {
        const uint digits = counts[s * 256u + tid];
        counts[s * 256u + tid] = total;
        total += digits;
    }
    uint tile_rows;
    starts[tid] = group_prefix(total, lane, subgroup, scan, tile_rows);
    bases[tid] = histogram[tid * tiles + tile];
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint item = 0; item < Items; ++item) {
        const uint digit = words_digit(keys[item], origin, shift);
        ranks[item] += starts[digit] + counts[subgroup * 256u + digit];
    }
    // Staged rows reuse the dead counts.
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint item = 0; item < Items; ++item) {
        if ((subgroup * Items + item) * 32u + lane < count) {
            staged[ranks[item]] = keys[item];
            counts[ranks[item]] = rows[item];
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint item = 0; item < Items; ++item) {
        const uint position = item * 256u + tid;
        if (position < count) {
            const ulong key = staged[position];
            const uint digit = words_digit(key, origin, shift);
            const uint target = bases[digit] + position - starts[digit];
            out_keys[target] = key;
            out_rows[target] = counts[position];
        }
    }
}

// args: n, tiles, pass (lane * 8 + byte).
#define WORDS_SORT_KERNELS(ITEMS) \
kernel void words_sort_histogram##ITEMS( \
    const device ulong* keys_a [[buffer(0)]], const device ulong* keys_b [[buffer(1)]], \
    device uint* histogram [[buffer(2)]], const device uint* plan [[buffer(3)]], \
    constant uint* args [[buffer(4)]], uint tile [[threadgroup_position_in_grid]], \
    uint tid [[thread_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]], \
    uint subgroup [[simdgroup_index_in_threadgroup]]) \
{ \
    threadgroup atomic_uint counts[2048]; \
    const uint step = plan[args[2]]; \
    if ((step & kWordSkip) != 0u) return; \
    words_sort_histogram_impl<ITEMS>((step & kWordInB) != 0u ? keys_b : keys_a, histogram, \
        args[0], args[1], words_plan_base(plan, args[2]), args[2] % 8u * 8u, tile, tid, lane, \
        subgroup, counts); \
} \
kernel void words_sort_scatter##ITEMS( \
    device ulong* keys_a [[buffer(0)]], device uint* rows_a [[buffer(1)]], \
    device ulong* keys_b [[buffer(2)]], device uint* rows_b [[buffer(3)]], \
    const device uint* histogram [[buffer(4)]], const device uint* plan [[buffer(5)]], \
    constant uint* args [[buffer(6)]], uint tile [[threadgroup_position_in_grid]], \
    uint tid [[thread_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]], \
    uint subgroup [[simdgroup_index_in_threadgroup]]) \
{ \
    threadgroup uint counts[2048]; \
    threadgroup ulong staged[256 * ITEMS]; \
    threadgroup uint starts[256], bases[256], scan[9]; \
    const uint step = plan[args[2]]; \
    if ((step & kWordSkip) != 0u) return; \
    const bool b = (step & kWordInB) != 0u; \
    words_sort_scatter_impl<ITEMS>(b ? keys_b : keys_a, b ? rows_b : rows_a, \
        b ? keys_a : keys_b, b ? rows_a : rows_b, histogram, args[0], args[1], \
        words_plan_base(plan, args[2]), args[2] % 8u * 8u, tile, tid, lane, subgroup, counts, \
        staged, starts, bases, scan); \
}
WORDS_SORT_KERNELS(4)
WORDS_SORT_KERNELS(8)
#undef WORDS_SORT_KERNELS

// The final stable order, and for one lane the sorted keys themselves.
struct WordsSorted {
    const device ulong* keys;
    const device uint* rows;
};

inline WordsSorted words_sorted(const device ulong* keys_a, const device uint* rows_a,
    const device ulong* keys_b, const device uint* rows_b, const device uint* plan)
{
    const bool b = (plan[kWordPlanFinal] & kWordInB) != 0u;
    return {b ? keys_b : keys_a, b ? rows_b : rows_a};
}

inline bool words_equal(const device ulong* keys, uint a, uint b, uint lanes) {
    for (uint m = 0; m < lanes; ++m) {
        if (keys[ulong(a) * lanes + m] != keys[ulong(b) * lanes + m]) return false;
    }
    return true;
}

// Whether stable position i starts a group. The sort keys hold one lane of
// every key (the last gathered), so they settle most heads in place; one
// lane needs nothing else, wider equal lanes compare every lane by row.
inline bool words_head(const device ulong* keys, WordsSorted sorted, uint i, uint lanes) {
    if (i == 0u || sorted.keys[i] != sorted.keys[i - 1u]) return true;
    return lanes != 1u && !words_equal(keys, sorted.rows[i], sorted.rows[i - 1u], lanes);
}

// args: n, lanes. flags[i] marks a group head in stable order; flags[n] = 0
// so the exclusive scan leaves the group count at prefix[n].
kernel void words_group_flags(
    const device ulong* keys [[buffer(0)]], const device ulong* keys_a [[buffer(1)]],
    const device uint* rows_a [[buffer(2)]], const device ulong* keys_b [[buffer(3)]],
    const device uint* rows_b [[buffer(4)]], const device uint* plan [[buffer(5)]],
    device uint* flags [[buffer(6)]], constant uint* args [[buffer(7)]],
    uint i [[thread_position_in_grid]])
{
    const uint n = args[0];
    if (i > n) return;
    if (i == n) { flags[i] = 0u; return; }
    flags[i] = uint(words_head(keys, words_sorted(keys_a, rows_a, keys_b, rows_b, plan), i, args[1]));
}

// args: n, lanes. Distinct keys in order, group start offsets with the
// terminal n, the stable permutation, and meta[0] = group count.
template<typename O>
inline void words_group_scatter_impl(
    const device ulong* keys, WordsSorted sorted, const device uint* prefix,
    device ulong* distinct, device O* offsets, device uint* permutation,
    device ulong* meta, constant uint* args, uint i)
{
    const uint n = args[0], lanes = args[1];
    if (i == 0u) {
        meta[0] = prefix[n];
        meta[1] = n;
        offsets[prefix[n]] = O(n);
    }
    if (i >= n) return;
    const uint row = sorted.rows[i];
    permutation[i] = row;
    if (prefix[i + 1u] == prefix[i]) return;
    const uint slot = prefix[i];
    offsets[slot] = O(i);
    if (lanes == 1u) {
        distinct[slot] = sorted.keys[i];
        return;
    }
    for (uint m = 0; m < lanes; ++m) {
        distinct[ulong(slot) * lanes + m] = keys[ulong(row) * lanes + m];
    }
}

#define WORDS_GROUP_SCATTER(NAME, O) \
kernel void NAME( \
    const device ulong* keys [[buffer(0)]], const device ulong* keys_a [[buffer(1)]], \
    const device uint* rows_a [[buffer(2)]], const device ulong* keys_b [[buffer(3)]], \
    const device uint* rows_b [[buffer(4)]], const device uint* plan [[buffer(5)]], \
    const device uint* prefix [[buffer(6)]], device ulong* distinct [[buffer(7)]], \
    device O* offsets [[buffer(8)]], device uint* permutation [[buffer(9)]], \
    device ulong* meta [[buffer(10)]], constant uint* args [[buffer(11)]], \
    uint i [[thread_position_in_grid]]) \
{ \
    words_group_scatter_impl<O>(keys, words_sorted(keys_a, rows_a, keys_b, rows_b, plan), \
        prefix, distinct, offsets, permutation, meta, args, i); \
}
WORDS_GROUP_SCATTER(words_group_scatter_o32, uint)
WORDS_GROUP_SCATTER(words_group_scatter_o64, ulong)
#undef WORDS_GROUP_SCATTER

// Lane-wise `a - b` for a >= b, as CUDA device::difference.
inline void words_difference(thread ulong* out, thread const ulong* a, thread const ulong* b, uint lanes) {
    ulong borrow = 0;
    for (uint lane = 0; lane < lanes; ++lane) {
        const ulong partial = a[lane] - b[lane];
        const ulong first = a[lane] < b[lane] ? 1ul : 0ul;
        out[lane] = partial - borrow;
        const ulong second = partial < borrow ? 1ul : 0ul;
        borrow = first | second;
    }
}

inline uint words_significant_bits(thread const ulong* word, uint lanes) {
    for (uint lane = lanes; lane-- > 0;) {
        if (word[lane] != 0ul) return lane * 64u + 64u - clz(word[lane]);
    }
    return 0u;
}

inline ulong words_extract(thread const ulong* word, uint lanes, uint shift, uint bits) {
    const uint lane = shift / 64u, offset = shift % 64u;
    ulong value = lane < lanes ? word[lane] >> offset : 0ul;
    if (offset != 0u && bits > 64u - offset && lane + 1u < lanes) {
        value |= word[lane + 1u] << (64u - offset);
    }
    return bits == 0u ? 0ul : value & (~0ul >> (64u - bits));
}

inline void words_load(thread ulong* out, const device ulong* keys, uint slot, uint lanes, uint ms_first) {
    for (uint lane = 0; lane < lanes; ++lane) out[lane] = word_lane(keys, slot, lanes, ms_first, lane);
}

// CUDA device::prefix: the top radix_bits bits of key - min at the run
// span's resolution, monotone in key.
inline ulong words_prefix(thread const ulong* key, thread const ulong* min,
    uint shift, uint radix_bits, uint lanes)
{
    ulong difference[kMaxWordLanes];
    words_difference(difference, key, min, lanes);
    return words_extract(difference, lanes, shift, radix_bits);
}

// args: lanes, ms_first, radix_bits; meta[0] is the group count.
// directory[bucket] is the first slot whose key prefix is at least bucket.
template<typename O>
inline void words_directory_impl(
    const device ulong* keys, const device ulong* meta, device O* directory,
    constant uint* args, uint bucket)
{
    const uint lanes = args[0], ms_first = args[1], radix_bits = args[2];
    const ulong entries = (1ul << radix_bits) + 1ul;
    if (bucket >= entries) return;
    const ulong len = meta[0];
    ulong min[kMaxWordLanes], max[kMaxWordLanes], span[kMaxWordLanes], key[kMaxWordLanes];
    words_load(min, keys, 0u, lanes, ms_first);
    words_load(max, keys, uint(len - 1ul), lanes, ms_first);
    words_difference(span, max, min, lanes);
    const uint significant = words_significant_bits(span, lanes);
    const uint shift = significant > radix_bits ? significant - radix_bits : 0u;
    ulong low = 0ul, high = len;
    while (low < high) {
        const ulong middle = low + (high - low) / 2ul;
        words_load(key, keys, uint(middle), lanes, ms_first);
        if (words_prefix(key, min, shift, radix_bits, lanes) < ulong(bucket)) low = middle + 1ul;
        else high = middle;
    }
    directory[bucket] = O(low);
}

#define WORDS_DIRECTORY(NAME, O) \
kernel void NAME( \
    const device ulong* keys [[buffer(0)]], const device ulong* meta [[buffer(1)]], \
    device O* directory [[buffer(2)]], constant uint* args [[buffer(3)]], \
    uint bucket [[thread_position_in_grid]]) \
{ \
    words_directory_impl<O>(keys, meta, directory, args, bucket); \
}
WORDS_DIRECTORY(words_directory_o32, uint)
WORDS_DIRECTORY(words_directory_o64, ulong)
#undef WORDS_DIRECTORY

// Item columns through a stable order: out[i] = in[order[i]]. args: n.
// Columns are moved as 4- or 8-byte words whatever their signedness.
#define WORDS_GATHER(NAME, T) \
kernel void NAME( \
    const device T* in [[buffer(0)]], const device uint* order [[buffer(1)]], \
    device T* out [[buffer(2)]], constant uint& n [[buffer(3)]], \
    uint i [[thread_position_in_grid]]) \
{ \
    if (i < n) out[i] = in[order[i]]; \
}
WORDS_GATHER(words_gather_b4, uint)
WORDS_GATHER(words_gather_b8, ulong)
#undef WORDS_GATHER

// Unsigned multi-word order of key i of a against key j of b.
inline int words_compare(const device ulong* a, uint i, const device ulong* b, uint j,
    uint lanes, uint ms_first)
{
    for (uint lane = lanes; lane-- > 0;) {
        const ulong x = word_lane(a, i, lanes, ms_first, lane);
        const ulong y = word_lane(b, j, lanes, ms_first, lane);
        if (x != y) return x < y ? -1 : 1;
    }
    return 0;
}

// Merge slots record their source run in the top bit; slot indices < 2^31.
constant uint kNewerSlot = 0x80000000u;

// args: older keys, newer keys, lanes, ms_first, tile, older rows, newer
// rows. Stable merge-path over two runs' distinct keys, ties older first;
// each thread owns `tile` consecutive slots. Slot s records its source,
// the occurrences before it (positions, strictly increasing: every group is
// nonempty) and in flags whether it starts a merged key. Older slots never
// repeat (an equal newer key follows them), so only a newer slot equal to
// the preceding older key is dropped. flags[slots] = 0 leaves the merged key
// count at the exclusive prefix[slots]; positions[slots] is the row total.
template<typename O>
inline void words_merge_slots_impl(
    const device ulong* older, const device O* older_offsets,
    const device ulong* newer, const device O* newer_offsets,
    device uint* tags, device O* positions, device uint* flags,
    constant uint* args, uint thread_id)
{
    const uint na = args[0], nb = args[1], lanes = args[2], ms_first = args[3], tile = args[4];
    const uint slots = na + nb;
    if (thread_id == 0u) {
        positions[slots] = O(ulong(args[5]) + ulong(args[6]));
        flags[slots] = 0u;
    }
    if (ulong(thread_id) * tile >= slots) return;
    const uint start = thread_id * tile, end = min(slots, start + tile);
    uint low = start > nb ? start - nb : 0u, high = min(start, na);
    while (low < high) {
        const uint middle = low + (high - low) / 2u;
        if (words_compare(older, middle, newer, start - middle - 1u, lanes, ms_first) <= 0) {
            low = middle + 1u;
        } else {
            high = middle;
        }
    }
    uint a = low, b = start - low;
    // Whether older[a - 1] equals newer[b]: the next newer slot repeats it.
    bool equal = a > 0u && b < nb && words_compare(older, a - 1u, newer, b, lanes, ms_first) == 0;
    for (uint slot = start; slot < end; ++slot) {
        positions[slot] = O(ulong(older_offsets[a]) + ulong(newer_offsets[b]));
        const int order = a == na ? 1 : b == nb ? -1 : words_compare(older, a, newer, b, lanes, ms_first);
        if (order <= 0) {
            tags[slot] = a++;
            flags[slot] = 1u;
            equal = order == 0;
        } else {
            tags[slot] = b++ | kNewerSlot;
            flags[slot] = equal ? 0u : 1u;
            equal = false;
        }
    }
}

#define WORDS_MERGE_SLOTS(NAME, O) \
kernel void NAME( \
    const device ulong* older [[buffer(0)]], const device O* older_offsets [[buffer(1)]], \
    const device ulong* newer [[buffer(2)]], const device O* newer_offsets [[buffer(3)]], \
    device uint* tags [[buffer(4)]], device O* positions [[buffer(5)]], \
    device uint* flags [[buffer(6)]], constant uint* args [[buffer(7)]], \
    uint i [[thread_position_in_grid]]) \
{ \
    words_merge_slots_impl<O>(older, older_offsets, newer, newer_offsets, tags, positions, flags, args, i); \
}
WORDS_MERGE_SLOTS(words_merge_slots_o32, uint)
WORDS_MERGE_SLOTS(words_merge_slots_o64, ulong)
#undef WORDS_MERGE_SLOTS

// args: slots, lanes. Compacts the merged keys and their group offsets with
// the terminal row total; meta = {key_count, rows}.
template<typename O>
inline void words_merge_scatter_impl(
    const device ulong* older, const device ulong* newer, const device uint* tags,
    const device O* positions, const device uint* prefix, device ulong* distinct,
    device O* offsets, device ulong* meta, constant uint* args, uint i)
{
    const uint slots = args[0], lanes = args[1];
    if (i == 0u) {
        meta[0] = prefix[slots];
        meta[1] = ulong(positions[slots]);
        offsets[prefix[slots]] = positions[slots];
    }
    if (i >= slots || prefix[i + 1u] == prefix[i]) return;
    const uint slot = prefix[i], tag = tags[i];
    offsets[slot] = positions[i];
    const device ulong* key = (tag & kNewerSlot) != 0u ? newer : older;
    const ulong row = ulong(tag & ~kNewerSlot) * lanes;
    for (uint m = 0; m < lanes; ++m) distinct[ulong(slot) * lanes + m] = key[row + m];
}

#define WORDS_MERGE_SCATTER(NAME, O) \
kernel void NAME( \
    const device ulong* older [[buffer(0)]], const device ulong* newer [[buffer(1)]], \
    const device uint* tags [[buffer(2)]], const device O* positions [[buffer(3)]], \
    const device uint* prefix [[buffer(4)]], device ulong* distinct [[buffer(5)]], \
    device O* offsets [[buffer(6)]], device ulong* meta [[buffer(7)]], \
    constant uint* args [[buffer(8)]], uint i [[thread_position_in_grid]]) \
{ \
    words_merge_scatter_impl<O>(older, newer, tags, positions, prefix, distinct, offsets, meta, args, i); \
}
WORDS_MERGE_SCATTER(words_merge_scatter_o32, uint)
WORDS_MERGE_SCATTER(words_merge_scatter_o64, ulong)
#undef WORDS_MERGE_SCATTER

// args: slots, rows, older rows. sources[p] is merged occurrence p's index
// in the concatenation older ++ newer: its slot is the last whose position
// is at most p, and it keeps its rank within that source group.
template<typename O>
inline void words_merge_sources_impl(
    const device O* older_offsets, const device O* newer_offsets, const device uint* tags,
    const device O* positions, device uint* sources, constant uint* args, uint p)
{
    const uint slots = args[0];
    if (p >= args[1]) return;
    uint low = 0u, high = slots;
    while (low < high) {
        const uint middle = low + (high - low) / 2u;
        if (ulong(positions[middle]) <= ulong(p)) low = middle + 1u;
        else high = middle;
    }
    const uint slot = low - 1u, tag = tags[slot];
    const ulong base = (tag & kNewerSlot) != 0u
        ? ulong(args[2]) + ulong(newer_offsets[tag & ~kNewerSlot])
        : ulong(older_offsets[tag]);
    sources[p] = uint(base + ulong(p) - ulong(positions[slot]));
}

#define WORDS_MERGE_SOURCES(NAME, O) \
kernel void NAME( \
    const device O* older_offsets [[buffer(0)]], const device O* newer_offsets [[buffer(1)]], \
    const device uint* tags [[buffer(2)]], const device O* positions [[buffer(3)]], \
    device uint* sources [[buffer(4)]], constant uint* args [[buffer(5)]], \
    uint p [[thread_position_in_grid]]) \
{ \
    words_merge_sources_impl<O>(older_offsets, newer_offsets, tags, positions, sources, args, p); \
}
WORDS_MERGE_SOURCES(words_merge_sources_o32, uint)
WORDS_MERGE_SOURCES(words_merge_sources_o64, ulong)
#undef WORDS_MERGE_SOURCES

// args: rows, older rows, newer bias. Gathers a column through the merge
// sources; the bias rebases newer stable-order rows after the older rows.
#define WORDS_MERGE_ITEMS(NAME, T) \
kernel void NAME( \
    const device T* older [[buffer(0)]], const device T* newer [[buffer(1)]], \
    const device uint* sources [[buffer(2)]], device T* out [[buffer(3)]], \
    constant uint* args [[buffer(4)]], uint p [[thread_position_in_grid]]) \
{ \
    if (p >= args[0]) return; \
    const uint source = sources[p]; \
    out[p] = source < args[1] ? older[source] : T(newer[source - args[1]] + T(args[2])); \
}
WORDS_MERGE_ITEMS(words_merge_b4, uint)
WORDS_MERGE_ITEMS(words_merge_b8, ulong)
#undef WORDS_MERGE_ITEMS
