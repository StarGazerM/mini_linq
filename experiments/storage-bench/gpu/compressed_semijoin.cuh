#pragma once

// Measurement-only E-b. Included after compressed_common.cuh in one CUDA TU.
// Stored bytes describe resident operator inputs plus the result representation;
// CUB workspace and oracle allocations are reported separately, never timed.
namespace compressed_sj {
using Clock = std::chrono::steady_clock;
inline double elapsed(Clock::time_point t) {
    return std::chrono::duration<double, std::milli>(Clock::now() - t).count();
}
inline void require(bool ok, const char* why) {
    if (!ok) { std::cerr << "E-b: " << why << '\n'; std::exit(5); }
}
__host__ __device__ inline Code mix(Code x) {
    x ^= x >> 16; x *= 0x7feb352du;
    x ^= x >> 15; x *= 0x846ca68bu;
    return x ^ (x >> 16);
}
inline size_t power2(size_t n) {
    size_t p = 1;
    while (p < n) p <<= 1;
    return p;
}
inline std::vector<Code> sorted_unique(std::vector<Code> v) {
    std::sort(v.begin(), v.end());
    v.erase(std::unique(v.begin(), v.end()), v.end());
    return v;
}
struct DateRange { Code low, high; };
inline DateRange quantiles(const std::vector<Code>& sorted, int low, int high) {
    return {sorted[sorted.size() * low / 100],
            high == 100 ? sorted.back() + 1 : sorted[sorted.size() * high / 100]};
}
inline std::vector<Code> select_dates(const std::vector<Code>& keys,
                                     const std::vector<Code>& dates, DateRange r) {
    require(keys.size() == dates.size(), "order key/date row counts differ");
    std::vector<Code> selected;
    for (size_t i = 0; i < keys.size(); ++i)
        if (dates[i] >= r.low && dates[i] < r.high) selected.push_back(keys[i]);
    require(!selected.empty(), "empty date selection");
    return selected;
}
inline std::vector<Code> bitmap(const std::vector<Code>& keys, Code minimum, Code maximum) {
    std::vector<Code> words((size_t(maximum) - minimum + 32) / 32, 0);
    for (Code k : keys) words[(k-minimum) / 32] |= Code(1) << ((k-minimum) % 32);
    return words;
}
template<class T>
__device__ inline bool contains(const T* sorted, size_t n, T key) {
    size_t lo = 0, hi = n;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        if (sorted[mid] < key) lo = mid + 1; else hi = mid;
    }
    return lo < n && sorted[lo] == key;
}
__device__ inline bool hash_contains(const Code* table, size_t mask, Code key) {
    if (!key) return false; // SF10 order keys are positive; checked on the CPU.
    size_t slot = mix(key) & mask;
    for (size_t step = 0; step <= mask; ++step) {
        Code value = table[slot];
        if (value == key) return true;
        if (!value) return false;
        slot = (slot + 1) & mask;
    }
    return false;
}
__global__ void hash_insert(const Code* keys, size_t n, Code* table, size_t mask) {
    for (size_t i = size_t(blockIdx.x) * blockDim.x + threadIdx.x;
         i < n; i += size_t(gridDim.x) * blockDim.x) {
        Code key = keys[i];
        size_t slot = mix(key) & mask;
        for (;;) {
            Code prior = atomicCAS(table + slot, 0u, key);
            if (!prior || prior == key) break;
            slot = (slot + 1) & mask;
        }
    }
}
__global__ void bitmap_insert(const Code* keys, size_t n, Code* words, Code minimum) {
    for (size_t i = size_t(blockIdx.x) * blockDim.x + threadIdx.x;
         i < n; i += size_t(gridDim.x) * blockDim.x) {
        Code key = keys[i] - minimum;
        atomicOr(words + key / 32, Code(1) << (key % 32));
    }
}
template<class T>
__global__ void compare_array(const T* actual, const Code* expected, size_t n,
                              unsigned* error) {
    for (size_t i = size_t(blockIdx.x) * blockDim.x + threadIdx.x;
         i < n; i += size_t(gridDim.x) * blockDim.x)
        if (actual[i] != T(expected[i])) atomicExch(error, 1u);
}
inline bool no_error(Buf& error) {
    unsigned value;
    CK(cudaMemcpy(&value, error.p, sizeof(value), cudaMemcpyDeviceToHost));
    return value == 0;
}
template<class T>
inline bool exact_array(Buf& actual, Buf& expected, size_t n, Buf& error) {
    CK(cudaMemset(error.p, 0, error.n));
    compare_array<T><<<1024, 256>>>(actual.as<T>(), expected.as<Code>(), n,
                                   error.as<unsigned>());
    return no_error(error);
}
__global__ void check_hash(const Code* table, size_t capacity, const Code* expected,
                           size_t n, unsigned* error, U* occupied) {
    U count = 0;
    for (size_t i = size_t(blockIdx.x) * blockDim.x + threadIdx.x;
         i < capacity; i += size_t(gridDim.x) * blockDim.x) {
        Code key = table[i];
        if (key) {
            ++count;
            if (!contains(expected, n, key)) atomicExch(error, 1u);
        }
    }
    for (size_t i = size_t(blockIdx.x) * blockDim.x + threadIdx.x;
         i < n; i += size_t(gridDim.x) * blockDim.x)
        if (!hash_contains(table, capacity - 1, expected[i])) atomicExch(error, 1u);
    blocksum(count, occupied);
}
inline bool exact_hash(Buf& table, Buf& expected, size_t n, Buf& error, Buf& count) {
    CK(cudaMemset(error.p, 0, error.n));
    CK(cudaMemset(count.p, 0, count.n));
    check_hash<<<1024, 256>>>(table.as<Code>(), table.n / sizeof(Code),
                             expected.as<Code>(), n, error.as<unsigned>(), count.as<U>());
    return no_error(error) && scalarcheck(count, n);
}

