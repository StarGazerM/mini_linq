// CUDA DeviceRadixRun layout on Metal (GPU-RadixRun, STORAGE §8.2): packed
// keys of `lanes` 64-bit words, ordered most-significant lane last in lane
// numbering. Memory word m of a key holds lane (ms_first ? lanes-1-m : m),
// the byte layout of CUDA's u64/u128 (ms_first = 0) and [u64; L] (= 1).
// Row and metal_stdlib are supplied by common.metal.

constant uint kMaxWordLanes = 8;

inline ulong word_lane(const device ulong* keys, uint row, uint lanes, uint ms_first, uint lane) {
    return keys[ulong(row) * lanes + (ms_first != 0u ? lanes - 1u - lane : lane)];
}

// args: n, lanes, ms_first, slice (32-bit slice of the key, 0 = least
// significant), first. A stable sort pass of these rows, LSD over slices,
// yields the stable key order; `first` reads identity row ids.
kernel void words_gather_slice(
    const device ulong* keys [[buffer(0)]], const device Row* order [[buffer(1)]],
    device Row* out [[buffer(2)]], constant uint* args [[buffer(3)]],
    uint i [[thread_position_in_grid]])
{
    const uint n = args[0];
    if (i >= n) return;
    const uint row = args[4] != 0u ? i : order[i].value;
    const ulong lane = word_lane(keys, row, args[1], args[2], args[3] / 2u);
    out[i] = Row{uint(lane >> (32u * (args[3] & 1u))), row};
}

inline bool words_equal(const device ulong* keys, uint a, uint b, uint lanes) {
    for (uint m = 0; m < lanes; ++m) {
        if (keys[ulong(a) * lanes + m] != keys[ulong(b) * lanes + m]) return false;
    }
    return true;
}

// args: n, lanes. flags[i] marks a group head in stable order; flags[n] = 0
// so the exclusive scan leaves the group count at prefix[n].
kernel void words_group_flags(
    const device ulong* keys [[buffer(0)]], const device Row* order [[buffer(1)]],
    device uint* flags [[buffer(2)]], constant uint* args [[buffer(3)]],
    uint i [[thread_position_in_grid]])
{
    const uint n = args[0];
    if (i > n) return;
    if (i == n) { flags[i] = 0u; return; }
    flags[i] = uint(i == 0u || !words_equal(keys, order[i].value, order[i - 1u].value, args[1]));
}

// args: n, lanes. Distinct keys in order, group start offsets with the
// terminal n, the stable permutation, and meta[0] = group count.
template<typename O>
inline void words_group_scatter_impl(
    const device ulong* keys, const device Row* order, const device uint* prefix,
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
    const uint row = order[i].value;
    permutation[i] = row;
    if (prefix[i + 1u] == prefix[i]) return;
    const uint slot = prefix[i];
    offsets[slot] = O(i);
    for (uint m = 0; m < lanes; ++m) {
        distinct[ulong(slot) * lanes + m] = keys[ulong(row) * lanes + m];
    }
}

#define WORDS_GROUP_SCATTER(NAME, O) \
kernel void NAME( \
    const device ulong* keys [[buffer(0)]], const device Row* order [[buffer(1)]], \
    const device uint* prefix [[buffer(2)]], device ulong* distinct [[buffer(3)]], \
    device O* offsets [[buffer(4)]], device uint* permutation [[buffer(5)]], \
    device ulong* meta [[buffer(6)]], constant uint* args [[buffer(7)]], \
    uint i [[thread_position_in_grid]]) \
{ \
    words_group_scatter_impl<O>(keys, order, prefix, distinct, offsets, permutation, meta, args, i); \
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
