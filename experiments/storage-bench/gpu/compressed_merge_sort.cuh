// Measurement-only O3/O4. Included after compressed_common.cuh in one TU.
// O3 uses 2^23 keys per input (2^24 total), preserving every duplicate.
// O4's 25% duplicate rate means (input_count - distinct_count)/input_count.
// All device allocations and CPU oracles are outside timed callbacks. The
// common bench supplies five warmups and 21 rotated, exactly checked samples.
namespace compressed_merge_sort {
constexpr unsigned tile_size = 1024;

__device__ unsigned frame_width(Code range, int kind) {
    unsigned w = range ? 32 - __clz(range) : 0;
    return kind == 2 ? (w <= 8 ? 8 : w <= 16 ? 16 : 32) : w;
}

// Stable merge-path: equal keys from A precede equal keys from B. The strict
// inequality in the second partition condition is essential for bag semantics.
__device__ size_t partition(RunView a, RunView b, size_t diagonal) {
    size_t lo = diagonal > b.n ? diagonal - b.n : 0;
    size_t hi = diagonal < a.n ? diagonal : a.n;
    while (lo <= hi) {
        size_t i = lo + (hi - lo) / 2, j = diagonal - i;
        if (i && j < b.n && code_at(a, i - 1) > code_at(b, j)) hi = i - 1;
        else if (j && i < a.n && code_at(b, j - 1) >= code_at(a, i)) lo = i + 1;
        else return i;
    }
    return lo;
}

__device__ Code next_code(RunView a, RunView b, size_t& i, size_t& j) {
    if (i < a.n && (j == b.n || code_at(a, i) <= code_at(b, j)))
        return code_at(a, i++);
    return code_at(b, j++);
}

template<class T>
__global__ void plain_merge(const T* a, size_t na, const T* b, size_t nb, T* out) {
    size_t diagonal = size_t(blockIdx.x) * tile_size + threadIdx.x * 4;
    if (diagonal >= na + nb) return;
    size_t lo = diagonal > nb ? diagonal - nb : 0;
    size_t hi = diagonal < na ? diagonal : na;
    size_t i = 0, j = 0;
    while (lo <= hi) {
        i = lo + (hi - lo) / 2; j = diagonal - i;
        if (i && j < nb && a[i - 1] > b[j]) hi = i - 1;
        else if (j && i < na && b[j - 1] >= a[i]) lo = i + 1;
        else break;
    }
    for (unsigned k = 0; k < 4 && diagonal + k < na + nb; ++k) {
        if (i < na && (j == nb || a[i] <= b[j])) out[diagonal + k] = a[i++];
        else out[diagonal + k] = b[j++];
    }
}

__global__ void decode_wide(RunView input, U* output) {
    for (size_t i = blockIdx.x * blockDim.x + threadIdx.x; i < input.n;
         i += size_t(blockDim.x) * gridDim.x) output[i] = code_at(input, i);
}

struct Plan { Code source, block; };

__device__ Frame fence_frame(RunView v, size_t block) {
    if (v.kind != 4) return v.frames[block];
    Frame f = v.frames[block * 16];
    size_t last = (block + 1) * 16 - 1;
    size_t count = (v.n + 63) / 64;
    if (last >= count) last = count - 1;
    f.maximum = v.frames[last].maximum;
    return f;
}

// Only descriptors are inspected here. A strictly nonoverlapping source block
// has a rank equal to its own row offset plus all preceding opposite blocks.
// A copy requires a complete aligned output block; otherwise it is merged.
// This also handles equal fences conservatively: equality is overlap.
__global__ void plan_copies(RunView own, RunView other, Plan* plans, Code source) {
    size_t block = blockIdx.x * blockDim.x + threadIdx.x;
    size_t count = (own.n + tile_size - 1) / tile_size;
    if (block >= count || (block + 1) * tile_size > own.n) return;
    Frame f = fence_frame(own, block);
    size_t lo = 0, hi = (other.n + tile_size - 1) / tile_size;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        if (fence_frame(other, mid).maximum < f.base) lo = mid + 1;
        else hi = mid;
    }
    size_t other_blocks = (other.n + tile_size - 1) / tile_size;
    if (lo < other_blocks && fence_frame(other, lo).base <= f.maximum) return;
    size_t preceding = lo == other_blocks ? other.n : lo * tile_size;
    size_t rank = block * tile_size + preceding;
    if (rank % tile_size == 0) plans[rank / tile_size] = {source, Code(block)};
}

