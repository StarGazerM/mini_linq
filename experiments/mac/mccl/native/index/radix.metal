// Grouped RadixRun construction. All counts and prefixes remain on the GPU.
// Final metadata: rows, distinct keys, points, min, max, shift, error, bad row.
constant uint radix_chunk_size = 256u;
// A corridor cannot fail within 32 ranks of its anchor: its upper ray has
// dy >= 33 and its lower ray is nonpositive over that prefix. Each emitted
// nonterminal anchor advances >=32 ranks, hence at most 9 points per chunk.
constant uint radix_chunk_points = 9u;

kernel void radix_group_flags(
    const device Row* rows [[buffer(0)]], const device uint* source_meta [[buffer(1)]],
    device uint* flags [[buffer(2)]], constant uint& capacity [[buffer(3)]],
    uint i [[thread_position_in_grid]])
{
    if (i < capacity) {
        flags[i] = uint(i < source_meta[0] && (i == 0u || rows[i - 1u].key != rows[i].key));
    }
}

kernel void radix_group_metadata(
    const device uint* source_meta [[buffer(0)]], const device uint* total [[buffer(1)]],
    device ulong* meta [[buffer(2)]], constant uint* args [[buffer(3)]],
    uint i [[thread_position_in_grid]])
{
    if (i == 0u) {
        meta[0] = args[1] != 0u ? args[2] : source_meta[0];
        meta[1] = args[0] != 0u ? total[0] : 0u;
        meta[2] = 0u;
        meta[3] = 0u;
        meta[4] = 0u;
        meta[5] = 0u;
        meta[6] = meta[1] != 0u ? 33u : 32u;
        meta[7] = source_meta[1];
    }
}

kernel void radix_group_scatter(
    const device Row* rows [[buffer(0)]], const device uint* source_meta [[buffer(1)]],
    const device uint* ranks [[buffer(2)]], const device ulong* meta [[buffer(3)]],
    device uint* keys [[buffer(4)]], device ulong* offsets [[buffer(5)]],
    device uint* items [[buffer(6)]], constant uint& merging [[buffer(7)]],
    uint i [[thread_position_in_grid]])
{
    if (i == 0u) offsets[meta[1]] = meta[0];
    if (i >= source_meta[0]) return;
    if (merging == 0u) items[i] = rows[i].value;
    if (i == 0u || rows[i - 1u].key != rows[i].key) {
        keys[ranks[i]] = rows[i].key;
        offsets[ranks[i]] = merging != 0u ? rows[i].value : i;
    }
}

// Positive is clockwise, matching the native greedy corridor. Key differences
// fit u32; rank differences are chunk-local (+/-32), so signed 64-bit products
// are exact even for keys spanning the entire u32 domain.
inline long radix_orientation(uint ax, long ay, uint bx, long by) {
    return ay * long(bx) - by * long(ax);
}

kernel void radix_spline_chunks(
    const device uint* keys [[buffer(0)]], const device ulong* meta [[buffer(1)]],
    device ulong2* scratch [[buffer(2)]], device uint* counts [[buffer(3)]],
    constant uint& chunks [[buffer(4)]], uint chunk [[thread_position_in_grid]])
{
    if (chunk >= chunks) return;
    const uint first = chunk * radix_chunk_size;
    const uint end = min(first + radix_chunk_size, uint(meta[1]));
    uint count = 0u;
    if (first < end) {
        uint anchor = first;
        scratch[chunk * radix_chunk_points + count++] = ulong2(keys[first], first);
        if (first + 1u < end) {
            uint previous = first + 1u;
            uint upper = previous;
            uint lower = previous;
            for (uint rank = first + 2u; rank < end; ++rank) {
                const uint origin = keys[anchor];
                const uint ux = keys[upper] - origin;
                const uint lx = keys[lower] - origin;
                const uint cx = keys[rank] - origin;
                const long uy = long(upper) - long(anchor) + 32;
                const long ly = long(lower) - long(anchor) - long(min(lower, 32u));
                const long cy = long(rank) - long(anchor);
                if (radix_orientation(ux, uy, cx, cy) <= 0 ||
                    radix_orientation(lx, ly, cx, cy) >= 0) {
                    scratch[chunk * radix_chunk_points + count++] = ulong2(keys[previous], previous);
                    anchor = previous;
                    upper = rank;
                    lower = rank;
                } else {
                    if (radix_orientation(ux, uy, cx, cy + 32) > 0) upper = rank;
                    if (radix_orientation(lx, ly, cx, cy - long(min(rank, 32u))) < 0) lower = rank;
                }
                previous = rank;
            }
            scratch[chunk * radix_chunk_points + count++] = ulong2(keys[previous], previous);
        }
    }
    counts[chunk] = count;
}

kernel void radix_spline_metadata(
    const device uint* keys [[buffer(0)]], const device uint* total [[buffer(1)]],
    device ulong* meta [[buffer(2)]], constant uint& chunks [[buffer(3)]],
    uint i [[thread_position_in_grid]])
{
    if (i == 0u) {
        meta[2] = chunks != 0u ? total[0] : 0u;
        if (meta[1] != 0u) {
            meta[3] = keys[0];
            meta[4] = keys[meta[1] - 1u];
            const uint span = uint(meta[4] - meta[3]);
            const uint bits = span == 0u ? 0u : 32u - clz(span);
            meta[5] = bits > 12u ? bits - 12u : 0u;
        }
    }
}

