// Batched exact equi-join of probe keys against one completed grouped RadixRun.
//
// join_probe_counts finds each probe's distinct-key rank and its occurrence
// count; an exclusive u64 scan turns counts into output offsets, whose final
// entry is the exact total. join_expand then assigns a fixed 2048-output tile
// to every threadgroup, so one heavily duplicated key spreads across many
// threadgroups instead of serializing inside one probe's thread.

// args: probes, distinct keys. Thread `probes` writes the zero terminal count,
// so the exclusive scan leaves the exact total in offsets[probes].
kernel void join_probe_counts(
    const device uint* probes [[buffer(0)]], const device uint* keys [[buffer(1)]],
    const device ulong* offsets [[buffer(2)]], device uint* ranks [[buffer(3)]],
    device ulong* counts [[buffer(4)]], constant uint* args [[buffer(5)]],
    uint i [[thread_position_in_grid]])
{
    const uint n = args[0];
    if (i == n) counts[n] = 0ul;
    if (i >= n) return;
    const uint key = probes[i];
    uint lo = 0u;
    uint hi = args[1];
    while (lo < hi) {
        const uint mid = lo + (hi - lo) / 2u;
        if (keys[mid] < key) lo = mid + 1u;
        else hi = mid;
    }
    const bool hit = lo < args[1] && keys[lo] == key;
    ranks[i] = hit ? lo : UINT_MAX;
    counts[i] = hit ? offsets[lo + 1u] - offsets[lo] : 0ul;
}

// Exclusive scan of 1024 consecutive ulongs, four per thread. The 256 thread
// sums are scanned in threadgroup memory (Hillis-Steele), avoiding 64-bit SIMD
// reductions. Recursively scanning block_sums and adding them back yields an
// exact arbitrary-length scan; per-probe counts are <= 2^30, so no sum wraps.
kernel void join_scan_blocks_u64(
    device ulong* data [[buffer(0)]], device ulong* block_sums [[buffer(1)]],
    constant uint* args [[buffer(2)]], uint block [[threadgroup_position_in_grid]],
    uint tid [[thread_index_in_threadgroup]])
{
    threadgroup ulong sums[2][256];
    const uint begin = block * 1024u + tid * 4u;
    ulong values[4];
    ulong sum = 0ul;
    for (uint i = 0; i < 4; ++i) {
        values[i] = begin + i < args[0] ? data[begin + i] : 0ul;
        sum += values[i];
    }
    uint current = 0u;
    sums[current][tid] = sum;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint stride = 1u; stride < 256u; stride <<= 1) {
        const ulong value = sums[current][tid] + (tid >= stride ? sums[current][tid - stride] : 0ul);
        current ^= 1u;
        sums[current][tid] = value;
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
    ulong prefix = sums[current][tid] - sum;
    for (uint i = 0; i < 4; ++i) {
        if (begin + i < args[0]) data[begin + i] = prefix;
        prefix += values[i];
    }
    if (tid == 255u) block_sums[block] = sums[current][255];
}

kernel void join_add_block_offsets_u64(
    device ulong* data [[buffer(0)]], const device ulong* block_offsets [[buffer(1)]],
    constant uint* args [[buffer(2)]], uint block [[threadgroup_position_in_grid]],
    uint tid [[thread_index_in_threadgroup]])
{
    const ulong offset = block_offsets[block];
    for (uint i = 0; i < 4; ++i) {
        const uint index = block * 1024u + i * 256u + tid;
        if (index < args[0]) data[index] += offset;
    }
}

// The probe owning output g: the largest p < probes with prefix[p] <= g. The
// caller guarantees prefix[lo] <= g < prefix[hi].
inline uint join_owner(const device ulong* prefix, ulong g, uint lo, uint hi) {
    while (lo + 1u < hi) {
        const uint mid = lo + (hi - lo) / 2u;
        if (prefix[mid] <= g) lo = mid;
        else hi = mid;
    }
    return lo;
}

