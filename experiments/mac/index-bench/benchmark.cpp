// Single-thread CPU benchmark. Payload enumeration uses a count/checksum sink,
// not output-tuple materialization. Input generation and correctness are untimed.
// LE-style here means one lookup-retain-expand kernel, not a full multiway
// semijoin plan. Every measured query mode has one identical untimed warmup.
#include "indexes.hpp"
#include "target/artifacts/unordered_dense/include/ankerl/unordered_dense.h"

#include <algorithm>
#include <array>
#include <charconv>
#include <chrono>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <iomanip>
#include <iostream>
#include <limits>
#include <stdexcept>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

namespace bench {
using Clock = std::chrono::steady_clock;
using Key = uint64_t;

uint64_t mix(uint64_t x) {
    x ^= x >> 30;
    x *= UINT64_C(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x *= UINT64_C(0x94d049bb133111eb);
    return x ^ (x >> 31);
}

struct Random {
    uint64_t state;
    uint64_t next() { return mix(state += UINT64_C(0x9e3779b97f4a7c15)); }
    size_t bounded(size_t n) {
        if (!n) throw std::runtime_error("zero random bound");
        const uint64_t threshold = -uint64_t(n) % uint64_t(n);
        uint64_t v;
        do { v = next(); } while (v < threshold);
        return size_t(v % n);
    }
};

template<class T> void shuffle(std::vector<T>& v, Random& r) {
    for (size_t i = v.size(); i > 1; --i) std::swap(v[i - 1], v[r.bounded(i)]);
}

struct Config {
    size_t keys = 1000000;
    size_t probes = 262144;
    size_t reps = 5;
    uint64_t seed = 1;
    size_t fanout = 4;
    double hit_rate = 0.5;
    std::string distribution = "uniform";
    std::string mode = "all";
    std::string filters = "none";
    bool self_test = false;
};

uint64_t integer(std::string_view text, const std::string& option) {
    uint64_t result = 0;
    auto [end, error] = std::from_chars(text.data(), text.data() + text.size(), result);
    if (error != std::errc{} || end != text.data() + text.size() || text.empty())
        throw std::runtime_error("invalid integer for " + option);
    return result;
}

Config parse(int argc, char** argv) {
    Config c;
    for (int i = 1; i < argc; ++i) {
        const std::string arg = argv[i];
        if (arg == "--self-test") { c.self_test = true; continue; }
        if (arg == "--help") {
            std::cout << "Single-thread CPU index/join benchmark\n"
                "--keys N --probes N --reps N --seed N --fanout N\n"
                "--distribution uniform|clustered --hit-rate [0,1]\n"
                "--mode all|lookup|expand|retained|sorted|merge\n"
                "--filters none|bloom --self-test\n"
                "sorted emits sorted_lookup, sorted_expand, sorted_retained.\n"
                "query_ms excludes sorting; total_ms adds sorting exactly once.\n"
                "payload_enumerated means a count/checksum sink, not tuple materialization.\n"
                "lookup rows report independently verified join output outside timing.\n";
            std::exit(0);
        }
        if (i + 1 == argc) throw std::runtime_error("missing value for " + arg);
        const std::string value = argv[++i];
        if (arg == "--keys") c.keys = integer(value, arg);
        else if (arg == "--probes") c.probes = integer(value, arg);
        else if (arg == "--reps") c.reps = integer(value, arg);
        else if (arg == "--seed") c.seed = integer(value, arg);
        else if (arg == "--fanout") c.fanout = integer(value, arg);
        else if (arg == "--distribution") c.distribution = value;
        else if (arg == "--mode") c.mode = value;
        else if (arg == "--filters") c.filters = value;
        else if (arg == "--hit-rate") {
            size_t used = 0;
            c.hit_rate = std::stod(value, &used);
            if (used != value.size()) throw std::runtime_error("invalid hit rate");
        } else throw std::runtime_error("unsupported argument: " + arg);
    }
    if (!c.reps || !c.fanout) throw std::runtime_error("reps and fanout must be positive");
    if (!std::isfinite(c.hit_rate) || c.hit_rate < 0 || c.hit_rate > 1)
        throw std::runtime_error("hit rate must be in [0,1]");
    if (c.distribution != "uniform" && c.distribution != "clustered")
        throw std::runtime_error("unsupported distribution: " + c.distribution);
    if (c.mode != "all" && c.mode != "lookup" && c.mode != "expand" &&
        c.mode != "retained" && c.mode != "sorted" && c.mode != "merge")
        throw std::runtime_error("unsupported mode: " + c.mode);
    if (c.filters != "none" && c.filters != "bloom")
        throw std::runtime_error("unsupported filters: " + c.filters);
    if (c.fanout > std::numeric_limits<size_t>::max() / 2)
        throw std::runtime_error("fanout is too large");
    // The clustered generator has 64 disjoint clusters of 2^23 even keys.
    if (c.distribution == "clustered" && c.keys > (size_t{1} << 28))
        throw std::runtime_error("clustered key count exceeds generator safety bound");
    return c;
}

struct Data {
    std::vector<Key> keys;
    std::vector<size_t> offsets;
    std::vector<uint64_t> payload;
    std::vector<Key> queries;
    size_t shared_bytes() const {
        return keys.capacity() * sizeof(Key) + offsets.capacity() * sizeof(size_t) +
            payload.capacity() * sizeof(uint64_t);
    }
};

void groups(Data& d, size_t fanout, uint64_t seed) {
    Random lengths{seed ^ UINT64_C(0x2e92b741a6839de5)};
    Random payload{seed ^ UINT64_C(0x17ec5a4f0896d32b)};
    d.offsets.resize(d.keys.size() + 1);
    const size_t low = std::max(size_t{1}, fanout / 2);
    const size_t high = fanout + fanout / 2;
    for (size_t i = 0; i < d.keys.size(); ++i) {
        const size_t n = low + lengths.bounded(high - low + 1);
        if (n > std::numeric_limits<size_t>::max() - d.offsets[i])
            throw std::runtime_error("group size overflow");
        d.offsets[i + 1] = d.offsets[i] + n;
    }
    d.payload.resize(d.offsets.back());
    for (auto& value : d.payload) value = payload.next();
}

Data generate(const Config& c) {
    Data d;
    Random keys{c.seed ^ UINT64_C(0x3c94e17b20af856d)};
    d.keys.reserve(c.keys);
    do {
        while (d.keys.size() < c.keys) {
            const uint64_t value = c.distribution == "uniform"
                ? keys.next() & UINT64_C(0x7ffffffffffffffe)
                : (uint64_t(keys.bounded(64)) << 56) | (keys.next() & UINT64_C(0xfffffe));
            d.keys.push_back(value);
        }
        std::sort(d.keys.begin(), d.keys.end());
        d.keys.erase(std::unique(d.keys.begin(), d.keys.end()), d.keys.end());
    } while (d.keys.size() < c.keys);
    groups(d, c.fanout, c.seed);
    Random queries{c.seed ^ UINT64_C(0xa1f09537b8e26dc4)};
    const size_t hits = c.keys ? size_t(std::round(static_cast<long double>(c.probes) * c.hit_rate)) : 0;
    d.queries.resize(c.probes);
    for (size_t i = 0; i < c.probes; ++i) {
        const Key base = d.keys.empty() ? queries.next() & ~Key{1} : d.keys[queries.bounded(d.keys.size())];
        d.queries[i] = i < hits ? base : base | Key{1};
    }
    Random order{c.seed ^ UINT64_C(0x84715ce6290db3fa)};
    shuffle(d.queries, order);
    return d;
}

struct StrongHash {
    size_t operator()(uint64_t key) const { return size_t(mix(key)); }
};

struct StdHash {
    const std::vector<Key>& keys;
    std::unordered_map<Key, size_t, StrongHash> table;
    explicit StdHash(const std::vector<Key>& k) : keys(k) {
        table.reserve(k.size());
        for (size_t i = 0; i < k.size(); ++i) table.emplace(k[i], i);
    }
    size_t find(Key k) const {
        auto it = table.find(k);
        return it == table.end() ? keys.size() : it->second;
    }
    size_t bytes() const {
        // Estimate: bucket pointers plus pair, link and cached hash per node;
        // excludes allocator metadata/padding, and is not an RSS measurement.
        return table.bucket_count() * sizeof(void*) + table.size() *
            (sizeof(decltype(table)::value_type) + 2 * sizeof(void*));
    }
    static std::string name() { return "std_unordered_map_splitmix64_reserved"; }
    static std::string memory_kind() { return "estimated_buckets_nodes_excludes_allocator"; }
};

struct DenseHash {
    const std::vector<Key>& keys;
    ankerl::unordered_dense::map<Key, size_t> table;
    explicit DenseHash(const std::vector<Key>& k) : keys(k) {
        table.reserve(k.size());
        for (size_t i = 0; i < k.size(); ++i) table.emplace(k[i], i);
    }
    size_t find(Key k) const {
        auto it = table.find(k);
        return it == table.end() ? keys.size() : it->second;
    }
    size_t bytes() const {
        return table.index_bytes() + table.values().capacity() * sizeof(decltype(table)::value_type);
    }
    static std::string name() { return "ankerl_unordered_dense_reserved"; }
    static std::string memory_kind() { return "allocated_index_values_excludes_allocator"; }
};

// Local implementation, not a port of an artifact. Eight double-hashed bit
// probes stay in one aligned 64-byte cache block, with about 12 bits/key.
class BlockedBloom {
    struct alignas(64) Block { std::array<uint64_t, 8> words{}; };
    std::vector<Block> blocks;
    static uint64_t hash(Key key) { return mix(key ^ UINT64_C(0xc36a5f92817b04ed)); }
public:
    explicit BlockedBloom(const std::vector<Key>& keys)
        : blocks(keys.empty() ? 0 : (keys.size() / 128 * 3 + (keys.size() % 128 * 12 + 511) / 512)) {
        for (Key key : keys) {
            const uint64_t h = hash(key);
            auto& block = blocks[mix(h) % blocks.size()];
            const unsigned step = unsigned(h >> 32) | 1U;
            for (unsigned i = 0; i < 8; ++i) {
                const unsigned bit = (unsigned(h) + i * step) & 511U;
                block.words[bit >> 6] |= UINT64_C(1) << (bit & 63U);
            }
        }
    }
    bool maybe(Key key) const {
        if (blocks.empty()) return false;
        const uint64_t h = hash(key);
        const auto& block = blocks[mix(h) % blocks.size()];
        const unsigned step = unsigned(h >> 32) | 1U;
        for (unsigned i = 0; i < 8; ++i) {
            const unsigned bit = (unsigned(h) + i * step) & 511U;
            if (!(block.words[bit >> 6] & (UINT64_C(1) << (bit & 63U)))) return false;
        }
        return true;
    }
    size_t bytes() const { return blocks.capacity() * sizeof(Block); }
};

template<class T> std::string memory_kind() {
    if constexpr (requires { T::memory_kind(); }) return T::memory_kind();
    else return "reported_auxiliary_excludes_allocator";
}

template<class T> std::string unsupported_reason(const std::vector<Key>& keys) {
    if constexpr (requires { T::unsupported_reason(keys); }) return T::unsupported_reason(keys);
    else return {};
}

template<class Base> struct Filtered {
    const std::vector<Key>& keys;
    Base base;
    BlockedBloom bloom;
    explicit Filtered(const std::vector<Key>& k) : keys(k), base(k), bloom(k) {}
    size_t find(Key k) const { return bloom.maybe(k) ? base.find(k) : keys.size(); }
    size_t bytes() const { return base.bytes() + bloom.bytes(); }
    static std::string name() { return Base::name() + "+local_blocked_bloom12_k8"; }
    static std::string memory_kind() { return bench::memory_kind<Base>(); }
    static std::string unsupported_reason(const std::vector<Key>& k) {
        return bench::unsupported_reason<Base>(k);
    }
};

template<class F> void index_types(bool filters, F&& f) {
    idxbench::sorted_index_types(f);
    f.template operator()<StdHash>();
    f.template operator()<DenseHash>();
    if (filters) {
        f.template operator()<Filtered<idxbench::ChtDefault>>();
        f.template operator()<Filtered<DenseHash>>();
    }
}

struct Result {
    uint64_t hits = 0;
    uint64_t ranks = 0;
    uint64_t count = 0;
    uint64_t checksum = 0;
};

void rank_sink(Result& out, size_t rank, size_t missing) {
    // Unsigned modular sums are order independent, retaining duplicate probes.
    out.ranks += uint64_t(rank) + 1;
    out.hits += rank != missing;
}

void expand_one(Result& out, const Data& d, size_t rank) {
    if (rank == d.keys.size()) return;
    const auto begin = d.offsets[rank];
    const auto end = d.offsets[rank + 1];
    out.count += end - begin;
    for (size_t j = begin; j < end; ++j) out.checksum += d.payload[j];
}

void consume(const Result& out) {
#if defined(__clang__) || defined(__GNUC__)
    asm volatile("" : : "g"(&out) : "memory");
#else
    static volatile uint64_t sink;
    sink = out.hits ^ out.ranks ^ out.count ^ out.checksum;
#endif
}

void require(bool condition, const std::string& message) {
    if (!condition) throw std::runtime_error(message);
}

void check_result(const Result& got, const Result& expected, bool expanded, const std::string& where) {
    require(got.hits == expected.hits && got.ranks == expected.ranks, where + ": rank checksum/hit mismatch");
    if (expanded) require(got.count == expected.count && got.checksum == expected.checksum,
        where + ": payload count/checksum mismatch");
}

size_t reference_rank(const Data& d, Key q) {
    const auto it = std::lower_bound(d.keys.begin(), d.keys.end(), q);
    return it != d.keys.end() && *it == q ? size_t(it - d.keys.begin()) : d.keys.size();
}

Result reference(const Data& d, const std::vector<Key>& queries) {
    Result r;
    for (Key q : queries) {
        const size_t rank = reference_rank(d, q);
        rank_sink(r, rank, d.keys.size());
        expand_one(r, d, rank);
    }
    return r;
}

template<class Index> Result verify_index(const Index& index, const Data& d,
                                          const std::vector<Key>& queries, const Result& expected) {
    Result r;
    for (Key q : queries) {
        const size_t rank = index.find(q);
        require(rank == reference_rank(d, q), Index::name() + ": exact rank mismatch for " + std::to_string(q));
        rank_sink(r, rank, d.keys.size());
        expand_one(r, d, rank);
    }
    check_result(r, expected, true, Index::name() + " verification");
    return r;
}

template<class Index, bool Expand> Result probe(const Index& index, const Data& d,
                                               const std::vector<Key>& queries) {
    Result out;
    for (Key q : queries) {
        const size_t rank = index.find(q);
        rank_sink(out, rank, d.keys.size());
        if constexpr (Expand) expand_one(out, d, rank);
    }
    consume(out);
    return out;
}

template<class Index> Result retain(const Index& index, const Data& d,
                                    const std::vector<Key>& queries, std::vector<size_t>& witness) {
    Result out;
    for (size_t i = 0; i < queries.size(); ++i) {
        const size_t rank = index.find(queries[i]);
        witness[i] = rank; // Allocation is untimed; these writes are timed.
        rank_sink(out, rank, d.keys.size());
    }
    consume(out);
    return out;
}

void expand_retained(Result& out, const Data& d, const std::vector<size_t>& witness) {
    for (size_t rank : witness) expand_one(out, d, rank);
    consume(out);
}

template<bool Verify = false> Result merge(const Data& d, const std::vector<Key>& queries) {
    Result out;
    size_t rank = 0;
    for (Key q : queries) {
        while (rank < d.keys.size() && d.keys[rank] < q) ++rank;
        const size_t found = rank < d.keys.size() && d.keys[rank] == q ? rank : d.keys.size();
        if constexpr (Verify)
            require(found == reference_rank(d, q), "sort_merge: exact rank mismatch for " + std::to_string(q));
        rank_sink(out, found, d.keys.size());
        // Each occurrence of a repeated query expands its complete duplicate group.
        expand_one(out, d, found);
    }
    consume(out);
    return out;
}

double milliseconds(Clock::time_point start, Clock::time_point end) {
    return std::chrono::duration<double, std::milli>(end - start).count();
}

struct Stats { double median = 0; double min = 0; };
Stats stats(std::vector<double> values) {
    if (values.empty()) return {};
    std::sort(values.begin(), values.end());
    const size_t n = values.size();
    return {n % 2 ? values[n / 2] : (values[n / 2 - 1] + values[n / 2]) / 2, values.front()};
}

struct Prepared {
    std::vector<Key> sorted;
    std::vector<double> sort_ms;
};

Prepared prepare(const Data& d, size_t reps, bool needed) {
    Prepared p;
    if (!needed) return p;
    p.sorted.resize(d.queries.size());
    for (size_t rep = 0; rep < reps; ++rep) {
        // Reset outside the timer. The reported sort is in-place preprocessing;
        // preserving the original query buffer requires separately reported memory.
        std::copy(d.queries.begin(), d.queries.end(), p.sorted.begin());
        const auto begin = Clock::now();
        std::sort(p.sorted.begin(), p.sorted.end());
        const auto end = Clock::now();
        p.sort_ms.push_back(milliseconds(begin, end));
    }
    return p;
}

struct FilterStats {
    size_t bytes = 0;
    uint64_t negatives = 0;
    uint64_t positives = 0;
    uint64_t false_positives = 0;
};

template<class Index> FilterStats filter_stats(const Index& index, const Data& d) {
    FilterStats result;
    if constexpr (requires { index.bloom; }) {
        result.bytes = index.bloom.bytes();
        for (Key q : d.queries) {
            const bool actual = reference_rank(d, q) != d.keys.size();
            const bool positive = index.bloom.maybe(q);
            require(!actual || positive, Index::name() + ": filter false negative");
            result.negatives += !actual;
            result.positives += positive;
            result.false_positives += positive && !actual;
        }
    }
    return result;
}

struct Measurements {
    std::vector<double> query;
    std::vector<double> total;
    std::vector<double> search;
    std::vector<double> expansion;
};

std::string csv(const std::string& text) {
    std::string escaped = "\"";
    for (char ch : text) {
        if (ch == '"') escaped += '"';
        escaped += ch;
    }
    return escaped + '"';
}

void header() {
    std::cout << "index,mode,threads,keys,probes,reps,seed,distribution,requested_hit_rate,actual_hit_rate,fanout,payload_rows,"
        "build_ms,build_min_ms,query_ms,query_min_ms,total_ms,total_min_ms,ns_per_probe,min_ns_per_probe,"
        "total_ns_per_probe,total_min_ns_per_probe,sorting_ms,sorting_min_ms,retained_search_ms,retained_search_min_ms,"
        "retained_expand_ms,retained_expand_min_ms,auxiliary_index_bytes,index_memory_kind,shared_bytes,query_bytes,"
        "sorted_query_bytes,witness_bytes,matched_probes,rank_checksum,output_count,output_checksum,payload_enumerated,"
        "filter,filter_bytes,filter_bits_per_key,filter_hashes,filter_negative_queries,filter_positive_queries,"
        "filter_false_positives,filter_false_positive_rate,status,unsupported_reason\n";
}

void row(const Config& c, const Data& d, const Prepared& p, const std::string& index,
         const std::string& mode, const std::vector<double>& builds, const Measurements& m,
         size_t bytes, const std::string& memory, const FilterStats& f, const Result& result,
         const std::string& unsupported = {}) {
    const bool sorted = mode.starts_with("sorted_") || mode == "merge";
    const bool retained = mode == "retained" || mode == "sorted_retained";
    const bool enumerated = unsupported.empty() && mode != "lookup" && mode != "sorted_lookup";
    const auto build = stats(builds);
    const auto query = stats(m.query);
    const auto total = stats(m.total);
    const auto sorting = sorted ? stats(p.sort_ms) : Stats{};
    const auto search = stats(m.search);
    const auto expansion = stats(m.expansion);
    const double per_probe = c.probes ? 1e6 / double(c.probes) : 0;
    const bool filtered = index.find("+local_blocked_bloom") != std::string::npos;
    std::cout << csv(index) << ',' << mode << ",1," << d.keys.size() << ',' << d.queries.size() << ',' << c.reps << ','
        << c.seed << ',' << c.distribution << ',' << c.hit_rate << ','
        << (c.probes ? double(result.hits) / c.probes : 0) << ',' << c.fanout << ',' << d.payload.size() << ','
        << build.median << ',' << build.min << ',' << query.median << ',' << query.min << ','
        << total.median << ',' << total.min << ',' << query.median * per_probe << ',' << query.min * per_probe << ','
        << total.median * per_probe << ',' << total.min * per_probe << ',' << sorting.median << ',' << sorting.min << ','
        << search.median << ',' << search.min << ',' << expansion.median << ',' << expansion.min << ','
        << bytes << ',' << memory << ',' << d.shared_bytes() << ',' << d.queries.capacity() * sizeof(Key) << ','
        << (sorted ? p.sorted.capacity() * sizeof(Key) : 0) << ',' << (retained ? c.probes * sizeof(size_t) : 0) << ','
        << result.hits << ',' << result.ranks << ',' << result.count << ',' << result.checksum << ',' << enumerated << ','
        << (filtered ? "local_cache_blocked_bloom" : "none") << ',' << f.bytes << ','
        << (d.keys.empty() ? 0 : double(f.bytes) * 8 / d.keys.size()) << ',' << (filtered ? 8 : 0) << ','
        << f.negatives << ',' << f.positives << ',' << f.false_positives << ','
        << (f.negatives ? double(f.false_positives) / f.negatives : 0) << ','
        << (unsupported.empty() ? "ok" : "unsupported") << ',' << csv(unsupported) << '\n';
}

std::vector<std::string> modes(const Config& c) {
    if (c.mode == "merge") return {};
    if (c.mode == "sorted") return {"sorted_lookup", "sorted_expand", "sorted_retained"};
    if (c.mode == "all") return {"lookup", "expand", "retained", "sorted_lookup", "sorted_expand", "sorted_retained"};
    return {c.mode};
}

template<class Index> void benchmark_index(const Config& c, const Data& d, const Prepared& p,
                                          const Result& expected) {
    std::cerr << "index " << Index::name() << '\n';
    const auto selected = modes(c);
    const auto unsupported = unsupported_reason<Index>(d.keys);
    if (!unsupported.empty()) {
        const double nan = std::numeric_limits<double>::quiet_NaN();
        const Measurements unavailable{{nan}, {nan}, {nan}, {nan}};
        for (const auto& mode : selected)
            row(c, d, p, Index::name(), mode, {nan}, unavailable, 0, "unavailable", {}, expected, unsupported);
        std::cerr << "unsupported " << Index::name() << ": " << unsupported << '\n';
        return;
    }
    std::vector<double> builds;
    std::vector<Measurements> measured(selected.size());
    std::vector<size_t> witness;
    if (c.mode == "all" || c.mode == "retained" || c.mode == "sorted") witness.resize(d.queries.size());
    size_t bytes = 0;
    FilterStats filters;
    for (size_t rep = 0; rep < c.reps; ++rep) {
        // Construct directly: no separately allocated wrapper is charged.
        const auto begin = Clock::now();
        Index index(d.keys);
        const auto end = Clock::now();
        builds.push_back(milliseconds(begin, end));
        bytes = index.bytes();
        if (rep == 0) {
            verify_index(index, d, d.queries, expected);
            if (!p.sort_ms.empty()) verify_index(index, d, p.sorted, expected);
            filters = filter_stats(index, d);
        }
        for (size_t mode_id = 0; mode_id < selected.size(); ++mode_id) {
            const auto& mode = selected[mode_id];
            const bool sorted = mode.starts_with("sorted_");
            const bool retained = mode == "retained" || mode == "sorted_retained";
            const bool expanded = mode != "lookup" && mode != "sorted_lookup";
            const auto& queries = sorted ? p.sorted : d.queries;
            auto& measurement = measured[mode_id];
            Result warmup;
            if (retained) {
                warmup = retain(index, d, queries, witness);
                expand_retained(warmup, d, witness);
            } else if (expanded) {
                warmup = probe<Index, true>(index, d, queries);
            } else {
                warmup = probe<Index, false>(index, d, queries);
            }
            check_result(warmup, expected, expanded, Index::name() + "/" + mode + " warmup");
            Result result;
            const auto start = Clock::now();
            if (retained) {
                result = retain(index, d, queries, witness);
                const auto split = Clock::now();
                expand_retained(result, d, witness);
                const auto finish = Clock::now();
                measurement.query.push_back(milliseconds(start, finish));
                measurement.search.push_back(milliseconds(start, split));
                measurement.expansion.push_back(milliseconds(split, finish));
            } else {
                if (expanded) result = probe<Index, true>(index, d, queries);
                else result = probe<Index, false>(index, d, queries);
                const auto finish = Clock::now();
                measurement.query.push_back(milliseconds(start, finish));
            }
            check_result(result, expected, expanded, Index::name() + "/" + mode);
            if (retained) {
                for (size_t i = 0; i < queries.size(); ++i)
                    require(witness[i] == reference_rank(d, queries[i]), Index::name() + ": retained witness mismatch");
            }
            measurement.total.push_back(measurement.query.back() + (sorted ? p.sort_ms[rep] : 0));
        }
    }
    for (size_t i = 0; i < selected.size(); ++i)
        row(c, d, p, Index::name(), selected[i], builds, measured[i], bytes, memory_kind<Index>(), filters, expected);
}

void benchmark_merge(const Config& c, const Data& d, const Prepared& p, const Result& expected) {
    check_result(merge<true>(d, p.sorted), expected, true, "sort_merge verification");
    Measurements measured;
    for (size_t rep = 0; rep < c.reps; ++rep) {
        check_result(merge(d, p.sorted), expected, true, "sort_merge warmup");
        const auto start = Clock::now();
        const auto result = merge(d, p.sorted);
        const auto finish = Clock::now();
        check_result(result, expected, true, "sort_merge");
        measured.query.push_back(milliseconds(start, finish));
        measured.total.push_back(measured.query.back() + p.sort_ms[rep]);
    }
    row(c, d, p, "sorted_merge_join", "merge", {}, measured, 0, "none", {}, expected);
}

void check_case(const Data& d) {
    const auto expected = reference(d, d.queries);
    auto sorted = d.queries;
    std::sort(sorted.begin(), sorted.end());
    index_types(true, [&]<class Index>() {
        const auto unsupported = unsupported_reason<Index>(d.keys);
        if (!unsupported.empty()) {
            bool rejected = false;
            try { Index index(d.keys); }
            catch (const std::domain_error&) { rejected = true; }
            require(rejected, Index::name() + ": unsupported domain was not rejected");
            std::cerr << "self-test unsupported " << Index::name() << ": " << unsupported << '\n';
            return;
        }
        Index index(d.keys);
        verify_index(index, d, d.queries, expected);
        verify_index(index, d, sorted, expected);
        filter_stats(index, d);
        for (const auto* queries : std::array<const std::vector<Key>*, 2>{&d.queries, &sorted}) {
            check_result(probe<Index, false>(index, d, *queries), expected, false, Index::name() + " lookup");
            check_result(probe<Index, true>(index, d, *queries), expected, true, Index::name() + " expand");
            std::vector<size_t> witness(queries->size());
            auto result = retain(index, d, *queries, witness);
            for (size_t i = 0; i < queries->size(); ++i)
                require(witness[i] == reference_rank(d, (*queries)[i]), Index::name() + " witness");
            expand_retained(result, d, witness);
            check_result(result, expected, true, Index::name() + " retained");
        }
    });
    check_result(merge<true>(d, sorted), expected, true, "sort_merge self-test");
}

void self_test() {
    const Key max = std::numeric_limits<Key>::max();
    const std::vector<std::vector<Key>> cases = {
        {}, {0}, {2}, {max}, {0, 2}, {0, 2, 8}, {0, 32, 64},
        {0, 2, 8, 100, 102}, {0, 2, max - 3, max - 1},
        {2, max}, {0, 2, max - 1, max}
    };
    for (const auto& keys : cases) {
        Data d;
        d.keys = keys;
        d.queries = {0, 0, 1, 2, 2, 3, 6, 8, 9, 99, 100, 101, 102, 103, max - 3, max - 2, max - 1, max};
        groups(d, 5, 31);
        check_case(d);
    }
    for (uint64_t seed = 1; seed <= 8; ++seed) {
        Config c;
        c.keys = size_t(seed * 37);
        c.probes = 203;
        c.fanout = size_t(seed % 5 + 1);
        c.seed = seed;
        c.hit_rate = double(seed % 3) / 2;
        c.distribution = seed % 2 ? "uniform" : "clustered";
        check_case(generate(c));
    }
    Data empty_queries;
    empty_queries.keys = {0, 2};
    groups(empty_queries, 3, 9);
    check_case(empty_queries);
    std::cerr << "self-test passed: every index, filter, rank, payload sink, retained witness, and merge\n";
}

int run(int argc, char** argv) {
    const Config c = parse(argc, argv);
    if (c.self_test) { self_test(); return 0; }
    std::cerr << "single-thread CPU; generating keys=" << c.keys << " probes=" << c.probes
              << " distribution=" << c.distribution << '\n';
    const Data d = generate(c);
    const auto expected = reference(d, d.queries);
    const bool needs_sort = c.mode == "all" || c.mode == "sorted" || c.mode == "merge";
    const auto prepared = prepare(d, c.reps, needs_sort);
    std::cout << std::setprecision(12);
    header();
    if (c.mode != "merge") index_types(c.filters == "bloom", [&]<class Index>() {
        benchmark_index<Index>(c, d, prepared, expected);
    });
    if (c.mode == "all" || c.mode == "merge") benchmark_merge(c, d, prepared, expected);
    return 0;
}
} // namespace bench

int main(int argc, char** argv) {
    try { return bench::run(argc, argv); }
    catch (const std::exception& e) {
        std::cerr << "benchmark error: " << e.what() << '\n';
        return 1;
    }
}