// Same lane-interleaved payload as bench.cu encode(kind=2)/get: 32 lanes,
// each with 32 residuals per 1024-code frame. No logical-u64 reconstruction.
// Frame retains the exact maximum for the zone variant, not base+(2^width-1).
struct Probe {
    Buf plain, packed, frames;
    explicit Probe(const std::vector<Code>& keys) {
        std::vector<Frame> descriptors;
        std::vector<Code> words;
        for (size_t st = 0; st < keys.size(); st += 1024) {
            size_t en = std::min(st + 1024, keys.size());
            auto mm = std::minmax_element(keys.begin() + st, keys.begin() + en);
            unsigned width = bits(*mm.second - *mm.first);
            size_t offset = words.size();
            require(offset <= UINT32_MAX, "FOR word offset exceeds u32");
            descriptors.push_back({*mm.first, *mm.second, Code(offset), width});
            words.resize(offset + size_t(width) * 32, 0);
            for (size_t i = st; i < en && width; ++i) {
                size_t j = i - st, bit = (j / 32) * width;
                size_t word = offset + (bit / 32) * 32 + j % 32;
                unsigned shift = bit % 32;
                Code residual = keys[i] - *mm.first;
                words[word] |= residual << shift;
                if (shift + width > 32) words[word + 32] |= residual >> (32 - shift);
            }
        }
        plain = upload(keys); packed = upload(words); frames = upload(descriptors);
    }
};
template<bool Packed>
__device__ inline Code probe_code(const Code* words, const Frame* frames, size_t i) {
    if constexpr (!Packed) return words[i];
    Frame f = frames[i / 1024];
    if (!f.width) return f.base;
    size_t j = i % 1024, bit = (j / 32) * f.width;
    size_t word = f.offset + (bit / 32) * 32 + j % 32;
    unsigned shift = bit % 32;
    Code x = words[word] >> shift;
    if (shift + f.width > 32) x |= words[word + 32] << (32 - shift);
    if (f.width < 32) x &= (Code(1) << f.width) - 1;
    return f.base + x;
}
struct SetView {
    const U* wide;
    const Code* sorted;
    const Code* hash;
    const Code* bitmap;
    const Code* bloom;
    size_t n, hash_mask, bloom_mask;
    Code minimum, maximum;
};
// Mode 0 is the old wide-key control; all others compare canonical u32 codes.
template<int Mode>
__device__ inline bool member(SetView set, Code key) {
    if constexpr (Mode == 0) return contains(set.wide, set.n, U(key));
    if constexpr (Mode == 1) return contains(set.sorted, set.n, key);
    if constexpr (Mode == 2) return hash_contains(set.hash, set.hash_mask, key);
    if constexpr (Mode == 3 || Mode == 4)
        return key >= set.minimum && key <= set.maximum && ((set.bitmap[(key-set.minimum) / 32] >> ((key-set.minimum) % 32)) & 1u);
    if constexpr (Mode == 5) {
        Code h = mix(key), step = mix(key ^ 0x9e3779b9u) | 1u;
        for (unsigned j = 0; j < 4; ++j) {
            size_t bit = (h + j * step) & set.bloom_mask;
            if (!(set.bloom[bit / 32] & (Code(1) << (bit % 32)))) return false;
        }
        return contains(set.sorted, set.n, key); // Bloom never decides membership.
    }
    return false;
}
template<bool Packed, int Mode>
__global__ void semijoin(const Code* words, const Frame* frames, size_t n,
                         SetView set, U* output) {
    __shared__ unsigned any_bits, missing_bits;
    U sum = 0;
    for (size_t tile = blockIdx.x; tile < (n + 1023) / 1024; tile += gridDim.x) {
        size_t st = tile * 1024, en = min(st + size_t(1024), n);
        if constexpr (Mode == 4) {
            if (!threadIdx.x) { any_bits = 0; missing_bits = 0; }
            __syncthreads();
            Frame f = frames[tile];
            bool any = false, missing = f.maximum > set.maximum || f.base < set.minimum;
            if (f.base <= set.maximum && f.maximum >= set.minimum) {
                Code low = max(f.base, set.minimum) - set.minimum;
                Code high = min(f.maximum, set.maximum) - set.minimum;
                size_t first = low / 32, last = high / 32;
                for (size_t w = first + threadIdx.x; w <= last; w += blockDim.x) {
                    Code mask = ~Code(0);
                    if (w == first) mask &= ~Code(0) << (low % 32);
                    if (w == last) mask &= ~Code(0) >> (31 - high % 32);
                    Code present = set.bitmap[w] & mask;
                    any |= present != 0;
                    missing |= present != mask;
                }
            }
            if (any) atomicOr(&any_bits, 1u);
            if (missing) atomicOr(&missing_bits, 1u);
            __syncthreads();
            // Exact range-word test: holes in the key domain count as missing.
            if (any_bits && !missing_bits) {
                if (!threadIdx.x) sum += en - st;
            } else if (any_bits) {
                for (size_t i = st + threadIdx.x; i < en; i += blockDim.x)
                    sum += member<3>(set, probe_code<Packed>(words, frames, i));
            }
            __syncthreads();
        } else {
            for (size_t i = st + threadIdx.x; i < en; i += blockDim.x)
                sum += member<Mode>(set, probe_code<Packed>(words, frames, i));
        }
    }
    blocksum(sum, output);
}
template<bool Packed, int Mode>
inline void add_probe(std::vector<Variant>& variants, const std::string& name,
                       Probe& probe, size_t n, SetView set, size_t set_bytes,
                       Buf& output, U oracle) {
    size_t probe_bytes = Packed ? probe.packed.n + probe.frames.n : probe.plain.n;
    if (!Packed && Mode == 4) probe_bytes += probe.frames.n;
    variants.push_back({name, probe_bytes + set_bytes + output.n, 0,
        [&, n, set] {
            CK(cudaMemsetAsync(output.p, 0, output.n));
            semijoin<Packed, Mode><<<4096, 256>>>(
                Packed ? probe.packed.as<Code>() : probe.plain.as<Code>(),
                probe.frames.as<Frame>(), n, set, output.as<U>());
        },
        [&, oracle] { return scalarcheck(output, oracle); }});
}
template<bool Packed>
inline void probe_group(Probe& probe, size_t n, SetView set,
                         size_t bitmap_bytes, size_t hash_bytes, size_t bloom_bytes,
                         Buf& output, U oracle, const std::string& caseid,
                         const std::string& extra) {
    std::vector<Variant> variants;
    add_probe<Packed, 0>(variants, "V0_u64", probe, n, set, set.n * 8, output, oracle);
    add_probe<Packed, 1>(variants, "V1_u32", probe, n, set, set.n * 4, output, oracle);
    variants.push_back(variants.back()); variants.back().name = "V1_noise";
    add_probe<Packed, 2>(variants, "V2_hash_u32", probe, n, set, hash_bytes, output, oracle);
    add_probe<Packed, 3>(variants, "V3_bitmap", probe, n, set, bitmap_bytes, output, oracle);
    add_probe<Packed, 4>(variants, "V4_bitmap_zone", probe, n, set, bitmap_bytes, output, oracle);
    add_probe<Packed, 5>(variants, "V5_bloom_exact_u32", probe, n, set,
                         bloom_bytes + set.n * 4, output, oracle);
    for (auto& v : variants)
        metadata("E_b_semijoin", caseid, v.name,
                 v.name == "V0_u64" ? "L2_u64_key_control" :
                 v.name == "V4_bitmap_zone" ? "L0+L1" : "L1", v.stored, n);
    bench("E_b_semijoin", caseid, n, variants, extra);
}