// args: total outputs (<= 2^30), probes. Outputs are {probe ordinal, item},
// ordered by probe ordinal and then by stored occurrence order.
kernel void join_expand(
    const device ulong* prefix [[buffer(0)]], const device uint* ranks [[buffer(1)]],
    const device ulong* offsets [[buffer(2)]], const device uint* items [[buffer(3)]],
    device uint2* pairs [[buffer(4)]], constant uint* args [[buffer(5)]],
    uint tile [[threadgroup_position_in_grid]], uint tid [[thread_index_in_threadgroup]])
{
    threadgroup uint bounds[2];
    const uint total = args[0];
    const uint first = tile * 2048u;
    const uint end = min(first + 2048u, total);
    if (tid < 2u) {
        bounds[tid] = join_owner(prefix, tid == 0u ? first : end - 1u, 0u, args[1]);
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint k = 0; k < 8u; ++k) {
        const uint g = first + k * 256u + tid;
        if (g >= end) break;
        const uint probe = join_owner(prefix, g, bounds[0], bounds[1] + 1u);
        const ulong item = offsets[ranks[probe]] + (ulong(g) - prefix[probe]);
        pairs[g] = uint2(probe, items[item]);
    }
}

// Semijoin filter of source row IDs against one grouped run, without leaving
// the GPU. Row IDs are read with a stride/offset, so join pairs ({probe, row})
// and compact u32 row sets share one kernel. A row is kept iff its int64 key
// column is in the u32 domain and present in the run; with an item column, the
// key's group must also contain that column's value (linear in group size).
//
// Row sets are capacity-sized buffers whose live length is count[0], written
// by the producing stage on the GPU, so chained stages need no host readback.
// Flags past the live length are zero, so prefix[capacity] is the kept count.
// args: capacity, row_stride, row_offset, columns, key_column, item_column
// (UINT_MAX: none), key_count, source_rows. invalid[0]: row ID outside.
kernel void join_semi_flags(
    const device uint* rows [[buffer(0)]], const device long* relation [[buffer(1)]],
    const device uint* keys [[buffer(2)]], const device ulong* offsets [[buffer(3)]],
    const device uint* items [[buffer(4)]], device uint* flags [[buffer(5)]],
    device uint* invalid [[buffer(6)]], constant uint* args [[buffer(7)]],
    const device uint* count [[buffer(8)]], uint i [[thread_position_in_grid]])
{
    if (i > args[0]) return;
    if (i >= count[0]) { flags[i] = 0u; return; }
    const uint row = rows[i * args[1] + args[2]];
    bool keep = false;
    if (row >= args[7]) {
        invalid[0] = 1u; // every writer stores the same value
    } else {
        const ulong base = ulong(row) * args[3];
        const long value = relation[base + args[4]];
        if (value >= 0 && value <= long(UINT_MAX)) {
            const uint key = uint(value);
            uint lo = 0u;
            uint hi = args[6];
            while (lo < hi) {
                const uint mid = lo + (hi - lo) / 2u;
                if (keys[mid] < key) lo = mid + 1u;
                else hi = mid;
            }
            if (lo < args[6] && keys[lo] == key) {
                if (args[5] == UINT_MAX) {
                    keep = true;
                } else {
                    const long wanted = relation[base + args[5]];
                    if (wanted >= 0 && wanted <= long(UINT_MAX)) {
                        for (ulong j = offsets[lo]; j < offsets[lo + 1u]; ++j) {
                            if (items[j] == uint(wanted)) { keep = true; break; }
                        }
                    }
                }
            }
        }
    }
    flags[i] = keep ? 1u : 0u;
}

// args: capacity, row_stride, row_offset. Kept rows retain their input order;
// kept[0] receives the live length of the output row set.
kernel void join_semi_scatter(
    const device uint* rows [[buffer(0)]], const device uint* prefix [[buffer(1)]],
    device uint* out [[buffer(2)]], constant uint* args [[buffer(3)]],
    device uint* kept [[buffer(4)]], uint i [[thread_position_in_grid]])
{
    if (i == 0u) kept[0] = prefix[args[0]];
    if (i >= args[0]) return;
    if (prefix[i + 1u] != prefix[i]) out[prefix[i]] = rows[i * args[1] + args[2]];
}