__global__ void plan_frames(RunView a, RunView b, const Plan* plans,
                            Frame* frames, Code* sizes, int kind) {
    size_t block = blockIdx.x * blockDim.x + threadIdx.x;
    unsigned frame_size = kind == 4 ? 64 : tile_size;
    unsigned per_fence = tile_size / frame_size;
    size_t n = a.n + b.n, start = block * frame_size;
    if (start >= n) return;
    Plan p = plans[block / per_fence];
    if (p.source) {
        Frame f = (p.source == 1 ? a : b).frames[p.block * per_fence + block % per_fence];
        frames[block] = f;
        sizes[block] = (frame_size / 32) * f.width;
        return;
    }
    size_t last = start + frame_size - 1 < n ? start + frame_size - 1 : n - 1;
    size_t ai = partition(a, b, start), bi = start - ai;
    Code first = next_code(a, b, ai, bi);
    Code maximum = first, range = 0;
    if (kind == 4) {
        for (size_t row = start + 1; row <= last; ++row) {
            Code value = next_code(a, b, ai, bi);
            range = max(range, value - maximum);
            maximum = value;
        }
    } else {
        ai = partition(a, b, last); bi = last - ai;
        maximum = next_code(a, b, ai, bi);
        range = maximum - first;
    }
    unsigned width = frame_width(range, kind);
    frames[block] = {first, maximum, 0, width};
    sizes[block] = (frame_size / 32) * width;
}


// Overlaps are merged directly into packed words. Each word owner co-ranks
// once, keeps canonical codes in registers, and writes only packed output.
__global__ void merge_packed(RunView a, RunView b, const Plan* plans,
                             Frame* frames, const Code* offsets,
                             Code* fences, Code* words) {
    unsigned frame_size = a.kind == 4 ? 64 : tile_size;
    unsigned per_fence = tile_size / frame_size;
    size_t block = blockIdx.x, start = block * frame_size;
    Frame f = frames[block];
    f.offset = offsets[block];
    __syncthreads();
    if (!threadIdx.x) {
        frames[block] = f;
        if (block % per_fence == 0) fences[block / per_fence] = f.base;
    }
    Plan p = plans[block / per_fence];
    if (p.source) {
        RunView input = p.source == 1 ? a : b;
        Frame original = input.frames[p.block * per_fence + block % per_fence];
        for (unsigned word = threadIdx.x; word < (frame_size / 32) * f.width; word += blockDim.x)
            words[f.offset + word] = input.words[original.offset + word];
        return;
    }
    for (unsigned word = threadIdx.x; word < (frame_size / 32) * f.width; word += blockDim.x) {
        unsigned bit = word * 32, index = bit / f.width, shift = bit % f.width;
        size_t diagonal = start + index;
        bool predecessor = a.kind == 4 && index > 0;
        size_t i = partition(a, b, diagonal - predecessor), j = diagonal - predecessor - i;
        Code prior = predecessor ? next_code(a, b, i, j) : f.base;
        Code value = diagonal < a.n+b.n ? next_code(a, b, i, j) : f.base;
        Code r = a.kind == 4 ? (index ? value-prior : 0) : value-f.base;
        Code packed = r >> shift;
        unsigned used = f.width-shift;
        while (used < 32 && ++index < frame_size) {
            prior = value;
            value = start+index < a.n+b.n ? next_code(a, b, i, j) : f.base;
            r = a.kind == 4 ? value-prior : value-f.base;
            packed |= r << used;
            used += f.width;
        }
        words[f.offset+word] = packed;
    }
}

template<class T>
__global__ void seal_frames(const T* input, size_t n, Frame* frames, Code* sizes, int kind) {
    unsigned frame_size = kind == 4 ? 64 : tile_size;
    size_t block = blockIdx.x * blockDim.x + threadIdx.x, start = block * frame_size;
    if (start >= n) return;
    size_t end = start + frame_size < n ? start + frame_size : n;
    Code first = input[start], maximum = input[end - 1], range = maximum - first;
    if (kind == 4) {
        range = 0;
        for (size_t row = start + 1; row < end; ++row) range = max(range, Code(input[row] - input[row - 1]));
    }
    unsigned width = frame_width(range, kind);
    frames[block] = {first, maximum, 0, width};
    sizes[block] = (frame_size / 32) * width;
}