template<class T>
struct SortedBuild {
    Buf input, sorted, output, count, scratch;
    size_t sort_bytes = 0, unique_bytes = 0, n;
    explicit SortedBuild(const std::vector<Code>& keys) :
        sorted(keys.size() * sizeof(T)), output(keys.size() * sizeof(T)),
        count(sizeof(int)), n(keys.size()) {
        std::vector<T> typed(keys.begin(), keys.end());
        input = upload(typed);
        CK(cub::DeviceRadixSort::SortKeys(nullptr, sort_bytes, input.as<T>(),
                                         sorted.as<T>(), int(n)));
        CK(cub::DeviceSelect::Unique(nullptr, unique_bytes, sorted.as<T>(),
                                     output.as<T>(), count.as<int>(), int(n)));
        scratch = Buf(std::max(sort_bytes, unique_bytes));
    }
    void run() {
        CK(cub::DeviceRadixSort::SortKeys(scratch.p, sort_bytes, input.as<T>(),
                                         sorted.as<T>(), int(n)));
        CK(cub::DeviceSelect::Unique(scratch.p, unique_bytes, sorted.as<T>(),
                                     output.as<T>(), count.as<int>(), int(n)));
    }
    bool check(Buf& expected, size_t expected_n, Buf& error) {
        int actual;
        CK(cudaMemcpy(&actual, count.p, sizeof(actual), cudaMemcpyDeviceToHost));
        return actual == int(expected_n) && exact_array<T>(output, expected, expected_n, error);
    }
    size_t stored() const { return input.n + output.n + count.n; }
};
__global__ void bitmap_and(const Code* a, const Code* b, Code* result, size_t n) {
    for (size_t i = size_t(blockIdx.x) * blockDim.x + threadIdx.x;
         i < n; i += size_t(gridDim.x) * blockDim.x) result[i] = a[i] & b[i];
}
template<class T>
__global__ void intersection_flags(const T* a, size_t na, const T* b, size_t nb,
                                   unsigned char* flags) {
    for (size_t i = size_t(blockIdx.x) * blockDim.x + threadIdx.x;
         i < na; i += size_t(gridDim.x) * blockDim.x)
        flags[i] = contains(b, nb, a[i]);
}
template<class T>
struct SortedIntersection {
    Buf a, b, flags, output, count, scratch;
    size_t na, nb, scratch_bytes = 0;
    SortedIntersection(const std::vector<Code>& left, const std::vector<Code>& right) :
        flags(left.size()), output(left.size() * sizeof(T)), count(sizeof(int)),
        na(left.size()), nb(right.size()) {
        a = upload(std::vector<T>(left.begin(), left.end()));
        b = upload(std::vector<T>(right.begin(), right.end()));
        CK(cub::DeviceSelect::Flagged(nullptr, scratch_bytes, a.as<T>(),
              flags.as<unsigned char>(), output.as<T>(), count.as<int>(), int(na)));
        scratch = Buf(scratch_bytes);
    }
    void run() {
        intersection_flags<T><<<1024, 256>>>(a.as<T>(), na, b.as<T>(), nb,
                                             flags.as<unsigned char>());
        CK(cub::DeviceSelect::Flagged(scratch.p, scratch_bytes, a.as<T>(),
              flags.as<unsigned char>(), output.as<T>(), count.as<int>(), int(na)));
    }
    bool check(Buf& expected, size_t n, Buf& error) {
        int actual;
        CK(cudaMemcpy(&actual, count.p, sizeof(actual), cudaMemcpyDeviceToHost));
        return actual == int(n) && exact_array<T>(output, expected, n, error);
    }
    size_t stored() const { return a.n + b.n + output.n + count.n; }
};
} // namespace compressed_sj

