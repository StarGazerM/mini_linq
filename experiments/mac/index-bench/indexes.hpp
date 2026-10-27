#pragma once

#include <algorithm>
#include <bit>
#include <cstddef>
#include <cstdint>
#include <iostream>  // Upstream search_algo.h and CHT builder use stream names.
#include <iterator>
#include <limits>
#include <optional>
#include <stdexcept>
#include <string>
#include <vector>

#include "pgm_index.h"  // Generated copy with recorded safe-projection patch.
#include "target/artifacts/CHT/include/cht/builder.h"
#include "target/artifacts/RadixSpline/include/rs/builder.h"

#if defined(__aarch64__) && defined(__ARM_NEON)
#include <arm_neon.h>
#endif

namespace idxbench {

// Contract: input is sorted, distinct, immutable, and outlives the index.
// find returns the exact distinct-key rank, or keys.size() for every miss.
// bytes reports logical retained auxiliary storage: the adapter object plus
// populated index/directory entries, excluding shared keys/payload, unused
// vector capacity, allocator overhead, and temporary construction workspace.
// Artifact checkouts remain unchanged; PGM uses the recorded projection fix.
// Unsupported domains are checked at build time; constructors throw domain_error.
namespace detail {

inline bool outside(const std::vector<uint64_t>& keys, uint64_t key) {
    return keys.empty() || key < keys.front() || key > keys.back();
}

template<bool Branchless = false>
inline size_t exact_search(const std::vector<uint64_t>& keys,
                           size_t lo, size_t hi, uint64_t key) {
    hi = std::min(hi, keys.size());
    lo = std::min(lo, hi);
    if (lo == hi) return keys.size();
    const auto first = keys.begin() + lo;
    const auto last = keys.begin() + hi;
    const auto found = [&] {
        if constexpr (Branchless)
            return search::lower_bound_branchless(first, last, key);
        else
            return std::lower_bound(first, last, key);
    }();
    // The artifact branchless function returns the final candidate, not end(),
    // when key exceeds the window. Equality, not its name, defines correctness.
    return found != last && *found == key
               ? static_cast<size_t>(found - keys.begin())
               : keys.size();
}

template<size_t Epsilon, bool Branchless, size_t Threshold>
class PgmStorage : public pgm::PGMIndex<uint64_t, Epsilon, 4, Branchless, Threshold> {
    using Base = pgm::PGMIndex<uint64_t, Epsilon, 4, Branchless, Threshold>;
public:
    using Base::Base;

    size_t dynamic_bytes() const {
        // Upstream size_in_bytes() omits levels_segment_count. Access protected
        // vectors directly rather than copying via the artifact's getters.
        return this->segments.size() * sizeof(typename Base::Segment) +
               this->levels_offsets.size() * sizeof(size_t) +
               this->levels_segment_count.size() * sizeof(size_t);
    }
};

}  // namespace detail

class StdLowerBound {
public:
    explicit StdLowerBound(const std::vector<uint64_t>& keys) : keys_(keys) {}
    size_t find(uint64_t key) const {
        return detail::exact_search(keys_, 0, keys_.size(), key);
    }
    size_t bytes() const { return sizeof(*this); }
    static std::string name() { return "std_lower_bound"; }
    static std::string unsupported_reason(const std::vector<uint64_t>&) { return {}; }
private:
    const std::vector<uint64_t>& keys_;
};

class ArtifactBranchless {
public:
    explicit ArtifactBranchless(const std::vector<uint64_t>& keys) : keys_(keys) {}
    size_t find(uint64_t key) const {
        return detail::exact_search<true>(keys_, 0, keys_.size(), key);
    }
    size_t bytes() const { return sizeof(*this); }
    static std::string name() { return "bench_search_branchless_exact"; }
    static std::string unsupported_reason(const std::vector<uint64_t>&) { return {}; }
private:
    const std::vector<uint64_t>& keys_;
};

template<size_t Epsilon, bool Branchless, size_t Threshold>
class PgmIndex {
public:
    explicit PgmIndex(const std::vector<uint64_t>& keys) : keys_(keys) {
        if (const auto reason = unsupported_reason(keys); !reason.empty())
            throw std::domain_error(reason);
        if (keys.size() <= 1) return;
        // UINT64_MAX is reserved by upstream. It is an exact, separately handled
        // final rank, not a dropped input row. All other keys enter the model.
        const size_t indexed = keys.size() - (keys.back() == UINT64_MAX);
        // make_segmentation reads end-2 even for a one-key model.
        if (indexed == 1) return;
        index_.emplace(keys.begin(), keys.begin() + indexed);
    }