template<class T>
__global__ void seal_words(const T* input, size_t n, Frame* frames,
                           const Code* offsets, Code* fences, Code* words, int kind) {
    unsigned frame_size = kind == 4 ? 64 : tile_size;
    unsigned per_fence = tile_size / frame_size;
    size_t start = size_t(blockIdx.x) * frame_size;
    Frame f = frames[blockIdx.x]; f.offset = offsets[blockIdx.x];
    __syncthreads();
    if (!threadIdx.x) {
        frames[blockIdx.x] = f;
        if (blockIdx.x % per_fence == 0) fences[blockIdx.x / per_fence] = f.base;
    }
    for (unsigned word = threadIdx.x; word < (frame_size/32)*f.width; word += blockDim.x) {
        unsigned bit=word*32, index=bit/f.width, shift=bit%f.width;
        Code value=start+index<n?Code(input[start+index]):f.base;
        Code prior=index?Code(input[start+index-1]):f.base;
        Code r=kind==4?(index?value-prior:0):value-f.base;
        Code packed=r>>shift;
        unsigned used=f.width-shift;
        while(used<32&&++index<frame_size) {
            prior=value; value=start+index<n?Code(input[start+index]):f.base;
            r=kind==4?value-prior:value-f.base;
            packed|=r<<used; used+=f.width;
        }
        words[f.offset+word]=packed;
    }
}

__global__ void check_frames(RunView v, const Code* oracle, size_t payload_words,
                             unsigned* error) {
    size_t block = blockIdx.x * blockDim.x + threadIdx.x;
    unsigned frame_size = v.kind == 4 ? 64 : tile_size;
    unsigned per_fence = tile_size / frame_size;
    size_t blocks = (v.n + frame_size - 1) / frame_size;
    if (block >= blocks) return;
    size_t start = block * frame_size;
    size_t end = start + frame_size < v.n ? start + frame_size : v.n;
    Frame f = v.frames[block];
    Code range = oracle[end - 1] - oracle[start];
    if (v.kind == 4) {
        range = 0;
        for (size_t row = start + 1; row < end; ++row) range = max(range, oracle[row] - oracle[row - 1]);
    }
    unsigned width = frame_width(range, v.kind);
    size_t expected_offset = block ? size_t(v.frames[block - 1].offset) + (frame_size / 32) * v.frames[block - 1].width : 0;
    if (f.base != oracle[start] || f.maximum != oracle[end - 1] || f.width != width ||
        f.offset != expected_offset || (block % per_fence == 0 && v.fences[block / per_fence] != oracle[start]) ||
        (block + 1 == blocks && size_t(f.offset) + (frame_size / 32) * f.width != payload_words))
        atomicExch(error, 1u);
}

inline size_t packed_bytes(const std::vector<Code>& values, int kind) {
    size_t frame_size = kind == 4 ? 64 : tile_size;
    size_t bytes = ((values.size() + tile_size - 1) / tile_size) * sizeof(Code);
    for (size_t start = 0; start < values.size(); start += frame_size) {
        size_t end = std::min(start + frame_size, values.size());
        Code range = values[end - 1] - values[start];
        if (kind == 4) {
            range = 0;
            for (size_t row = start + 1; row < end; ++row) range = std::max(range, values[row] - values[row - 1]);
        }
        unsigned width = bits(range);
        if (kind == 2) width = width <= 8 ? 8 : width <= 16 ? 16 : 32;
        bytes += (frame_size / 8) * width + sizeof(Frame);
    }
    return bytes;
}