// Distinct full-width tuples of source rows, in ascending lexicographic order
// of up to four columns. Each column k occupies a bit field [lo_k, lo_k+w_k)
// of one composite key, the first column most significant; w_k is 32 unless
// the caller proves a smaller bound. LSD passes gather one 32-bit slice of the
// composite as the key of a {key, row} pair and stably sort only its live
// bytes; the first pass reads row IDs from the input set (stride/offset as
// above), later passes the previous pass's sorted payloads. Adjacent equal
// tuples are then flagged, scanned and scattered as full int64 rows.
// invalid[0]: a row ID outside the relation; invalid[1]: a value outside its
// field (beyond u32, or above the caller's bound).
kernel void distinct_clear(device uint* invalid [[buffer(0)]], uint i [[thread_position_in_grid]]) {
    if (i < 2u) invalid[i] = 0u;
}

// args: capacity, row_stride, row_offset, columns, source_rows, slice_lo,
// column_count, then {column, width, lo} per tuple column.
kernel void distinct_gather_key(
    const device uint* rows [[buffer(0)]], const device long* relation [[buffer(1)]],
    device uint2* pairs [[buffer(2)]], device uint* invalid [[buffer(3)]],
    constant uint* args [[buffer(4)]], const device uint* count [[buffer(5)]],
    uint i [[thread_position_in_grid]])
{
    if (i >= count[0]) return;
    const uint row = rows[i * args[1] + args[2]];
    uint key = 0u;
    if (row >= args[4]) {
        invalid[0] = 1u; // every writer stores the same value
    } else {
        for (uint k = 0; k < args[6]; ++k) {
            const uint width = args[8 + 3 * k];
            const long value = relation[ulong(row) * args[3] + args[7 + 3 * k]];
            const long limit = width == 32u ? long(UINT_MAX) : long((1u << width) - 1u);
            if (value < 0 || value > limit) { invalid[1] = 1u; continue; }
            // Position of this field's bit 0 relative to the slice's bit 0.
            const int shift = int(args[9 + 3 * k]) - int(args[5]);
            const ulong field = ulong(value);
            if (shift >= 0) { if (shift < 32) key |= uint(field << shift); }
            else if (-shift < 64) key |= uint(field >> (-shift));
        }
    }
    pairs[i] = uint2(key, row);
}

// args: capacity, columns, column_count, column[0..4).
inline bool distinct_equal(const device long* relation, uint a, uint b, constant uint* args) {
    for (uint k = 0; k < args[2]; ++k) {
        if (relation[ulong(a) * args[1] + args[3 + k]] !=
            relation[ulong(b) * args[1] + args[3 + k]]) return false;
    }
    return true;
}

kernel void distinct_flags(
    const device uint2* sorted [[buffer(0)]], const device long* relation [[buffer(1)]],
    device uint* flags [[buffer(2)]], constant uint* args [[buffer(3)]],
    const device uint* count [[buffer(4)]], uint i [[thread_position_in_grid]])
{
    if (i > args[0]) return;
    if (i >= count[0]) { flags[i] = 0u; return; }
    flags[i] = (i == 0u || !distinct_equal(relation, sorted[i].y, sorted[i - 1u].y, args)) ? 1u : 0u;
}

// distinct[0] receives the number of distinct tuples.
kernel void distinct_scatter(
    const device uint2* sorted [[buffer(0)]], const device uint* prefix [[buffer(1)]],
    const device long* relation [[buffer(2)]], device long* out [[buffer(3)]],
    constant uint* args [[buffer(4)]], device uint* distinct [[buffer(5)]],
    uint i [[thread_position_in_grid]])
{
    if (i == 0u) distinct[0] = prefix[args[0]];
    if (i >= args[0] || prefix[i + 1u] == prefix[i]) return;
    const ulong source = ulong(sorted[i].y) * args[1];
    const ulong target = ulong(prefix[i]) * args[2];
    for (uint k = 0; k < args[2]; ++k) out[target + k] = relation[source + args[3 + k]];
}