kernel void radix_spline_compact(
    const device ulong2* scratch [[buffer(0)]], const device uint* ranks [[buffer(1)]],
    const device ulong* meta [[buffer(2)]], device ulong2* points [[buffer(3)]],
    constant uint& chunks [[buffer(4)]], uint chunk [[thread_position_in_grid]])
{
    if (chunk >= chunks) return;
    const uint begin = ranks[chunk];
    const uint end = chunk + 1u < chunks ? ranks[chunk + 1u] : uint(meta[2]);
    for (uint i = begin; i < end; ++i) points[i] = scratch[chunk * radix_chunk_points + i - begin];
}

// Removing only exactly collinear interior points preserves the piecewise
// linear function, including across chunk boundaries. Simultaneously removing
// a chain is safe because every adjacent slope in that chain is identical.
// Products fit u64: key differences are <=UINT32_MAX, ranks <=1<<30.
kernel void radix_spline_keep_flags(
    const device ulong2* points [[buffer(0)]], const device ulong* meta [[buffer(1)]],
    device uint* flags [[buffer(2)]], constant uint& capacity [[buffer(3)]],
    uint i [[thread_position_in_grid]])
{
    if (i >= capacity) return;
    const uint count = uint(meta[2]);
    bool keep = i < count;
    if (keep && i != 0u && i + 1u < count) {
        const ulong2 previous = points[i - 1u];
        const ulong2 current = points[i];
        const ulong2 next = points[i + 1u];
        keep = (current.x - previous.x) * (next.y - current.y) !=
               (next.x - current.x) * (current.y - previous.y);
    }
    flags[i] = uint(keep);
}

kernel void radix_spline_coalesce(
    const device ulong2* points [[buffer(0)]], const device ulong* meta [[buffer(1)]],
    const device uint* ranks [[buffer(2)]], const device uint* total [[buffer(3)]],
    device ulong2* scratch [[buffer(4)]], uint i [[thread_position_in_grid]])
{
    const uint count = uint(meta[2]);
    if (i >= count) return;
    const uint begin = ranks[i];
    const uint end = i + 1u < count ? ranks[i + 1u] : total[0];
    if (begin != end) scratch[begin] = points[i];
}

// Keep the published output's allocation and ownership unchanged. All reads
// here are from scratch/total, so updating the point count cannot race readers
// of the previous count in this dispatch.
kernel void radix_spline_coalesced_copy(
    const device ulong2* scratch [[buffer(0)]], const device uint* total [[buffer(1)]],
    device ulong2* points [[buffer(2)]], device ulong* meta [[buffer(3)]],
    uint i [[thread_position_in_grid]])
{
    const uint count = total[0];
    if (i < count) points[i] = scratch[i];
    if (i == 0u) meta[2] = count;
}

kernel void radix_spline_directory(
    const device ulong2* points [[buffer(0)]], const device ulong* meta [[buffer(1)]],
    device ulong* directory [[buffer(2)]], uint prefix [[thread_position_in_grid]])
{
    if (prefix >= 4098u) return;
    uint lo = 0u;
    uint hi = uint(meta[2]);
    while (lo < hi) {
        const uint mid = lo + (hi - lo) / 2u;
        const ulong p = (points[mid].x - meta[3]) >> uint(meta[5]);
        if (p < prefix) lo = mid + 1u;
        else hi = mid;
    }
    directory[prefix] = lo;
}

// Merge the DISTINCT key lists directly. Equal-key candidates are adjacent;
// their payload starts describe the old-before-new concatenation. The only
// temporary pairs here are per-key candidates, never occurrence key/payloads.
kernel void radix_merge_keys(
    const device uint* own_keys [[buffer(0)]], const device ulong* own_offsets [[buffer(1)]],
    const device uint* other_keys [[buffer(2)]], const device ulong* other_offsets [[buffer(3)]],
    device Row* candidates [[buffer(4)]], device uint* deltas [[buffer(5)]],
    constant uint* args [[buffer(6)]], uint i [[thread_position_in_grid]])
{
    if (i >= args[0]) return;
    const uint key = own_keys[i];
    uint lo = 0u;
    uint hi = args[1];
    while (lo < hi) {
        const uint mid = lo + (hi - lo) / 2u;
        if (other_keys[mid] < key || (args[2] != 0u && other_keys[mid] == key)) lo = mid + 1u;
        else hi = mid;
    }
    const uint delta = uint(other_offsets[lo]);
    deltas[i] = delta;
    candidates[i + lo] = Row{key, uint(own_offsets[i]) + delta};
}

kernel void radix_merge_items(
    const device ulong* offsets [[buffer(0)]], const device uint* items [[buffer(1)]],
    const device uint* deltas [[buffer(2)]], device uint* output [[buffer(3)]],
    constant uint* args [[buffer(4)]], uint row [[thread_position_in_grid]])
{
    if (row >= args[0]) return;
    uint lo = 0u;
    uint hi = args[1];
    while (lo + 1u < hi) {
        const uint mid = lo + (hi - lo) / 2u;
        if (offsets[mid] <= row) lo = mid;
        else hi = mid;
    }
    output[row + deltas[lo]] = items[row];
}