struct PackedOutput {
    size_t n, blocks, fence_count, stored, scan_bytes = 0;
    int kind;
    Code minimum, maximum;
    Buf words, frames, fences, sizes, offsets, scan;
    PackedOutput(const std::vector<Code>& oracle, int layout)
        : n(oracle.size()), blocks((n + (layout == 4 ? 63 : tile_size - 1)) / (layout == 4 ? 64 : tile_size)),
          fence_count((n + tile_size - 1) / tile_size),
          stored(packed_bytes(oracle, layout)), kind(layout),
          minimum(oracle.front()), maximum(oracle.back()), words(fence_count * tile_size * sizeof(Code)),
          frames(blocks * sizeof(Frame)), fences(fence_count * sizeof(Code)),
          sizes(blocks * sizeof(Code)), offsets(blocks * sizeof(Code)) {
        CK(cub::DeviceScan::ExclusiveSum(nullptr, scan_bytes, sizes.as<Code>(), offsets.as<Code>(), blocks));
        scan = Buf(scan_bytes);
    }
    RunView view() const {
        return {words.as<Code>(), nullptr, frames.as<Frame>(), fences.as<Code>(), nullptr,
                n, minimum, maximum, kind};
    }
    void scan_offsets() {
        CK(cub::DeviceScan::ExclusiveSum(scan.p, scan_bytes, sizes.as<Code>(), offsets.as<Code>(), blocks));
    }
    template<class T> void seal(const T* input) {
        seal_frames<<<(blocks + 255) / 256, 256>>>(input, n, frames.as<Frame>(), sizes.as<Code>(), kind);
        scan_offsets();
        seal_words<<<blocks, 256>>>(input, n, frames.as<Frame>(), offsets.as<Code>(), fences.as<Code>(), words.as<Code>(), kind);
    }
    bool check(Buf& oracle, Buf& error) const {
        CK(cudaMemset(error.p, 0, sizeof(unsigned)));
        check_run_kernel<<<1024, 256>>>(view(), oracle.as<Code>(), n, error.as<unsigned>());
        size_t payload_words = (stored - blocks * sizeof(Frame) - fence_count * sizeof(Code)) / sizeof(Code);
        check_frames<<<(blocks + 255) / 256, 256>>>(view(), oracle.as<Code>(), payload_words, error.as<unsigned>());
        unsigned e;
        CK(cudaMemcpy(&e, error.p, sizeof(e), cudaMemcpyDeviceToHost));
        return e == 0;
    }
};

template<class T>
__global__ void check_plain_values(const T* values, const Code* oracle, size_t n, unsigned* error) {
    for (size_t i = blockIdx.x * blockDim.x + threadIdx.x; i < n;
         i += size_t(blockDim.x) * gridDim.x)
        if (U(values[i]) != U(oracle[i])) atomicExch(error, 1u);
}

inline bool check_plain(const Buf& values, int kind, Buf& oracle, size_t n, Buf& error) {
    CK(cudaMemset(error.p, 0, sizeof(unsigned)));
    if (kind == 0) check_plain_values<<<1024, 256>>>(values.as<U>(), oracle.as<Code>(), n, error.as<unsigned>());
    else check_plain_values<<<1024, 256>>>(values.as<Code>(), oracle.as<Code>(), n, error.as<unsigned>());
    unsigned e;
    CK(cudaMemcpy(&e, error.p, sizeof(e), cudaMemcpyDeviceToHost));
    return e == 0;
}

} // namespace compressed_merge_sort