inline void do_semijoin(int pct) {
    using namespace compressed_sj;
    require(pct == 3 || pct == 15 || pct == 50, "semijoin selectivity must be 3, 15, or 50");
    auto start = Clock::now();
    auto orders = load_codes("orders.o_orderkey", 8);
    auto dates = load_codes("orders.o_orderdate", 4);
    auto lines = load_codes("lineitem.l_orderkey", 8);
    double load_ms = elapsed(start);
    require(lines.size() == 59986052, "expected all 59,986,052 SF10 lineitem rows");
    require(orders.size() == 15000000, "expected 15,000,000 SF10 orders");
    require(*std::min_element(orders.begin(), orders.end()) > 0, "zero hash sentinel is an order key");
    start = Clock::now();
    auto sorted_dates = dates;
    std::sort(sorted_dates.begin(), sorted_dates.end());
    int low_percentile = (100 - pct) / 2;
    DateRange range = quantiles(sorted_dates, low_percentile, low_percentile + pct);
    auto selected = select_dates(orders, dates, range);
    size_t selected_rows = selected.size();
    selected = sorted_unique(std::move(selected));
    Code minimum = selected.front(), maximum = selected.back();
    auto bitmap_host = bitmap(selected, minimum, maximum);
    U oracle = 0;
    for (Code key : lines) oracle += key >= minimum && key <= maximum &&
        ((bitmap_host[(key-minimum) / 32] >> ((key-minimum) % 32)) & 1u);
    double oracle_ms = elapsed(start);
    start = Clock::now();
    Probe probe(lines);
    auto d_sorted = upload(selected);
    auto d_wide = upload(std::vector<U>(selected.begin(), selected.end()));
    auto d_bitmap = upload(bitmap_host);
    size_t capacity = power2(selected.size() * 2);
    Buf d_hash(capacity * sizeof(Code));
    CK(cudaMemset(d_hash.p, 0, d_hash.n));
    hash_insert<<<1024, 256>>>(d_sorted.as<Code>(), selected.size(), d_hash.as<Code>(), capacity - 1);
    size_t bloom_bits = power2(std::max(size_t(32), selected.size() * 10));
    std::vector<Code> bloom_host(bloom_bits / 32, 0);
    for (Code key : selected) {
        Code h = mix(key), step = mix(key ^ 0x9e3779b9u) | 1u;
        for (unsigned j = 0; j < 4; ++j) {
            size_t bit = (h + j * step) & (bloom_bits - 1);
            bloom_host[bit / 32] |= Code(1) << (bit % 32);
        }
    }
    auto d_bloom = upload(bloom_host);
    CK(cudaDeviceSynchronize());
    double encoding_ms = elapsed(start);
    Buf output(sizeof(U));
    SetView set{d_wide.as<U>(), d_sorted.as<Code>(), d_hash.as<Code>(),
                d_bitmap.as<Code>(), d_bloom.as<Code>(), selected.size(), capacity - 1,
                bloom_bits - 1, minimum, maximum};
    std::ostringstream extra;
    extra << ",\"dataset\":\"TPC-H SF10\",\"requested_selectivity_pct\":" << pct
          << ",\"order_rows\":" << orders.size() << ",\"selected_order_rows\":" << selected_rows
          << ",\"set_keys\":" << selected.size()
          << ",\"actual_selectivity\":" << double(selected_rows) / orders.size()
          << ",\"probe_rows\":" << lines.size() << ",\"expected_count\":" << oracle
          << ",\"probe_selectivity\":" << double(oracle) / lines.size()
          << ",\"date_low_inclusive_days\":" << range.low
          << ",\"date_high_exclusive_days\":" << range.high
          << ",\"date_epoch\":\"1970-01-01\",\"quantile_low_pct\":" << low_percentile
          << ",\"quantile_high_pct\":" << low_percentile + pct
          << ",\"load_wall_ms\":" << load_ms << ",\"cpu_selection_oracle_wall_ms\":" << oracle_ms
          << ",\"encoding_upload_wall_ms\":" << encoding_ms
          << ",\"for_layout\":\"per1024_lane_interleaved_u32_exact_min_max\""
          << ",\"plain_probe_bytes\":" << probe.plain.n
          << ",\"for_payload_bytes\":" << probe.packed.n
          << ",\"frame_bytes\":" << probe.frames.n
          << ",\"bitmap_bytes\":" << d_bitmap.n << ",\"bitmap_rank_bytes\":0"
          << ",\"bitmap_minimum\":" << minimum << ",\"bitmap_maximum\":" << maximum
          << ",\"hash_capacity\":" << capacity << ",\"hash_load_factor\":" << double(selected.size()) / capacity
          << ",\"hash_empty_sentinel\":0,\"bloom_hashes\":4,\"bloom_bits\":" << bloom_bits
          << ",\"bloom_exact_followup\":\"sorted_u32_binary_search\""
          << ",\"oracle\":\"full_CPU_count_every_sample\",\"output\":\"count\"";
    probe_group<false>(probe, lines.size(), set, d_bitmap.n, d_hash.n, d_bloom.n,
                       output, oracle, "sf10_s" + std::to_string(pct) + "_plain_u32", extra.str());
    probe_group<true>(probe, lines.size(), set, d_bitmap.n, d_hash.n, d_bloom.n,
                      output, oracle, "sf10_s" + std::to_string(pct) + "_for1024", extra.str());
}

