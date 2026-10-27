// Shared relation inputs are immutable. Only selected values are narrowed;
// stable exclusive mask ranks preserve original row order before radix sorting.
kernel void relation_selection_flags(
    const device uint* selection [[buffer(0)]],
    device uint* flags [[buffer(1)]],
    constant uint& n [[buffer(2)]],
    uint row [[thread_position_in_grid]])
{
    if (row < n) flags[row] = uint(selection[row] != 0u);
}

// metadata = { selected_count, first_invalid_source_row }. The sentinel is
// outside the accepted source-row domain, including for an empty relation.
kernel void relation_metadata(
    const device uint* selected_total [[buffer(0)]],
    device uint* metadata [[buffer(1)]],
    constant uint* args [[buffer(2)]],
    uint tid [[thread_position_in_grid]])
{
    if (tid == 0) {
        metadata[0] = args[1] != 0u && args[0] != 0u ? selected_total[0] : args[0];
        metadata[1] = 0xffffffffu;
    }
}

inline bool relation_value_fits(uint value) { return true; }
inline bool relation_value_fits(long value) {
    return value >= 0 && value <= long(0xffffffffu);
}

template<typename T>
inline void relation_project_impl(
    const device T* relation, const device uint* selection,
    const device uint* offsets, device Row* output,
    device atomic_uint* metadata, constant uint* args, uint row)
{
    // args = {source_rows, columns, key_column, payload_column, has_selection}.
    // UINT_MAX payload_column means original source row ID, never compact rank.
    if (row >= args[0] || (args[4] != 0u && selection[row] == 0u)) return;
    const ulong base = ulong(row) * ulong(args[1]);
    const T key = relation[base + args[2]];
    const T value = args[3] == 0xffffffffu ? T(row) : relation[base + args[3]];
    if (!relation_value_fits(key) || !relation_value_fits(value)) {
        atomic_fetch_min_explicit(&metadata[1], row, memory_order_relaxed);
    }
    const uint destination = args[4] != 0u ? offsets[row] : row;
    output[destination] = Row{uint(key), uint(value)};
}

#define RELATION_PROJECT_KERNEL(NAME, TYPE) \
kernel void relation_project_##NAME( \
    const device TYPE* relation [[buffer(0)]], const device uint* selection [[buffer(1)]], \
    const device uint* offsets [[buffer(2)]], device Row* output [[buffer(3)]], \
    device atomic_uint* metadata [[buffer(4)]], constant uint* args [[buffer(5)]], \
    uint row [[thread_position_in_grid]]) \
{ \
    relation_project_impl<TYPE>(relation, selection, offsets, output, metadata, args, row); \
}
RELATION_PROJECT_KERNEL(u32, uint)
RELATION_PROJECT_KERNEL(i64, long)
#undef RELATION_PROJECT_KERNEL