inline void do_merge(const std::string& regime) {
    using namespace compressed_merge_sort;
    auto preparation_start = std::chrono::steady_clock::now();
    if (regime != "disjoint" && regime != "interleaved") {
        std::cerr << "O3 regime must be disjoint or interleaved\n"; std::exit(5);
    }
    const size_t na = size_t(1) << 23, nb = na, n = na + nb, blocks = n / tile_size;
    std::vector<Code> a(na), b(nb), oracle(n);
    for (size_t i = 0; i < na; ++i) {
        // Repeated adjacent keys on both sides; interleaved also duplicates
        // across inputs. Disjoint ranges have a strict gap at their boundary.
        a[i] = Code(i / 2 * 4);
        b[i] = regime == "disjoint" ? Code(na * 4 + i / 2 * 4)
                                    : Code(i / 2 * 4 + (i / 2 % 2 ? 0 : 1));
    }
    std::merge(a.begin(), a.end(), b.begin(), b.end(), oracle.begin());
    Buf reference = upload(oracle), error(sizeof(unsigned));
    std::array<Run, 5> ar, br;
    for (int kind = 0; kind < 5; ++kind) { ar[kind] = make_run(a, kind); br[kind] = make_run(b, kind); }
    Buf out64(n * sizeof(U)), out32(n * sizeof(Code));
    Buf decoded_a(na * sizeof(U)), decoded_b(nb * sizeof(U));
    Buf plans(blocks * sizeof(Plan));
    PackedOutput bytes(oracle, 2), packed(oracle, 3), delta(oracle, 4);
    std::vector<Variant> variants;
    for (int kind = 0; kind < 2; ++kind) {
        std::string name = run_names[kind];
        size_t stored = ar[kind].bytes() + br[kind].bytes() + n * (kind == 0 ? sizeof(U) : sizeof(Code));
        metadata("O3", regime, name, "L2", stored, n);
        variants.push_back({name, stored, double(stored), [&, kind] {
            if (kind == 0) plain_merge<<<blocks, 256>>>(ar[0].du.as<U>(), na, br[0].du.as<U>(), nb, out64.as<U>());
            else plain_merge<<<blocks, 256>>>(ar[1].dw.as<Code>(), na, br[1].dw.as<Code>(), nb, out32.as<Code>());
        }, [&, kind] {
            return check_plain(kind == 0 ? out64 : out32, kind, reference, n, error);
        }});
    }
    Variant noise = variants[1]; noise.name = "V1_noise";
    metadata("O3", regime, noise.name, "L2", noise.stored, n); variants.push_back(noise);
    for (int kind = 2; kind < 5; ++kind) {
        PackedOutput* output = kind == 2 ? &bytes : kind == 3 ? &packed : &delta;
        size_t stored = ar[kind].bytes() + br[kind].bytes() + output->stored;
        std::string name = run_names[kind];
        metadata("O3", regime, name, "L0+L1", stored, n);
        variants.push_back({name, stored, double(stored), [&, kind, output] {
            CK(cudaMemsetAsync(plans.p, 0, plans.n));
            plan_copies<<<(na / tile_size + 255) / 256, 256>>>(ar[kind].view(), br[kind].view(), plans.as<Plan>(), 1);
            plan_copies<<<(nb / tile_size + 255) / 256, 256>>>(br[kind].view(), ar[kind].view(), plans.as<Plan>(), 2);
            plan_frames<<<(output->blocks + 255) / 256, 256>>>(ar[kind].view(), br[kind].view(), plans.as<Plan>(), output->frames.as<Frame>(), output->sizes.as<Code>(), kind);
            output->scan_offsets();
            merge_packed<<<output->blocks, 256>>>(ar[kind].view(), br[kind].view(), plans.as<Plan>(), output->frames.as<Frame>(), output->offsets.as<Code>(), output->fences.as<Code>(), output->words.as<Code>());
        }, [&, output] { return output->check(reference, error); }});
        name += "_decode_merge_reencode";
        metadata("O3", regime, name, "L2", stored, n);
        variants.push_back({name, stored, double(stored + n * sizeof(U) * 4), [&, kind, output] {
            decode_wide<<<1024, 256>>>(ar[kind].view(), decoded_a.as<U>());
            decode_wide<<<1024, 256>>>(br[kind].view(), decoded_b.as<U>());
            plain_merge<<<blocks, 256>>>(decoded_a.as<U>(), na, decoded_b.as<U>(), nb, out64.as<U>());
            output->seal(out64.as<U>());
        }, [&, output] { return output->check(reference, error); }});
    }
    std::string extra = ",\"input_a_keys\":" + std::to_string(na) + ",\"input_b_keys\":" + std::to_string(nb)
        + ",\"output_keys\":" + std::to_string(n) + ",\"bag_preserving\":true,\"planning_timed\":true"
        + ",\"stored_bytes_scope\":\"inputs_plus_output_excluding_workspace\""
        + ",\"compressed_output_fence_keys\":1024,\"fence_bytes_per_block\":4,\"V2_V3_frame_bytes_per_block\":16,\"V4_frame_bytes_per_block\":256,\"V4_restart_keys\":64"
        + ",\"copyable_input_blocks\":" + std::to_string(regime == "disjoint" ? blocks : 0)
        + ",\"overlap_input_blocks\":" + std::to_string(regime == "disjoint" ? 0 : blocks)
        + ",\"copied_output_blocks\":" + std::to_string(regime == "disjoint" ? blocks : 0)
        + ",\"reencoded_output_blocks\":" + std::to_string(regime == "disjoint" ? 0 : blocks)
        + ",\"block_counts_scope\":\"V2_bytes_V3_for_V4_delta_structural_variants;1024_key_blocks;derived_from_input_fences\""
        + ",\"V5_bitmap_rank\":\"not_applicable:membership_bitmap_cannot_preserve_bag_multiplicities\""
        + ",\"V2_output_bytes\":" + std::to_string(bytes.stored)
        + ",\"V3_output_bytes\":" + std::to_string(packed.stored)
        + ",\"V4_output_bytes\":" + std::to_string(delta.stored);
    loading_record("O3_"+regime, preparation_start);
    bench("O3", regime, n, variants, extra);
}