inline void do_setops(int pct) {
    using namespace compressed_sj;
    auto start = Clock::now();
    auto orders = load_codes("orders.o_orderkey", 8);
    auto dates = load_codes("orders.o_orderdate", 4);
    double load_ms = elapsed(start);
    require(orders.size() == 15000000, "expected 15,000,000 SF10 orders");
    require(*std::min_element(orders.begin(), orders.end()) > 0, "zero hash sentinel is an order key");
    start = Clock::now();
    auto sorted_dates = dates;
    std::sort(sorted_dates.begin(), sorted_dates.end());
    int low = (100-pct)/2;
    DateRange ar = quantiles(sorted_dates, low, low+pct);
    DateRange br = quantiles(sorted_dates, low+pct/2, low+pct+pct/2);
    std::string caseid = "sf10_s"+std::to_string(pct);
    auto input_a = select_dates(orders, dates, ar);
    auto input_b = select_dates(orders, dates, br);
    auto a = sorted_unique(input_a), b = sorted_unique(input_b);
    std::vector<Code> intersection;
    intersection.reserve(std::min(a.size(), b.size()));
    // Independent CPU sorted-set oracle; compare every emitted GPU key and length.
    size_t ai = 0, bi = 0;
    while (ai < a.size() && bi < b.size()) {
        if (a[ai] < b[bi]) ++ai;
        else if (b[bi] < a[ai]) ++bi;
        else { intersection.push_back(a[ai]); ++ai; ++bi; }
    }
    Code minimum = std::min(a.front(), b.front()), maximum = std::max(a.back(), b.back());
    auto bitmap_a = bitmap(a, minimum, maximum), bitmap_b = bitmap(b, minimum, maximum);
    auto bitmap_intersection = bitmap(intersection, minimum, maximum);
    double oracle_ms = elapsed(start);
    start = Clock::now();
    auto d_input = upload(input_a), d_reference = upload(a);
    auto d_bitmap_reference = upload(bitmap_a);
    auto d_intersection_reference = upload(intersection);
    auto d_bitmap_intersection_reference = upload(bitmap_intersection);
    auto d_bitmap_a = upload(bitmap_a), d_bitmap_b = upload(bitmap_b);
    size_t capacity = power2(a.size() * 2);
    Buf bitmap_output(d_bitmap_a.n), hash_output(capacity * sizeof(Code));
    Buf error(sizeof(unsigned)), occupied(sizeof(U));
    SortedBuild<U> build64(input_a);
    SortedBuild<Code> build32(input_a);
    SortedIntersection<U> intersect64(a, b);
    SortedIntersection<Code> intersect32(a, b);
    CK(cudaDeviceSynchronize());
    double encoding_ms = elapsed(start);
    std::ostringstream extra;
    extra << ",\"dataset\":\"TPC-H SF10\",\"order_rows\":" << orders.size()
          << ",\"a_input_rows\":" << input_a.size() << ",\"b_input_rows\":" << input_b.size()
          << ",\"a_keys\":" << a.size() << ",\"b_keys\":" << b.size()
          << ",\"a_actual_selectivity\":" << double(input_a.size()) / orders.size()
          << ",\"b_actual_selectivity\":" << double(input_b.size()) / orders.size()
          << ",\"a_date_low_inclusive_days\":" << ar.low << ",\"a_date_high_exclusive_days\":" << ar.high
          << ",\"b_date_low_inclusive_days\":" << br.low << ",\"b_date_high_exclusive_days\":" << br.high
          << ",\"date_epoch\":\"1970-01-01\",\"intersection_keys\":" << intersection.size()
          << ",\"domain_minimum\":" << minimum << ",\"domain_maximum\":" << maximum << ",\"bitmap_words\":" << bitmap_a.size()
          << ",\"load_wall_ms\":" << load_ms << ",\"cpu_selection_oracle_wall_ms\":" << oracle_ms
          << ",\"encoding_upload_allocation_wall_ms\":" << encoding_ms
          << ",\"hash_capacity\":" << capacity << ",\"hash_load_factor\":" << double(a.size()) / capacity
          << ",\"hash_empty_sentinel\":0,\"build32_workspace_bytes\":" << build32.scratch.n + build32.sorted.n
          << ",\"build64_workspace_bytes\":" << build64.scratch.n + build64.sorted.n
          << ",\"intersect32_workspace_bytes\":" << intersect32.scratch.n + intersect32.flags.n
          << ",\"intersect64_workspace_bytes\":" << intersect64.scratch.n + intersect64.flags.n
          << ",\"oracle\":\"every_output_element_and_cardinality_every_sample\"";
    std::vector<Variant> builds;
    builds.push_back({"V0_u64", build64.stored(), 0,
        [&] { build64.run(); }, [&] { return build64.check(d_reference, a.size(), error); }});
    builds.push_back({"V1_u32", build32.stored(), 0,
        [&] { build32.run(); }, [&] { return build32.check(d_reference, a.size(), error); }});
    builds.push_back(builds.back()); builds.back().name = "V1_noise";
    builds.push_back({"bitmap_atomicOr_zero", d_input.n + bitmap_output.n, 0,
        [&] {
            CK(cudaMemsetAsync(bitmap_output.p, 0, bitmap_output.n));
            bitmap_insert<<<1024, 256>>>(d_input.as<Code>(), input_a.size(), bitmap_output.as<Code>(), minimum);
        }, [&] { return exact_array<Code>(bitmap_output, d_bitmap_reference, bitmap_a.size(), error); }});
    builds.push_back({"hash_zero_insert_u32", d_input.n + hash_output.n, 0,
        [&] {
            CK(cudaMemsetAsync(hash_output.p, 0, hash_output.n));
            hash_insert<<<1024, 256>>>(d_input.as<Code>(), input_a.size(), hash_output.as<Code>(), capacity - 1);
        }, [&] { return exact_hash(hash_output, d_reference, a.size(), error, occupied); }});
    for (auto& v : builds)
        metadata("E_b_set_build", caseid, v.name,
                 v.name == "bitmap_atomicOr_zero" ? "L1" : "L2_materialized_codes", v.stored, input_a.size());
    bench("E_b_set_build", caseid, input_a.size(), builds,
          extra.str() + ",\"sorted_build\":\"CUB_radix_sort_then_unique\",\"zeroing_included\":true");
    std::vector<Variant> intersections;
    intersections.push_back({"V0_u64", intersect64.stored(), 0,
        [&] { intersect64.run(); },
        [&] { return intersect64.check(d_intersection_reference, intersection.size(), error); }});
    intersections.push_back({"V1_u32", intersect32.stored(), 0,
        [&] { intersect32.run(); },
        [&] { return intersect32.check(d_intersection_reference, intersection.size(), error); }});
    intersections.push_back(intersections.back()); intersections.back().name = "V1_noise";
    intersections.push_back({"bitmap_AND", d_bitmap_a.n + d_bitmap_b.n + bitmap_output.n,
        double(d_bitmap_a.n + d_bitmap_b.n + bitmap_output.n),
        [&] { bitmap_and<<<1024, 256>>>(d_bitmap_a.as<Code>(), d_bitmap_b.as<Code>(),
                                        bitmap_output.as<Code>(), bitmap_a.size()); },
        [&] { return exact_array<Code>(bitmap_output, d_bitmap_intersection_reference,
                                        bitmap_intersection.size(), error); }});
    for (auto& v : intersections)
        metadata("E_b_set_intersection", caseid, v.name,
                 v.name == "bitmap_AND" ? "L0" : "L2_materialized_codes",
                 v.stored, a.size() + b.size());
    bench("E_b_set_intersection", caseid, a.size() + b.size(),
          intersections, extra.str() + ",\"sorted_intersection\":\"binary_search_flags_then_CUB_stable_select\","
          "\"bitmap_output\":\"exact_membership_words_no_expansion\"");
}