    size_t find(uint64_t key) const {
        if (detail::outside(keys_, key)) return keys_.size();
        if (keys_.size() == 1) return 0;
        if (key == UINT64_MAX) return keys_.size() - 1;
        if (!index_) return key == keys_.front() ? 0 : keys_.size();
        const auto bound = index_->search(key);
        return detail::exact_search<Branchless>(keys_, bound.lo, bound.hi, key);
    }

    size_t bytes() const {
        return sizeof(*this) + (index_ ? index_->dynamic_bytes() : 0);
    }
    static std::string name() {
        return std::string(Branchless ? "pgmpp" : "pgm_original") +
               "_epsilon" + std::to_string(Epsilon) + "_recursive4_" +
               (Branchless ? "branchless" : "branchy") +
               "_threshold" + std::to_string(Threshold) + "_safe_projection";
    }
    static std::string unsupported_reason(const std::vector<uint64_t>& keys) {
        if (keys.size() > std::numeric_limits<uint32_t>::max())
            return "PGM artifact uses uint32_t segment intercepts";
        return {};
    }
private:
    const std::vector<uint64_t>& keys_;
    std::optional<detail::PgmStorage<Epsilon, Branchless, Threshold>> index_;
};

template<size_t RadixBits>
class RadixSplineIndex {
    using Index = rs::RadixSpline<uint64_t>;
public:
    explicit RadixSplineIndex(const std::vector<uint64_t>& keys) : keys_(keys) {
        if (const auto reason = unsupported_reason(keys); !reason.empty())
            throw std::domain_error(reason);
        // The upstream builder invokes clz(0) for a zero-width domain.
        if (keys.size() <= 1) return;
        rs::Builder<uint64_t> builder(keys.front(), keys.back(), RadixBits, 32);
        for (const auto key : keys) builder.AddKey(key);
        index_.emplace(builder.Finalize());
    }

    size_t find(uint64_t key) const {
        if (detail::outside(keys_, key)) return keys_.size();
        if (keys_.size() == 1) return 0;
        const auto bound = index_->GetSearchBound(key);
        return detail::exact_search(keys_, bound.begin, bound.end, key);
    }

    size_t bytes() const {
        return sizeof(*this) + (index_ ? index_->GetSize() - sizeof(Index) : 0);
    }
    static std::string name() {
        return "radix_spline_radix" + std::to_string(RadixBits) + "_error32";
    }
    static std::string unsupported_reason(const std::vector<uint64_t>& keys) {
        // Spline-point offsets in the radix directory are uint32_t.
        if (keys.size() > std::numeric_limits<uint32_t>::max())
            return "RadixSpline artifact uses uint32_t spline-point offsets";
        return {};
    }
private:
    const std::vector<uint64_t>& keys_;
    std::optional<Index> index_;
};

template<size_t Terminal>
class ChtIndex {
    using Index = cht::CompactHistTree<uint64_t>;
    static_assert(Terminal == 32 || Terminal == 128);
public:
    explicit ChtIndex(const std::vector<uint64_t>& keys) : keys_(keys) {
        if (const auto reason = unsupported_reason(keys); !reason.empty())
            throw std::domain_error(reason);
        if (keys.size() <= 1) return;
        auto minimum = keys.front();
        const auto span = keys.back() - minimum;
        // Upstream ceil(log2(span)) puts the last key in bin64 when span is
        // exactly a power of two. Widen only the declared lower bound; all input
        // keys, ranks, and the required actual maximum remain unchanged.
        if (std::has_single_bit(span)) --minimum;
        cht::Builder<uint64_t> builder(minimum, keys.back(), 64, Terminal,
                                       false, false);
        for (const auto key : keys) builder.AddKey(key);
        index_.emplace(builder.Finalize());
    }

    size_t find(uint64_t key) const {
        if (detail::outside(keys_, key)) return keys_.size();
        if (keys_.size() == 1) return 0;
        const auto bound = index_->GetSearchBound(key);
        return detail::exact_search(keys_, bound.begin, bound.end, key);
    }