inline void do_sort() {
    using namespace compressed_merge_sort;
    auto preparation_start = std::chrono::steady_clock::now();
    const size_t n = size_t(1) << 24, distinct = n * 3 / 4;
    std::vector<Code> input(n);
    for (size_t i = 0; i < n; ++i) input[i] = Code((1u << 28) + (i % distinct) * 4);
    std::mt19937 random(0x534f5254); std::shuffle(input.begin(), input.end(), random);
    std::vector<Code> oracle = input;
    std::sort(oracle.begin(), oracle.end());
    oracle.erase(std::unique(oracle.begin(), oracle.end()), oracle.end());
    if (oracle.size() != distinct) { std::cerr << "O4 duplicate construction failed\n"; std::exit(5); }
    // The distinguishing span is min XOR max, not bits(max-min) alone:
    // high common bits and the two known-zero alignment bits need no pass.
    Code differing = oracle.front() ^ oracle.back();
    int begin_bit = 2, end_bit = int(bits(differing));
    Buf input32 = upload(input), reference = upload(oracle), error(sizeof(unsigned));
    std::vector<U> wide_input(input.begin(), input.end()), wide_oracle(oracle.begin(), oracle.end());
    Buf input64 = upload(wide_input), reference64 = upload(wide_oracle);
    Buf sorted64(n * sizeof(U)), unique64(n * sizeof(U));
    Buf sorted32(n * sizeof(Code)), unique32(n * sizeof(Code)), selected(sizeof(int));
    size_t sort64_bytes = 0, sort32_bytes = 0, sortbits_bytes = 0, unique64_bytes = 0, unique32_bytes = 0;
    CK(cub::DeviceRadixSort::SortKeys(nullptr, sort64_bytes, input64.as<U>(), sorted64.as<U>(), int(n), 0, 64));
    CK(cub::DeviceRadixSort::SortKeys(nullptr, sort32_bytes, input32.as<Code>(), sorted32.as<Code>(), int(n), 0, 32));
    CK(cub::DeviceRadixSort::SortKeys(nullptr, sortbits_bytes, input32.as<Code>(), sorted32.as<Code>(), int(n), begin_bit, end_bit));
    CK(cub::DeviceSelect::Unique(nullptr, unique64_bytes, sorted64.as<U>(), unique64.as<U>(), selected.as<int>(), int(n)));
    CK(cub::DeviceSelect::Unique(nullptr, unique32_bytes, sorted32.as<Code>(), unique32.as<Code>(), selected.as<int>(), int(n)));
    Buf workspace(std::max({sort64_bytes, sort32_bytes, sortbits_bytes, unique64_bytes, unique32_bytes}));
    std::vector<Variant> variants;
    const std::array<std::string, 3> names = {"V0_u64", "V1_u32", "V1_u32_significant_bits"};
    for (int mode = 0; mode < 3; ++mode) {
        size_t stored = (n + distinct) * (mode == 0 ? sizeof(U) : sizeof(Code));
        // Even narrow code radix sort is L2: it emits global plain arrays.
        metadata("O4", "unsorted_25pct_duplicates", names[mode], "L2", stored, n);
        variants.push_back({names[mode], stored, double(stored), [&, mode] {
            if (mode == 0) {
                CK(cub::DeviceRadixSort::SortKeys(workspace.p, sort64_bytes, input64.as<U>(), sorted64.as<U>(), int(n), 0, 64));
                CK(cub::DeviceSelect::Unique(workspace.p, unique64_bytes, sorted64.as<U>(), unique64.as<U>(), selected.as<int>(), int(n)));
            } else {
                size_t& temp_bytes = mode == 1 ? sort32_bytes : sortbits_bytes;
                CK(cub::DeviceRadixSort::SortKeys(workspace.p, temp_bytes, input32.as<Code>(), sorted32.as<Code>(), int(n), mode == 1 ? 0 : begin_bit, mode == 1 ? 32 : end_bit));
                CK(cub::DeviceSelect::Unique(workspace.p, unique32_bytes, sorted32.as<Code>(), unique32.as<Code>(), selected.as<int>(), int(n)));
            }
        }, [&, mode] {
            int count; CK(cudaMemcpy(&count, selected.p, sizeof(count), cudaMemcpyDeviceToHost));
            if (count != int(distinct)) return false;
            return check_plain(mode == 0 ? unique64 : unique32, mode == 0 ? 0 : 1, reference, distinct, error);
        }});
    }
    Variant noise = variants[1]; noise.name = "V1_noise";
    metadata("O4", "unsorted_25pct_duplicates", noise.name, "L2", noise.stored, n); variants.push_back(noise);
    std::string extra = ",\"input_keys\":" + std::to_string(n) + ",\"unique_keys\":" + std::to_string(distinct)
        + ",\"duplicate_excess_keys\":" + std::to_string(n - distinct)
        + ",\"duplicate_fraction\":0.25,\"duplicate_definition\":\"(n-distinct)/n\""
        + ",\"significant_begin_bit\":" + std::to_string(begin_bit)
        + ",\"significant_end_bit\":" + std::to_string(end_bit)
        + ",\"code_min\":" + std::to_string(oracle.front()) + ",\"code_max\":" + std::to_string(oracle.back())
        + ",\"level_reason\":\"CUB sort and adjacent unique materialize global plain arrays, including u32 codes\""
        + ",\"stored_bytes_scope\":\"input_plus_unique_output_excluding_workspace\",\"timed_operations\":\"radix_sort_plus_adjacent_unique\"";
    loading_record("O4_unsorted_25pct_duplicates", preparation_start);
    bench("O4", "unsorted_25pct_duplicates", n, variants, extra);

    // A separate resident-batch measurement isolates sealing from radix sort.
    // Its input is the complete CPU sorted-unique oracle uploaded before timing.
    preparation_start = std::chrono::steady_clock::now();
    PackedOutput bytes(oracle, 2), packed(oracle, 3);
    std::vector<Variant> seals;
    for (int kind = 0; kind < 4; ++kind) {
        PackedOutput* output = kind == 2 ? &bytes : kind == 3 ? &packed : nullptr;
        size_t output_bytes = output ? output->stored : distinct * (kind == 0 ? sizeof(U) : sizeof(Code));
        size_t stored = distinct * (kind == 0 ? sizeof(U) : sizeof(Code)) + output_bytes;
        metadata("O4_seal", "sorted_unique", run_names[kind], kind < 2 ? "L2" : "L1", stored, distinct);
        seals.push_back({run_names[kind], stored, double(stored), [&, kind, output] {
            if (kind == 0) CK(cudaMemcpyAsync(unique64.p, reference64.p, distinct * sizeof(U), cudaMemcpyDeviceToDevice));
            else if (kind == 1) CK(cudaMemcpyAsync(unique32.p, reference.p, distinct * sizeof(Code), cudaMemcpyDeviceToDevice));
            else output->seal(reference.as<Code>());
        }, [&, kind, output] {
            if (output) return output->check(reference, error);
            return check_plain(kind == 0 ? unique64 : unique32, kind, reference, distinct, error);
        }});
    }
    Variant seal_noise = seals[1]; seal_noise.name = "V1_noise";
    metadata("O4_seal", "sorted_unique", seal_noise.name, "L2", seal_noise.stored, distinct); seals.push_back(seal_noise);
    loading_record("O4_seal_sorted_unique", preparation_start);
    bench("O4_seal", "sorted_unique", distinct, seals,
          ",\"input_layout\":\"resident_plain_sorted_unique\",\"sort_and_unique_included\":false"
          ",\"plain_controls\":\"device_to_device_copy\",\"planning_and_scan_timed\":true"
          ",\"stored_bytes_scope\":\"input_plus_output_excluding_workspace\""
          ",\"level_reason\":\"V2/V3 read existing u32 codes and emit only framed codes; plain controls emit global plain arrays\""
          ",\"V2_output_bytes\":" + std::to_string(bytes.stored) + ",\"V3_output_bytes\":" + std::to_string(packed.stored));
}