    size_t bytes() const {
        return sizeof(*this) + (index_ ? index_->GetSize() - sizeof(Index) : 0);
    }
    static std::string name() {
        return "cht_stoian_offline_fanout64_terminal" + std::to_string(Terminal);
    }
    static std::string unsupported_reason(const std::vector<uint64_t>& keys) {
        if (keys.size() >= (uint64_t{1} << 31))
            return "CHT artifact reserves the high rank bit for leaf tagging";
        if (keys.size() <= 1) return {};
        const auto span = keys.back() - keys.front();
        if (span <= 32)
            return "CHT fanout64 requires nontrivial key span greater than32";
        if (keys.front() == 0 && std::has_single_bit(span))
            return "CHT power-of-two span with minimum0 cannot widen lower bound";
        return {};
    }
private:
    const std::vector<uint64_t>& keys_;
    std::optional<Index> index_;
};

// Local baseline, NOT a LiBox reproduction: a fence (maximum) per contiguous
// leaf block, a scalar binary search over fences, and exact SIMD leaf scanning.
// Leaves reference the original sorted keys: no copied keys or hidden payloads.
template<size_t BlockKeys>
class BlockedSortedFence {
    static_assert(BlockKeys == 16 || BlockKeys == 64);
public:
    explicit BlockedSortedFence(const std::vector<uint64_t>& keys) : keys_(keys) {
        fences_.reserve(keys.size() / BlockKeys + (keys.size() % BlockKeys != 0));
        for (size_t first = 0; first < keys.size();) {
            const size_t end = first + std::min(BlockKeys, keys.size() - first);
            fences_.push_back(keys[end - 1]);
            first = end;
        }
    }

    size_t find(uint64_t key) const {
        const auto fence = std::lower_bound(fences_.begin(), fences_.end(), key);
        if (fence == fences_.end()) return keys_.size();
        size_t pos = static_cast<size_t>(fence - fences_.begin()) * BlockKeys;
        const size_t end = pos + std::min(BlockKeys, keys_.size() - pos);
#if defined(__aarch64__) && defined(__ARM_NEON)
        const uint64x2_t needle = vdupq_n_u64(key);
        for (; end - pos >= 2; pos += 2) {
            const uint64x2_t matches = vceqq_u64(vld1q_u64(keys_.data() + pos), needle);
            const uint64_t first = vgetq_lane_u64(matches, 0);
            const uint64_t second = vgetq_lane_u64(matches, 1);
            if (first | second) return pos + (first == 0);
        }
#endif
        for (; pos < end; ++pos)
            if (keys_[pos] == key) return pos;
        return keys_.size();
    }

    size_t bytes() const { return sizeof(*this) + fences_.size() * sizeof(uint64_t); }
    static std::string name() {
#if defined(__aarch64__) && defined(__ARM_NEON)
        return "local_blocked_sorted_fence_neon_u64_block" + std::to_string(BlockKeys);
#else
        return "local_blocked_sorted_fence_scalar_u64_block" + std::to_string(BlockKeys);
#endif
    }
    static std::string unsupported_reason(const std::vector<uint64_t>&) { return {}; }
private:
    const std::vector<uint64_t>& keys_;
    std::vector<uint64_t> fences_;
};

// Match the artifact main.cpp's branchy threshold0 and PGM++ threshold8.
using PgmOriginal = PgmIndex<64, false, 0>;
using PgmPlus16 = PgmIndex<16, true, 8>;
using PgmPlus64 = PgmIndex<64, true, 8>;
using PgmDefault = PgmPlus64;
using ChtDefault = ChtIndex<32>;

template<class F>
void sorted_index_types(F&& f) {
    f.template operator()<StdLowerBound>();
    f.template operator()<ArtifactBranchless>();
    f.template operator()<ChtIndex<32>>();
    f.template operator()<ChtIndex<128>>();
    f.template operator()<RadixSplineIndex<12>>();
    f.template operator()<RadixSplineIndex<18>>();
    f.template operator()<PgmOriginal>();
    f.template operator()<PgmPlus16>();
    f.template operator()<PgmPlus64>();
    f.template operator()<BlockedSortedFence<16>>();
    f.template operator()<BlockedSortedFence<64>>();
}

}  // namespace idxbench
