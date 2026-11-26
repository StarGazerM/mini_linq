#include "mccl.h"
#include "relation_index.h"

#include <algorithm>
#include <array>
#include <climits>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <dlfcn.h>
#include <exception>
#include <filesystem>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

#include "mlx/allocator.h"
#include "mlx/backend/metal/device.h"
#include "mlx/c/private/array.h"
#include "mlx/c/private/stream.h"
#include "mlx/ops.h"
#include "mlx/primitives.h"

namespace mx = mlx::core;

struct mccl_bloom {
  mccl_bloom(mx::Stream stream, uint32_t num_blocks, uint64_t seed)
      : stream(stream),
        num_blocks(num_blocks),
        seed(seed),
        owner(std::this_thread::get_id()),
        bits({static_cast<int>(num_blocks) * 8}, mx::uint32, nullptr, {}) {
    bits.set_data(mx::allocator::malloc(bits.nbytes()));
  }

  const mx::Stream stream;
  const uint32_t num_blocks;
  const uint64_t seed;
  const std::thread::id owner;
  mx::array bits;
};

struct mccl_map {
  mccl_map(mx::Stream stream, uint32_t capacity, uint32_t seed)
      : stream(stream),
        capacity(capacity),
        seed(seed),
        owner(std::this_thread::get_id()),
        keys({static_cast<int>(capacity)}, mx::uint32, nullptr, {}),
        values({static_cast<int>(capacity)}, mx::uint32, nullptr, {}),
        winners({static_cast<int>(capacity)}, mx::uint32, nullptr, {}),
        stats({4}, mx::uint32, nullptr, {}),
        slots({0}, mx::uint32, nullptr, {}) {
    keys.set_data(mx::allocator::malloc(keys.nbytes()));
    values.set_data(mx::allocator::malloc(values.nbytes()));
    winners.set_data(mx::allocator::malloc(winners.nbytes()));
    stats.set_data(mx::allocator::malloc(stats.nbytes()));
  }

  const mx::Stream stream;
  uint32_t capacity;
  const uint32_t seed;
  const std::thread::id owner;
  uint32_t size = 0;
  uint32_t tombstones = 0;
  mx::array keys;
  mx::array values;
  // Zero between completed insertions; publication clears every winning slot.
  mx::array winners;
  mx::array stats;
  // Reused across insert batches; only the current input prefix is accessed.
  mx::array slots;
  // Device owns these cached pipelines; avoid specialization-vector/string
  // allocation on every completed lookup after each width has been resolved.
  MTL::ComputePipelineState* lookup_kernels[4] = {};
};

namespace {

constexpr size_t kThreads = 256;
constexpr size_t kTile = 1024;
constexpr size_t kMaxPairRows = size_t{1} << 30;
constexpr size_t kDirectoryEntries = 4097;
thread_local char last_error[1024] = {};

enum class Operation { Merge, Copy };

// Resolve relative to this dylib, never to the process working directory.
const std::string& library_directory() {
  static const std::string directory = [] {
    Dl_info info{};
    if (!dladdr(reinterpret_cast<const void*>(&mccl_last_error), &info) ||
        !info.dli_fname) {
      throw std::runtime_error("mccl: cannot locate the native shared library");
    }
    return std::filesystem::canonical(info.dli_fname).parent_path().string();
  }();
  return directory;
}

class U32Operation final : public mx::Primitive {
 public:
  U32Operation(mx::Stream stream, Operation operation)
      : mx::Primitive(stream), operation_(operation) {}

  const char* name() const override {
    return operation_ == Operation::Merge ? "McclMergeSortedU32" : "McclCopyU32";
  }

  void eval_cpu(
      const std::vector<mx::array>&,
      std::vector<mx::array>&) override {
    throw std::runtime_error("mccl operations require a GPU stream");
  }

  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& out = outputs[0];
    // Never donate or alias an input, including for the copy baseline.
    out.set_data(mx::allocator::malloc(out.nbytes()));
    if (out.size() == 0) {
      return;
    }

    auto& device = mx::metal::device(stream().device);
    auto* library = device.get_library("mccl", library_directory());
    const bool merge = operation_ == Operation::Merge;
    auto* kernel = device.get_kernel(
        merge ? "mccl_merge_u32" : "mccl_copy_u32_kernel", library);
    if (kernel->maxTotalThreadsPerThreadgroup() < kThreads) {
      throw std::runtime_error("mccl kernel requires 256 threads per threadgroup");
    }

    auto& encoder = mx::metal::get_command_encoder(stream());
    encoder.set_compute_pipeline_state(kernel);
    if (merge) {
      const auto na = static_cast<uint32_t>(inputs[0].size());
      const auto nb = static_cast<uint32_t>(inputs[1].size());
      // A zero-sized MLX array can have no Metal buffer. Bind the other input
      // instead; its corresponding zero count ensures that pointer is not read.
      encoder.set_input_array(na != 0 ? inputs[0] : inputs[1], 0);
      encoder.set_input_array(nb != 0 ? inputs[1] : inputs[0], 1);
      encoder.set_output_array(out, 2);
      encoder.set_bytes(na, 3);
      encoder.set_bytes(nb, 4);
    } else {
      const auto count = static_cast<uint32_t>(inputs[0].size());
      encoder.set_input_array(inputs[0], 0);
      encoder.set_output_array(out, 1);
      encoder.set_bytes(count, 2);
    }
    encoder.dispatch_threadgroups(
        MTL::Size((out.size() + kTile - 1) / kTile, 1, 1),
        MTL::Size(kThreads, 1, 1));
  }

 private:
  Operation operation_;
};

class DifferenceScatter final : public mx::Primitive {
 public:
  explicit DifferenceScatter(mx::Stream stream) : mx::Primitive(stream) {}

  const char* name() const override {
    return "McclDifferenceScatterU32";
  }

  void eval_cpu(
      const std::vector<mx::array>&,
      std::vector<mx::array>&) override {
    throw std::runtime_error("mccl operations require a GPU stream");
  }

  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& out = outputs[0];
    // Only the retained prefix will be initialized; never donate an input.
    out.set_data(mx::allocator::malloc(out.nbytes()));
    if (out.size() == 0) {
      return;
    }

    auto& device = mx::metal::device(stream().device);
    auto* library = device.get_library("mccl", library_directory());
    auto* kernel = device.get_kernel("mccl_difference_scatter_u32", library);
    if (kernel->maxTotalThreadsPerThreadgroup() < kThreads) {
      throw std::runtime_error("mccl kernel requires 256 threads per threadgroup");
    }

    auto& encoder = mx::metal::get_command_encoder(stream());
    encoder.set_compute_pipeline_state(kernel);
    encoder.set_input_array(inputs[0], 0);
    encoder.set_input_array(inputs[1], 1);
    encoder.set_input_array(inputs[2], 2);
    encoder.set_output_array(out, 3);
    const auto capacity = static_cast<uint32_t>(out.size());
    encoder.set_bytes(capacity, 4);
    encoder.dispatch_threadgroups(
        MTL::Size((out.size() + kTile - 1) / kTile, 1, 1),
        MTL::Size(kThreads, 1, 1));
  }
};

void validate_input(const mx::array& input) {
  if (input.ndim() != 1 || input.dtype() != mx::uint32) {
    throw std::invalid_argument("mccl inputs must be one-dimensional uint32 arrays");
  }
  if (input.size() > static_cast<size_t>(INT_MAX)) {
    throw std::invalid_argument("mccl input length exceeds INT_MAX");
  }
}

mx::Stream validate_stream(mlx_stream stream) {
  auto result = mlx_stream_get_(stream);
  if (result.device.type != mx::Device::gpu) {
    throw std::invalid_argument("mccl operations require a GPU stream");
  }
  return result;
}

mccl_bloom& validate_bloom(mccl_bloom* filter) {
  if (!filter) {
    throw std::invalid_argument("mccl Bloom filter must not be null");
  }
  if (filter->owner != std::this_thread::get_id()) {
    throw std::invalid_argument("mccl Bloom filter must be used on its creating thread");
  }
  return *filter;
}

mx::array prepare_bloom_keys(const mccl_bloom& filter, mlx_array keys) {
  const auto& input = mlx_array_get_(keys);
  validate_input(input);
  auto contiguous = mx::contiguous(input, false, filter.stream);
  // array::eval waits for the entire dependency graph, including other streams.
  // Do this before encoding direct accesses outside MLX's lazy scheduler.
  contiguous.eval();
  return contiguous;
}

MTL::ComputePipelineState* bloom_kernel(
    const mccl_bloom& filter, const char* name) {
  auto& device = mx::metal::device(filter.stream.device);
  auto* library = device.get_library("mccl", library_directory());
  auto* kernel = device.get_kernel(name, library);
  if (kernel->maxTotalThreadsPerThreadgroup() < kThreads) {
    throw std::runtime_error("mccl kernel requires 256 threads per threadgroup");
  }
  return kernel;
}

void clear_bloom(mccl_bloom& filter) {
  auto pool = NS::TransferPtr(NS::AutoreleasePool::alloc()->init());
  auto* kernel = bloom_kernel(filter, "mccl_bloom_clear_kernel");
  auto& encoder = mx::metal::get_command_encoder(filter.stream);
  // Command buffers use unretained references. Retain the array even on an
  // encoding/commit exception; end_encoding transfers it to completion cleanup.
  // Private bits and unpublished results never need cross-stream fence tracking.
  encoder.add_temporary(filter.bits);
  encoder.set_compute_pipeline_state(kernel);
  encoder.set_output_array(filter.bits, 0);
  const auto count = static_cast<uint32_t>(filter.bits.size());
  encoder.set_bytes(count, 1);
  encoder.dispatch_threadgroups(
      MTL::Size((count + kThreads - 1) / kThreads, 1, 1),
      MTL::Size(kThreads, 1, 1));
  // Pinned MLX synchronize ends encoding, commits THIS command buffer, waits
  // until completion, then throws any GPU error. A separate commit is needless.
  encoder.synchronize();
  filter.bits.set_status(mx::array::Status::available);
}

mccl_map& validate_map(mccl_map* map) {
  if (!map) {
    throw std::invalid_argument("mccl map must not be null");
  }
  if (map->owner != std::this_thread::get_id()) {
    throw std::invalid_argument("mccl map must be used on its creating thread");
  }
  return *map;
}

void validate_map_capacity(uint32_t capacity, mx::Stream stream) {
  if (capacity == 0 || capacity > static_cast<uint32_t>(INT_MAX)) {
    throw std::invalid_argument("mccl map capacity must be positive and <= INT_MAX");
  }
  const auto max_bytes =
      mx::metal::device(stream.device).mtl_device()->maxBufferLength();
  if (static_cast<size_t>(capacity) * sizeof(uint32_t) > max_bytes) {
    throw std::invalid_argument("mccl map capacity exceeds the Metal buffer limit");
  }
}

mx::array allocate_map_array(uint32_t count) {
  mx::array result({static_cast<int>(count)}, mx::uint32, nullptr, {});
  result.set_data(mx::allocator::malloc(result.nbytes()));
  return result;
}

mx::array prepare_map_keys(const mccl_map& map, mlx_array keys) {
  const auto& input = mlx_array_get_(keys);
  validate_input(input);
  auto contiguous = mx::contiguous(input, false, map.stream);
  contiguous.eval();
  return contiguous;
}

MTL::ComputePipelineState* map_kernel(const mccl_map& map, const char* name) {
  auto& device = mx::metal::device(map.stream.device);
  auto* library = device.get_library("mccl", library_directory());
  auto* kernel = device.get_kernel(name, library);
  if (kernel->maxTotalThreadsPerThreadgroup() < kThreads) {
    throw std::runtime_error("mccl kernel requires 256 threads per threadgroup");
  }
  return kernel;
}

mx::metal::Device& validate_index_device(mx::Stream stream) {
  auto& device = mx::metal::device(stream.device);
  if (!device.mtl_device()->hasUnifiedMemory()) {
    throw std::invalid_argument("mccl index operations require unified memory");
  }
  return device;
}

void validate_capacity(
    size_t elements, mx::metal::Device& device,
    size_t itemsize = sizeof(uint32_t)) {
  if (elements > std::numeric_limits<size_t>::max() / itemsize ||
      elements * itemsize > device.mtl_device()->maxBufferLength()) {
    throw std::invalid_argument("mccl allocation exceeds device buffer capacity");
  }
}

void allocate_storage(mx::array& array) {
  auto buffer = mx::allocator::malloc(array.nbytes());
  try {
    array.set_data(buffer);
  } catch (...) {
    mx::allocator::free(buffer);
    throw;
  }
}

mx::array allocate_leaf(
    mx::Shape shape, mx::metal::Device& device, mx::Dtype dtype = mx::uint32) {
  // A primitive-free leaf owns fresh shared storage. Mark it available only
  // after allocation; no initialization kernel or CPU zero-fill is needed.
  mx::array result(std::move(shape), dtype, nullptr, {});
  validate_capacity(result.size(), device, result.itemsize());
  allocate_storage(result);
  if (result.size() != 0 && !result.data<void>()) {
    throw std::runtime_error("mccl allocation has no CPU mapping");
  }
  result.set_status(mx::array::Status::available);
  return result;
}

void validate_pairs(const mx::array& input, mx::metal::Device& device) {
  if (input.ndim() != 2 || input.shape(1) != 2 ||
      input.dtype() != mx::uint32) {
    throw std::invalid_argument("mccl pair inputs must have shape [N,2] and dtype uint32");
  }
  if (static_cast<size_t>(input.shape(0)) > kMaxPairRows) {
    throw std::invalid_argument("mccl pair row count exceeds 1<<30");
  }
  validate_capacity(input.size(), device);
}

bool index_kernel_fits(
    mx::metal::Device& device, MTL::ComputePipelineState* kernel) {
  return kernel->threadExecutionWidth() == 32 &&
      kernel->maxTotalThreadsPerThreadgroup() >= kThreads &&
      kernel->staticThreadgroupMemoryLength() <=
          device.mtl_device()->maxThreadgroupMemoryLength();
}

MTL::ComputePipelineState* index_kernel(
    mx::metal::Device& device, const char* name) {
  auto* library = device.get_library("mccl", library_directory());
  auto* kernel = device.get_kernel(name, library);
  if (!index_kernel_fits(device, kernel)) {
    throw std::runtime_error(
        std::string("mccl index kernel ") + name +
        " requires SIMD32, 256 threads and device-supported shared memory; got SIMD=" +
        std::to_string(kernel->threadExecutionWidth()) +
        ", max_threads=" + std::to_string(kernel->maxTotalThreadsPerThreadgroup()) +
        ", shared_bytes=" + std::to_string(kernel->staticThreadgroupMemoryLength()) +
        ", device_shared_limit=" +
        std::to_string(device.mtl_device()->maxThreadgroupMemoryLength()));
  }
  return kernel;
}

void dispatch_map(mx::metal::CommandEncoder& encoder, uint32_t count) {
  encoder.dispatch_threadgroups(
      MTL::Size((static_cast<size_t>(count) + kThreads - 1) / kThreads, 1, 1),
      MTL::Size(kThreads, 1, 1));
}

void clear_map(mccl_map& map, bool initialize_winners = false) {
  auto pool = NS::TransferPtr(NS::AutoreleasePool::alloc()->init());
  auto* kernel = map_kernel(map, "mccl_map_clear_kernel");
  auto& encoder = mx::metal::get_command_encoder(map.stream);
  encoder.add_temporary(map.keys);
  encoder.add_temporary(map.winners);
  encoder.set_compute_pipeline_state(kernel);
  encoder.set_output_array(map.keys, 0);
  encoder.set_output_array(map.winners, 1);
  encoder.set_bytes(map.capacity, 2);
  const uint32_t winner_count = initialize_winners ? map.capacity : 0;
  encoder.set_bytes(winner_count, 3);
  dispatch_map(encoder, map.capacity);
  encoder.synchronize();
  map.keys.set_status(mx::array::Status::available);
  if (initialize_winners) {
    map.winners.set_status(mx::array::Status::available);
  }
  map.size = 0;
  map.tombstones = 0;
}

void rehash_map(mccl_map& map, uint32_t capacity) {
  validate_map_capacity(capacity, map.stream);
  if (capacity < map.size) {
    throw std::invalid_argument("mccl map rehash capacity is smaller than its size");
  }
  if (capacity == map.capacity && !map.tombstones) {
    mx::synchronize(map.stream);
    return;
  }

  // Allocate everything and resolve kernels before encoding. The old table is
  // read-only throughout the rebuild and remains installed until completion.
  auto keys = allocate_map_array(capacity);
  auto values = allocate_map_array(capacity);
  auto winners = capacity == map.capacity
      ? map.winners
      : allocate_map_array(capacity);
  auto pool = NS::TransferPtr(NS::AutoreleasePool::alloc()->init());
  auto* clear = map_kernel(map, "mccl_map_clear_kernel");
  auto* rehash = map_kernel(map, "mccl_map_rehash_kernel");
  auto& encoder = mx::metal::get_command_encoder(map.stream);
  encoder.add_temporary(map.keys);
  encoder.add_temporary(map.values);
  encoder.add_temporary(keys);
  encoder.add_temporary(values);
  encoder.add_temporary(winners);
  encoder.set_compute_pipeline_state(clear);
  encoder.set_output_array(keys, 0);
  encoder.set_output_array(winners, 1);
  encoder.set_bytes(capacity, 2);
  const uint32_t winner_count = capacity == map.capacity ? 0 : capacity;
  encoder.set_bytes(winner_count, 3);
  dispatch_map(encoder, capacity);
  if (map.size != 0) {
    encoder.set_compute_pipeline_state(rehash);
    encoder.set_input_array(map.keys, 0);
    encoder.set_input_array(map.values, 1);
    encoder.set_output_array(keys, 2);
    encoder.set_output_array(values, 3);
    encoder.set_bytes(map.capacity, 4);
    encoder.set_bytes(capacity, 5);
    encoder.set_bytes(map.seed, 6);
    dispatch_map(encoder, map.capacity);
  }
  encoder.synchronize();
  keys.set_status(mx::array::Status::available);
  values.set_status(mx::array::Status::available);
  winners.set_status(mx::array::Status::available);
  map.keys = std::move(keys);
  map.values = std::move(values);
  map.winners = std::move(winners);
  map.capacity = capacity;
  map.tombstones = 0;
}

void reset_map_stats(
    mccl_map& map,
    mx::metal::CommandEncoder& encoder,
    MTL::ComputePipelineState* kernel) {
  encoder.add_temporary(map.stats);
  encoder.set_compute_pipeline_state(kernel);
  encoder.set_output_array(map.stats, 0);
  dispatch_map(encoder, 4);
}

struct IndexTile {
  MTL::ComputePipelineState* kernel;
  uint32_t items;
};

IndexTile index_tile(
    mx::metal::Device& device, const char* preferred, const char* smaller,
    uint32_t preferred_items) {
  auto* library = device.get_library("mccl", library_directory());
  auto* kernel = device.get_kernel(preferred, library);
  if (index_kernel_fits(device, kernel)) {
    return {kernel, preferred_items};
  }
  // Instrumentation can enlarge static threadgroup storage. Select the same
  // stable algorithm with a smaller tile from actual pipeline capabilities,
  // never from an environment-variable test or a hidden CPU fallback.
  return {index_kernel(device, smaller), preferred_items / 2};
}

size_t ceil_div(size_t count, size_t block) {
  return count / block + (count % block != 0);
}

void dispatch(mx::metal::CommandEncoder& encoder, size_t groups) {
  encoder.dispatch_threadgroups(
      MTL::Size(groups, 1, 1), MTL::Size(kThreads, 1, 1));
}

std::vector<mx::array> allocate_scan(
    size_t length, mx::metal::Device& device,
    mx::metal::CommandEncoder& encoder) {
  std::vector<mx::array> levels;
  levels.push_back(allocate_leaf({static_cast<int>(length)}, device));
  // Even a single flag needs a separate total slot. Every slot is GPU-written
  // before it is read; no scratch initialization is performed by the CPU.
  do {
    length = ceil_div(length, kTile);
    levels.push_back(allocate_leaf({static_cast<int>(length)}, device));
  } while (length > 1);
  for (const auto& level : levels) {
    encoder.add_temporary(level);
  }
  return levels;
}

void encode_scan(
    std::vector<mx::array>& levels, mx::metal::Device& device,
    mx::metal::CommandEncoder& encoder,
    const char* scan_kernel = "sort_scan_blocks",
    const char* add_kernel = "sort_add_block_offsets") {
  auto* scan = index_kernel(device, scan_kernel);
  auto* add = index_kernel(device, add_kernel);
  const size_t depth = levels.size() - 1;
  for (size_t level = 0; level < depth; ++level) {
    const auto n = static_cast<uint32_t>(levels[level].size());
    encoder.set_compute_pipeline_state(scan);
    // Output registration includes input hazards for read/write scans. MLX
    // inserts the barriers required by its untracked allocation buffers.
    encoder.set_output_array(levels[level], 0);
    encoder.set_output_array(levels[level + 1], 1);
    encoder.set_bytes(n, 2);
    dispatch(encoder, ceil_div(n, kTile));
  }
  for (size_t level = depth - 1; level > 0;) {
    --level;
    const auto n = static_cast<uint32_t>(levels[level].size());
    encoder.set_compute_pipeline_state(add);
    encoder.set_output_array(levels[level], 0);
    encoder.set_input_array(levels[level + 1], 1);
    encoder.set_bytes(n, 2);
    dispatch(encoder, ceil_div(n, kTile));
  }
}

// Stable LSD sort of {key, payload} pairs over the key's low `bytes` bytes
// (1..4). Higher key bytes must be zero, or the order they imply is ignored.
void encode_sort(
    const mx::array& input, mx::array& out,
    mx::metal::Device& device, mx::metal::CommandEncoder& encoder,
    const mx::array* metadata = nullptr, uint32_t bytes = 4) {
  const bool dynamic = metadata != nullptr;
  const auto tile = index_tile(device,
      dynamic ? "sort_scatter_dynamic_byte8" : "sort_scatter_byte8",
      dynamic ? "sort_scatter_dynamic_byte4" : "sort_scatter_byte4", 8);
  auto* histogram = index_kernel(device, dynamic
      ? (tile.items == 8 ? "sort_histogram_dynamic_byte8" : "sort_histogram_dynamic_byte4")
      : (tile.items == 8 ? "sort_histogram_byte8" : "sort_histogram_byte4"));
  const auto capacity = static_cast<uint32_t>(input.shape(0));
  const size_t tiles = ceil_div(capacity, kThreads * tile.items);
  auto scratch = allocate_leaf(out.shape(), device);
  encoder.add_temporary(scratch);
  auto levels = allocate_scan(tiles * 256, device, encoder);
  for (uint32_t pass = 0; pass < bytes; ++pass) {
    // Alternate so the final pass writes `out`, never the input.
    auto& destination = (bytes - 1 - pass) % 2 == 0 ? out : scratch;
    const auto& source = pass == 0 ? input : ((bytes - pass) % 2 == 0 ? out : scratch);
    const uint32_t args[] = {capacity, static_cast<uint32_t>(tiles), pass * 8};
    encoder.set_compute_pipeline_state(histogram);
    encoder.set_input_array(source, 0);
    encoder.set_output_array(levels[0], 1);
    encoder.set_bytes(args, 3, 2);
    if (metadata) encoder.set_input_array(*metadata, 3);
    dispatch(encoder, tiles);
    encode_scan(levels, device, encoder);
    encoder.set_compute_pipeline_state(tile.kernel);
    encoder.set_input_array(source, 0);
    encoder.set_output_array(destination, 1);
    encoder.set_input_array(levels[0], 2);
    encoder.set_bytes(args, 3, 3);
    if (metadata) encoder.set_input_array(*metadata, 4);
    dispatch(encoder, tiles);
  }
}

void encode_directory(
    const mx::array& rows, mx::array& out, mx::metal::Device& device,
    mx::metal::CommandEncoder& encoder, const mx::array* metadata = nullptr) {
  encoder.set_compute_pipeline_state(index_kernel(device,
      metadata ? "maintain_directory_dynamic" : "maintain_directory"));
  // Empty arrays may have no Metal buffer. The zero count prevents row reads.
  encoder.set_input_array(rows.size() == 0 ? out : rows, 0);
  encoder.set_output_array(out, 1);
  if (metadata) {
    encoder.set_input_array(*metadata, 2);
  } else {
    encoder.set_bytes(static_cast<uint32_t>(rows.shape(0)), 2);
  }
  dispatch(encoder, ceil_div(kDirectoryEntries, kThreads));
}

enum class PairOperation { Sort, Merge, Directory };

class PairU32Operation final : public mx::Primitive {
 public:
  PairU32Operation(mx::Stream stream, PairOperation operation)
      : mx::Primitive(stream), operation_(operation) {}

  const char* name() const override {
    switch (operation_) {
      case PairOperation::Sort: return "McclSortPairsU32";
      case PairOperation::Merge: return "McclMergeSortedPairsU32";
      case PairOperation::Directory: return "McclRadixDirectoryU32";
    }
    return "McclPairsU32";
  }

  void eval_cpu(
      const std::vector<mx::array>&,
      std::vector<mx::array>&) override {
    throw std::runtime_error("mccl operations require a GPU stream");
  }

  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& device = validate_index_device(stream());
    auto& out = outputs[0];
    validate_capacity(out.size(), device);
    allocate_storage(out);
    if (out.size() == 0) {
      return;
    }
    auto& encoder = mx::metal::get_command_encoder(stream());
    switch (operation_) {
      case PairOperation::Sort:
        encode_sort(inputs[0], out, device, encoder);
        break;
      case PairOperation::Merge:
        merge(inputs[0], inputs[1], out, device, encoder);
        break;
      case PairOperation::Directory:
        encode_directory(inputs[0], out, device, encoder);
        break;
    }
  }

 private:

  static void merge(
      const mx::array& older, const mx::array& newer, mx::array& out,
      mx::metal::Device& device, mx::metal::CommandEncoder& encoder) {
    auto* partition = index_kernel(device, "maintain_partition");
    const auto tile = index_tile(
        device, "maintain_merge_partitioned_4", "maintain_merge_partitioned_2", 4);
    const size_t tiles = ceil_div(
        static_cast<size_t>(out.shape(0)), kThreads * tile.items);
    auto bounds = allocate_leaf({static_cast<int>(tiles + 1)}, device);
    encoder.add_temporary(bounds);
    const uint32_t args[] = {
        static_cast<uint32_t>(older.shape(0)),
        static_cast<uint32_t>(newer.shape(0)), tile.items};
    // Output is nonempty here, so at least one valid input buffer exists.
    const auto& a = args[0] == 0 ? newer : older;
    const auto& b = args[1] == 0 ? older : newer;
    encoder.set_compute_pipeline_state(partition);
    encoder.set_input_array(a, 0);
    encoder.set_input_array(b, 1);
    encoder.set_output_array(bounds, 2);
    encoder.set_bytes(args, 3, 3);
    dispatch(encoder, ceil_div(tiles + 1, kThreads));
    encoder.set_compute_pipeline_state(tile.kernel);
    encoder.set_input_array(a, 0);
    encoder.set_input_array(b, 1);
    encoder.set_output_array(out, 2);
    encoder.set_input_array(bounds, 3);
    encoder.set_bytes(args, 3, 4);
    dispatch(encoder, tiles);
  }

  PairOperation operation_;
};

class RelationViewU32 final : public mx::Primitive {
 public:
  explicit RelationViewU32(mx::Stream stream) : mx::Primitive(stream) {}
  const char* name() const override { return "McclRelationViewU32"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("mccl relation views require a GPU stream");
  }
  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    const auto& source = inputs[0];
    if (!source.flags().row_contiguous) {
      throw std::invalid_argument("mccl relation source must be contiguous");
    }
    auto& out = outputs[0];
    const mx::array::Flags flags{
        true, true, out.shape(0) <= 1 || out.shape(1) == 1};
    out.copy_shared_buffer(source, out.strides(), flags, out.size());
  }
};

class RelationIndexOperation final : public mx::Primitive {
 public:
  RelationIndexOperation(mx::Stream stream, int key_column, int payload_column,
                         bool build_directory)
      : mx::Primitive(stream), key_column_(key_column),
        payload_column_(payload_column), build_directory_(build_directory) {}
  const char* name() const override { return "McclBuildRelationIndexU32"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("mccl index construction requires a GPU stream");
  }
  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& device = validate_index_device(stream());
    const auto& relation = inputs[0];
    const bool masked = inputs.size() == 2;
    if (!relation.flags().row_contiguous ||
        (masked && !inputs[1].flags().row_contiguous)) {
      throw std::invalid_argument("mccl relation and selection must be row-contiguous");
    }
    auto& rows = outputs[0];
    auto& directory = outputs[1];
    auto& metadata = outputs[2];
    for (auto& output : outputs) allocate_storage(output);
    auto& encoder = mx::metal::get_command_encoder(stream());
    const auto count = static_cast<uint32_t>(relation.shape(0));
    std::vector<mx::array> selection_scan;
    if (masked && count != 0) {
      selection_scan = allocate_scan(count, device, encoder);
      encoder.set_compute_pipeline_state(index_kernel(device, "relation_selection_flags"));
      encoder.set_input_array(inputs[1], 0);
      encoder.set_output_array(selection_scan[0], 1);
      encoder.set_bytes(count, 2);
      dispatch(encoder, ceil_div(count, kThreads));
      encode_scan(selection_scan, device, encoder);
    }
    const uint32_t count_args[] = {count, uint32_t(masked)};
    encoder.set_compute_pipeline_state(index_kernel(device, "relation_metadata"));
    encoder.set_input_array(selection_scan.empty() ? metadata : selection_scan.back(), 0);
    encoder.set_output_array(metadata, 1);
    encoder.set_bytes(count_args, 2, 2);
    dispatch(encoder, 1);
    if (count != 0) {
      // The canonical unmasked pair layout already is a projection. Sorting
      // reads it directly, but never donates or writes its immutable storage.
      const bool direct = !masked && relation.dtype() == mx::uint32 &&
          relation.shape(1) == 2 && key_column_ == 0 && payload_column_ == 1;
      if (direct) {
        encode_sort(relation, rows, device, encoder, &metadata);
      } else {
        auto projected = allocate_leaf(rows.shape(), device);
        encoder.add_temporary(projected);
        const uint32_t args[] = {
            count, static_cast<uint32_t>(relation.shape(1)),
            static_cast<uint32_t>(key_column_), static_cast<uint32_t>(payload_column_),
            uint32_t(masked)};
        encoder.set_compute_pipeline_state(index_kernel(device,
            relation.dtype() == mx::uint32 ? "relation_project_u32" : "relation_project_i64"));
        encoder.set_input_array(relation, 0);
        encoder.set_input_array(masked ? inputs[1] : metadata, 1);
        encoder.set_input_array(masked ? selection_scan[0] : metadata, 2);
        encoder.set_output_array(projected, 3);
        encoder.set_output_array(metadata, 4);
        encoder.set_bytes(args, 5, 5);
        dispatch(encoder, ceil_div(count, kThreads));
        encode_sort(projected, rows, device, encoder, &metadata);
      }
    }
    if (build_directory_) encode_directory(rows, directory, device, encoder, &metadata);
  }
 private:
  int key_column_;
  int payload_column_;
  bool build_directory_;
};

template <typename T>
void allocate_shared(
    mlx_array* out, const int* shape, int ndim, T** data,
    mlx_stream stream, mx::Dtype dtype) {
  if (!out || !data || !shape) {
    throw std::invalid_argument("mccl allocation pointers must not be null");
  }
  if (ndim != 1 && ndim != 2) {
    throw std::invalid_argument("mccl allocation requires a one- or two-dimensional shape");
  }
  size_t elements = 1;
  mx::Shape dimensions;
  for (int axis = 0; axis < ndim; ++axis) {
    if (shape[axis] < 0) {
      throw std::invalid_argument("mccl allocation dimensions must be nonnegative");
    }
    const size_t dimension = static_cast<size_t>(shape[axis]);
    if (dimension != 0 &&
        elements > std::numeric_limits<size_t>::max() / dimension) {
      throw std::invalid_argument("mccl allocation shape product overflows");
    }
    elements *= dimension;
    dimensions.push_back(shape[axis]);
  }
  auto& device = validate_index_device(validate_stream(stream));
  validate_capacity(elements, device, sizeof(T));
  auto result = allocate_leaf(std::move(dimensions), device, dtype);
  auto* pointer = elements == 0 ? nullptr : result.data<T>();
  mlx_array_set_(*out, std::move(result));
  *data = pointer;
}

template <typename Function>
int capture_error(Function&& function) noexcept {
  try {
    function();
    last_error[0] = '\0';
    return 0;
  } catch (const std::exception& error) {
    std::snprintf(last_error, sizeof(last_error), "%s", error.what());
  } catch (...) {
    std::snprintf(last_error, sizeof(last_error), "%s", "mccl: unknown C++ exception");
  }
  return 1;
}

} // namespace

namespace mccl::native {

std::vector<mx::array> make_relation_index_graph(
    const mx::array& relation, int key_column, int payload_column,
    const mx::array* selection, mx::Stream stream, bool build_directory) {
  if (stream.device.type != mx::Device::gpu) {
    throw std::invalid_argument("mccl index construction requires a GPU stream");
  }
  auto& device = validate_index_device(stream);
  if (relation.ndim() != 2 || relation.shape(1) <= 0 ||
      (relation.dtype() != mx::uint32 && relation.dtype() != mx::int64) ||
      !relation.flags().row_contiguous) {
    throw std::invalid_argument("mccl relation must be contiguous uint32 or int64 [N,C], C>0");
  }
  const auto count = static_cast<size_t>(relation.shape(0));
  if (count > kMaxPairRows) {
    throw std::invalid_argument("mccl relation row count exceeds 1<<30");
  }
  if (key_column < 0 || key_column >= relation.shape(1) ||
      payload_column < -1 || payload_column >= relation.shape(1)) {
    throw std::invalid_argument("mccl relation index column is out of bounds");
  }
  validate_capacity(relation.size(), device, relation.itemsize());
  validate_capacity(count * 2, device);
  validate_capacity(kDirectoryEntries, device);
  std::vector<mx::array> inputs{relation};
  if (selection) {
    if (selection->ndim() != 2 || selection->shape(0) != relation.shape(0) ||
        selection->shape(1) != 1 || selection->dtype() != mx::uint32 ||
        !selection->flags().row_contiguous) {
      throw std::invalid_argument("mccl selection must be contiguous uint32 [N,1]");
    }
    validate_capacity(selection->size(), device);
    inputs.push_back(*selection);
  }
  return mx::array::make_arrays(
      {{static_cast<int>(count), 2},
       {build_directory ? static_cast<int>(kDirectoryEntries) : 0}, {2}},
      {mx::uint32, mx::uint32, mx::uint32},
      std::make_shared<RelationIndexOperation>(
          stream, key_column, payload_column, build_directory),
      inputs);
}

RelationIndexResult finish_relation_index_graph(std::vector<mx::array>& graph) {
  if (graph.size() != 3) {
    throw std::invalid_argument("mccl relation graph requires three sibling outputs");
  }
  mx::eval(graph);
  const auto* metadata = graph[2].data<uint32_t>();
  if (metadata[1] != std::numeric_limits<uint32_t>::max()) {
    throw std::invalid_argument(
        "mccl selected int64 key/payload is outside uint32 at source row " +
        std::to_string(metadata[1]));
  }
  const auto count = static_cast<size_t>(metadata[0]);
  if (count > static_cast<size_t>(graph[0].shape(0))) {
    throw std::runtime_error("mccl GPU selected count exceeds relation capacity");
  }
  // copy_shared_buffer shares the owning Data descriptor, not a weak buffer.
  // The concrete leaf exposes only initialized rows and has no capacity sibling
  // or lazy primitive that could later resurrect the inaccessible tail.
  mx::array rows({static_cast<int>(count), 2}, mx::uint32, nullptr, {});
  const mx::array::Flags flags{true, true, count <= 1};
  rows.copy_shared_buffer(graph[0], rows.strides(), flags, rows.size());
  rows.set_status(mx::array::Status::available);
  return {std::move(rows), graph[1], count};
}

} // namespace mccl::native

extern "C" int mccl_merge_sorted_u32(
    mlx_array* out, mlx_array a, mlx_array b, mlx_stream stream) {
  return capture_error([&] {
    if (!out) {
      throw std::invalid_argument("mccl output pointer must not be null");
    }
    const auto& left = mlx_array_get_(a);
    const auto& right = mlx_array_get_(b);
    validate_input(left);
    validate_input(right);
    const size_t count = left.size() + right.size();
    if (count > static_cast<size_t>(INT_MAX)) {
      throw std::invalid_argument("mccl combined input length exceeds INT_MAX");
    }
    const auto s = validate_stream(stream);
    mx::array result(
        {static_cast<int>(count)}, mx::uint32,
        std::make_shared<U32Operation>(s, Operation::Merge),
        {mx::contiguous(left, false, s), mx::contiguous(right, false, s)});
    mlx_array_set_(*out, std::move(result));
  });
}

extern "C" int mccl_copy_u32(mlx_array* out, mlx_array a, mlx_stream stream) {
  return capture_error([&] {
    if (!out) {
      throw std::invalid_argument("mccl output pointer must not be null");
    }
    const auto& input = mlx_array_get_(a);
    validate_input(input);
    const auto s = validate_stream(stream);
    mx::array result(
        input.shape(), mx::uint32,
        std::make_shared<U32Operation>(s, Operation::Copy),
        {mx::contiguous(input, false, s)});
    mlx_array_set_(*out, std::move(result));
  });
}

extern "C" int mccl_difference_sorted_u32(
    mlx_array* storage,
    mlx_array* count,
    mlx_array a,
    mlx_array b,
    mlx_stream stream) {
  return capture_error([&] {
    if (!storage || !count) {
      throw std::invalid_argument("mccl output pointers must not be null");
    }
    if (storage == count || (storage->ctx && storage->ctx == count->ctx)) {
      throw std::invalid_argument("mccl difference outputs must be distinct handles");
    }
    const auto& left = mlx_array_get_(a);
    const auto& right = mlx_array_get_(b);
    validate_input(left);
    validate_input(right);
    const auto s = validate_stream(stream);
    auto values = mx::contiguous(left, false, s);
    auto flags = [&] {
      if (left.size() == 0) {
        return mx::zeros(left.shape(), mx::uint32, s);
      }
      if (right.size() == 0) {
        return mx::ones(left.shape(), mx::uint32, s);
      }
      auto removals = mx::contiguous(right, false, s);
      auto ordinal = mx::arange(static_cast<double>(left.size()), mx::uint32, s);
      auto first = mx::searchsorted(values, values, "left", s);
      auto lower = mx::searchsorted(removals, values, "left", s);
      auto upper = mx::searchsorted(removals, values, "right", s);
      auto rank = mx::subtract(ordinal, first, s);
      auto multiplicity = mx::subtract(upper, lower, s);
      // On unsorted input the unsigned differences may wrap, but comparison
      // still produces only 0/1 flags. Their scan is bounded by len(A).
      return mx::astype(mx::greater_equal(rank, multiplicity, s), mx::uint32, s);
    }();
    flags = mx::contiguous(flags, false, s);
    auto offsets = left.size() == 0
        ? flags
        : mx::cumsum(flags, 0, false, false, mx::uint32, s);
    mx::array result(
        left.shape(), mx::uint32, std::make_shared<DifferenceScatter>(s),
        {values, flags, mx::contiguous(offsets, false, s)});
    auto result_count = left.size() == 0
        ? mx::zeros({}, mx::uint64, s)
        : mx::sum(mx::astype(flags, mx::uint64, s), s);

    // Build both graphs and allocate any missing handle contexts before
    // changing either output. Outputs may alias input handles, whose original
    // array descriptors have already been retained by the graphs above.
    auto storage_context = storage->ctx
        ? nullptr
        : std::make_unique<mx::array>(std::move(result));
    auto count_context = count->ctx
        ? nullptr
        : std::make_unique<mx::array>(std::move(result_count));
    if (storage_context) {
      storage->ctx = storage_context.release();
    } else {
      mlx_array_set_(*storage, std::move(result));
    }
    if (count_context) {
      count->ctx = count_context.release();
    } else {
      mlx_array_set_(*count, std::move(result_count));
    }
  });
}

extern "C" int mccl_bloom_create(
    mccl_bloom** out, uint32_t num_blocks, uint64_t seed, mlx_stream stream) {
  return capture_error([&] {
    if (!out) {
      throw std::invalid_argument("mccl Bloom output pointer must not be null");
    }
    if (num_blocks == 0 || num_blocks > static_cast<uint32_t>(INT_MAX / 8)) {
      throw std::invalid_argument(
          "mccl Bloom num_blocks must be positive and num_blocks * 8 <= INT_MAX");
    }
    auto filter = std::make_unique<mccl_bloom>(
        validate_stream(stream), num_blocks, seed);
    clear_bloom(*filter);
    *out = filter.release();
  });
}

extern "C" int mccl_bloom_free(mccl_bloom* filter) {
  return capture_error([&] {
    if (filter) {
      validate_bloom(filter);
      delete filter;
    }
  });
}

extern "C" int mccl_bloom_clear(mccl_bloom* filter) {
  return capture_error([&] { clear_bloom(validate_bloom(filter)); });
}

extern "C" int mccl_bloom_add(mccl_bloom* filter, mlx_array keys) {
  return capture_error([&] {
    auto& bloom = validate_bloom(filter);
    auto input = prepare_bloom_keys(bloom, keys);
    if (input.size() == 0) {
      mx::synchronize(bloom.stream);
      return;
    }
    auto pool = NS::TransferPtr(NS::AutoreleasePool::alloc()->init());
    auto* kernel = bloom_kernel(bloom, "mccl_bloom_add_kernel");
    auto& encoder = mx::metal::get_command_encoder(bloom.stream);
    encoder.add_temporary(input);
    encoder.add_temporary(bloom.bits);
    encoder.set_compute_pipeline_state(kernel);
    encoder.set_input_array(input, 0);
    // set_output_array also registers a read dependency for the in-place OR.
    encoder.set_output_array(bloom.bits, 1);
    const auto count = static_cast<uint32_t>(input.size());
    encoder.set_bytes(count, 2);
    encoder.set_bytes(bloom.num_blocks, 3);
    encoder.set_bytes(bloom.seed, 4);
    encoder.dispatch_threadgroups(
        MTL::Size((count + kThreads - 1) / kThreads, 1, 1),
        MTL::Size(kThreads, 1, 1));
    encoder.synchronize();
  });
}

extern "C" int mccl_bloom_contains(
    mlx_array* out, mccl_bloom* filter, mlx_array keys) {
  return capture_error([&] {
    if (!out) {
      throw std::invalid_argument("mccl output pointer must not be null");
    }
    auto& bloom = validate_bloom(filter);
    auto input = prepare_bloom_keys(bloom, keys);
    mx::array result(input.shape(), mx::uint32, nullptr, {});
    result.set_data(mx::allocator::malloc(result.nbytes()));
    if (input.size() != 0) {
      auto pool = NS::TransferPtr(NS::AutoreleasePool::alloc()->init());
      auto* kernel = bloom_kernel(bloom, "mccl_bloom_contains_kernel");
      auto& encoder = mx::metal::get_command_encoder(bloom.stream);
      encoder.add_temporary(input);
      encoder.add_temporary(bloom.bits);
      encoder.add_temporary(result);
      encoder.set_compute_pipeline_state(kernel);
      encoder.set_input_array(input, 0);
      encoder.set_input_array(bloom.bits, 1);
      encoder.set_output_array(result, 2);
      const auto count = static_cast<uint32_t>(input.size());
      encoder.set_bytes(count, 3);
      encoder.set_bytes(bloom.num_blocks, 4);
      encoder.set_bytes(bloom.seed, 5);
      encoder.dispatch_threadgroups(
          MTL::Size((count + kThreads - 1) / kThreads, 1, 1),
          MTL::Size(kThreads, 1, 1));
      encoder.synchronize();
    } else {
      mx::synchronize(bloom.stream);
    }
    result.set_status(mx::array::Status::available);
    mlx_array_set_(*out, std::move(result));
  });
}

extern "C" int mccl_bloom_read_words(
    mccl_bloom* filter, uint32_t* out, size_t words) {
  return capture_error([&] {
    auto& bloom = validate_bloom(filter);
    if (!out) {
      throw std::invalid_argument("mccl Bloom word output pointer must not be null");
    }
    if (words != bloom.bits.size()) {
      throw std::invalid_argument("mccl Bloom readback requires exactly num_blocks * 8 words");
    }
    mx::synchronize(bloom.stream);
    std::memcpy(out, bloom.bits.data<uint32_t>(), bloom.bits.nbytes());
  });
}

extern "C" int mccl_map_create(
    mccl_map** out, uint32_t capacity, uint32_t seed, mlx_stream stream) {
  return capture_error([&] {
    if (!out) {
      throw std::invalid_argument("mccl map output pointer must not be null");
    }
    const auto s = validate_stream(stream);
    validate_map_capacity(capacity, s);
    auto map = std::make_unique<mccl_map>(s, capacity, seed);
    clear_map(*map, true);
    *out = map.release();
  });
}

extern "C" int mccl_map_free(mccl_map* map) {
  return capture_error([&] {
    if (map) {
      validate_map(map);
      delete map;
    }
  });
}

extern "C" int mccl_map_clear(mccl_map* map) {
  return capture_error([&] { clear_map(validate_map(map)); });
}

extern "C" int mccl_map_insert(
    mccl_map* map,
    mlx_array keys,
    mlx_array values,
    uint64_t* inserted,
    uint64_t* full,
    uint64_t* invalid) {
  return capture_error([&] {
    auto& table = validate_map(map);
    if (!inserted || !full || !invalid) {
      throw std::invalid_argument("mccl map report pointers must not be null");
    }
    if (inserted == full || inserted == invalid || full == invalid) {
      throw std::invalid_argument("mccl map report pointers must be distinct");
    }
    const auto& key_input = mlx_array_get_(keys);
    const auto& value_input = mlx_array_get_(values);
    validate_input(key_input);
    validate_input(value_input);
    if (key_input.size() != value_input.size()) {
      throw std::invalid_argument("mccl map keys and values must have equal lengths");
    }
    const auto max_bytes =
        mx::metal::device(table.stream.device).mtl_device()->maxBufferLength();
    if (key_input.size() * sizeof(uint32_t) > max_bytes) {
      throw std::invalid_argument("mccl map input exceeds the Metal buffer limit");
    }
    auto input_keys = mx::contiguous(key_input, false, table.stream);
    auto input_values = mx::contiguous(value_input, false, table.stream);
    // Evaluate both graphs before directly mutating storage on the map stream.
    input_keys.eval();
    input_values.eval();
    if (input_keys.size() == 0) {
      mx::synchronize(table.stream);
      *inserted = *full = *invalid = 0;
      return;
    }

    const auto count = static_cast<uint32_t>(input_keys.size());
    const bool sparse = count <= table.capacity / 8;
    const uint64_t capacity = table.capacity;
    // Conservative maintenance policy bounds accumulated probe-chain debris;
    // small erases otherwise reuse holes without allocating a replacement table.
    const bool maintenance = table.tombstones != 0 &&
        (table.size == 0 ||
         table.tombstones >= std::max<uint64_t>(1, capacity / 16) ||
         uint64_t(table.size) + table.tombstones >= (15 * capacity + 15) / 16);
    const bool reuse = table.tombstones != 0 && !maintenance;
    const bool needs_slots = sparse || reuse;
    auto pool = NS::TransferPtr(NS::AutoreleasePool::alloc()->init());
    auto* reset = map_kernel(table, "mccl_map_reset_stats_kernel");
    auto* prelookup = reuse
        ? map_kernel(table, "mccl_map_prelookup_kernel")
        : nullptr;
    auto* claim = map_kernel(table, reuse
        ? "mccl_map_insert_reuse_kernel"
        : "mccl_map_insert_kernel");
    auto* publish = map_kernel(table, "mccl_map_publish_kernel");
    if (needs_slots && table.slots.size() < count) {
      table.slots = allocate_map_array(count);
    }
    if (maintenance) {
      if (table.size == 0) {
        clear_map(table);
      } else {
        rehash_map(table, table.capacity);
      }
    }
    auto& encoder = mx::metal::get_command_encoder(table.stream);
    encoder.add_temporary(input_keys);
    encoder.add_temporary(input_values);
    encoder.add_temporary(table.keys);
    encoder.add_temporary(table.values);
    encoder.add_temporary(table.winners);
    if (needs_slots) encoder.add_temporary(table.slots);
    reset_map_stats(table, encoder, reset);
    if (reuse) {
      encoder.set_compute_pipeline_state(prelookup);
      encoder.set_input_array(input_keys, 0);
      encoder.set_input_array(table.keys, 1);
      encoder.set_output_array(table.slots, 2);
      encoder.set_bytes(count, 3);
      encoder.set_bytes(table.capacity, 4);
      encoder.set_bytes(table.seed, 5);
      dispatch_map(encoder, count);
    }
    encoder.set_compute_pipeline_state(claim);
    encoder.set_input_array(input_keys, 0);
    encoder.set_output_array(table.keys, 1);
    encoder.set_output_array(table.winners, 2);
    encoder.set_output_array(table.stats, 3);
    // This read/write binding depends on prelookup's slot output even for dense
    // batches: MLX inserts a dispatch barrier before any key can be claimed.
    encoder.set_output_array(needs_slots ? table.slots : table.winners, 4);
    encoder.set_bytes(count, 5);
    encoder.set_bytes(table.capacity, 6);
    encoder.set_bytes(table.seed, 7);
    dispatch_map(encoder, count);

    // set_output_array also inserts the write/read barrier after the atomic
    // claim pass. Publication restores winners to zero before synchronization;
    // no query or later insertion can observe an unfinished publication.
    encoder.set_compute_pipeline_state(publish);
    encoder.set_input_array(input_values, 0);
    encoder.set_output_array(table.winners, 1);
    encoder.set_input_array(sparse ? table.slots : table.winners, 2);
    encoder.set_output_array(table.values, 3);
    encoder.set_bytes(count, 4);
    encoder.set_bytes(table.capacity, 5);
    dispatch_map(encoder, sparse ? count : table.capacity);
    encoder.synchronize();
    const auto* stats = table.stats.data<uint32_t>();
    table.size += stats[0];
    table.tombstones -= stats[3];
    *inserted = stats[0];
    *full = stats[1];
    *invalid = stats[2];
  });
}

extern "C" int mccl_map_find(
    mlx_array* values, mlx_array* found, mccl_map* map, mlx_array keys) {
  return capture_error([&] {
    if (!values || !found) {
      throw std::invalid_argument("mccl map lookup outputs must not be null");
    }
    if (values == found || (values->ctx && values->ctx == found->ctx)) {
      throw std::invalid_argument("mccl map lookup outputs must be distinct handles");
    }
    auto& table = validate_map(map);
    auto input = prepare_map_keys(table, keys);
    const auto count = static_cast<uint32_t>(input.size());
    auto result_values = allocate_map_array(count);
    auto result_found = allocate_map_array(count);
    if (count != 0) {
      auto pool = NS::TransferPtr(NS::AutoreleasePool::alloc()->init());
      const uint32_t width =
          table.capacity % 16 == 0 &&
              uint64_t(table.size + table.tombstones) * 4 >= uint64_t(table.capacity) * 3
          ? 16 : (table.capacity % 8 == 0 ? 8 : (table.capacity % 4 == 0 ? 4 : 1));
      const uint32_t variant = width == 1 ? 0 : (width == 4 ? 1 : (width == 8 ? 2 : 3));
      auto*& kernel = table.lookup_kernels[variant];
      if (!kernel) {
        auto& device = mx::metal::device(table.stream.device);
        auto* library = device.get_library("mccl", library_directory());
        auto* compiled = device.get_kernel(
            "mccl_map_find_kernel", library,
            "mccl_map_find_kernel_w" + std::to_string(width),
            {{&width, MTL::DataTypeUInt, 0}});
        if (compiled->maxTotalThreadsPerThreadgroup() < kThreads) {
          throw std::runtime_error("mccl kernel requires 256 threads per threadgroup");
        }
        kernel = compiled;
      }
      auto& encoder = mx::metal::get_command_encoder(table.stream);
      encoder.add_temporary(input);
      encoder.add_temporary(table.keys);
      encoder.add_temporary(table.values);
      encoder.add_temporary(result_values);
      encoder.add_temporary(result_found);
      encoder.set_compute_pipeline_state(kernel);
      encoder.set_input_array(input, 0);
      encoder.set_input_array(table.keys, 1);
      encoder.set_input_array(table.values, 2);
      encoder.set_output_array(result_values, 3);
      encoder.set_output_array(result_found, 4);
      encoder.set_bytes(count, 5);
      encoder.set_bytes(table.capacity, 6);
      encoder.set_bytes(table.seed, 7);
      dispatch_map(encoder, count);
      encoder.synchronize();
    } else {
      mx::synchronize(table.stream);
    }
    result_values.set_status(mx::array::Status::available);
    result_found.set_status(mx::array::Status::available);

    // As with difference, allocate missing contexts before publishing either
    // result. Input handles may alias an output: input retains its own storage.
    auto values_context = values->ctx
        ? nullptr
        : std::make_unique<mx::array>(std::move(result_values));
    auto found_context = found->ctx
        ? nullptr
        : std::make_unique<mx::array>(std::move(result_found));
    if (values_context) {
      values->ctx = values_context.release();
    } else {
      mlx_array_set_(*values, std::move(result_values));
    }
    if (found_context) {
      found->ctx = found_context.release();
    } else {
      mlx_array_set_(*found, std::move(result_found));
    }
  });
}

extern "C" int mccl_map_erase(
    mccl_map* map, mlx_array keys, uint64_t* erased) {
  return capture_error([&] {
    auto& table = validate_map(map);
    if (!erased) {
      throw std::invalid_argument("mccl map erased output pointer must not be null");
    }
    auto input = prepare_map_keys(table, keys);
    if (input.size() == 0) {
      mx::synchronize(table.stream);
      *erased = 0;
      return;
    }
    auto pool = NS::TransferPtr(NS::AutoreleasePool::alloc()->init());
    auto* reset = map_kernel(table, "mccl_map_reset_stats_kernel");
    auto* kernel = map_kernel(table, "mccl_map_erase_kernel");
    auto& encoder = mx::metal::get_command_encoder(table.stream);
    encoder.add_temporary(input);
    encoder.add_temporary(table.keys);
    reset_map_stats(table, encoder, reset);
    encoder.set_compute_pipeline_state(kernel);
    encoder.set_input_array(input, 0);
    encoder.set_output_array(table.keys, 1);
    encoder.set_output_array(table.stats, 2);
    const auto count = static_cast<uint32_t>(input.size());
    encoder.set_bytes(count, 3);
    encoder.set_bytes(table.capacity, 4);
    encoder.set_bytes(table.seed, 5);
    dispatch_map(encoder, count);
    encoder.synchronize();
    const auto removed = table.stats.data<uint32_t>()[0];
    table.size -= removed;
    table.tombstones += removed;
    *erased = removed;
  });
}

extern "C" int mccl_map_rehash(mccl_map* map, uint32_t capacity) {
  return capture_error([&] { rehash_map(validate_map(map), capacity); });
}

extern "C" const char* mccl_last_error(void) {
  return last_error;
}

extern "C" int mccl_allocate_u32(
    mlx_array* out, const int* shape, int ndim, uint32_t** data,
    mlx_stream stream) {
  return capture_error([&] {
    allocate_shared(out, shape, ndim, data, stream, mx::uint32);
  });
}

extern "C" int mccl_allocate_i64(
    mlx_array* out, const int* shape, int ndim, int64_t** data,
    mlx_stream stream) {
  return capture_error([&] {
    allocate_shared(out, shape, ndim, data, stream, mx::int64);
  });
}

extern "C" int mccl_relation_view_u32(
    mlx_array* out, mlx_array source, int columns, mlx_stream stream) {
  return capture_error([&] {
    if (!out) {
      throw std::invalid_argument("mccl output pointer must not be null");
    }
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    const auto& input = mlx_array_get_(source);
    if (input.ndim() != 1 || input.dtype() != mx::uint32 ||
        !input.flags().row_contiguous || columns <= 0 ||
        input.size() % static_cast<size_t>(columns) != 0) {
      throw std::invalid_argument("mccl relation view requires contiguous uint32 [N*C], C>0");
    }
    const auto count = input.size() / static_cast<size_t>(columns);
    if (count > kMaxPairRows) {
      throw std::invalid_argument("mccl relation row count exceeds 1<<30");
    }
    validate_capacity(input.size(), device);
    mx::array result(
        {static_cast<int>(count), columns}, mx::uint32,
        std::make_shared<RelationViewU32>(s), {input});
    mlx_array_set_(*out, std::move(result));
  });
}

extern "C" int mccl_build_relation_index_u32(
    mlx_array* out_rows, mlx_array* out_directory, size_t* out_count,
    mlx_array relation, int key_column, int payload_column,
    mlx_array selection, mlx_stream stream) {
  return capture_error([&] {
    if (!out_rows || !out_directory || !out_count ||
        out_rows == out_directory ||
        (out_rows->ctx && out_rows->ctx == out_directory->ctx)) {
      throw std::invalid_argument("mccl requires distinct output handles and a count pointer");
    }
    for (const auto* out : {out_rows, out_directory}) {
      if (out->ctx && (out->ctx == relation.ctx || out->ctx == selection.ctx)) {
        throw std::invalid_argument("mccl index outputs must not alias relation or selection handles");
      }
    }
    const auto s = validate_stream(stream);
    auto graph = mccl::native::make_relation_index_graph(
        mlx_array_get_(relation), key_column, payload_column,
        selection.ctx ? &mlx_array_get_(selection) : nullptr, s);
    auto result = mccl::native::finish_relation_index_graph(graph);
    // Prepare every potentially allocating publication step first. Official
    // mlx_array_new() handles have null ctx; failure of either allocation must
    // preserve BOTH prior handles as well as the caller's count.
    std::unique_ptr<mx::array> new_rows, new_directory;
    if (!out_rows->ctx) new_rows = std::make_unique<mx::array>(std::move(result.rows));
    if (!out_directory->ctx) {
      new_directory = std::make_unique<mx::array>(std::move(result.directory));
    }
    if (out_rows->ctx) {
      *static_cast<mx::array*>(out_rows->ctx) = std::move(result.rows);
    } else {
      out_rows->ctx = new_rows.release();
    }
    if (out_directory->ctx) {
      *static_cast<mx::array*>(out_directory->ctx) = std::move(result.directory);
    } else {
      out_directory->ctx = new_directory.release();
    }
    *out_count = result.count;
  });
}

extern "C" int mccl_sort_pairs_u32(
    mlx_array* out, mlx_array input, mlx_stream stream) {
  return capture_error([&] {
    if (!out) {
      throw std::invalid_argument("mccl output pointer must not be null");
    }
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    const auto& pairs = mlx_array_get_(input);
    validate_pairs(pairs, device);
    mx::array result(
        pairs.shape(), mx::uint32,
        std::make_shared<PairU32Operation>(s, PairOperation::Sort),
        {mx::contiguous(pairs, false, s)});
    mlx_array_set_(*out, std::move(result));
  });
}

extern "C" int mccl_merge_sorted_pairs_u32(
    mlx_array* out, mlx_array older, mlx_array newer, mlx_stream stream) {
  return capture_error([&] {
    if (!out) {
      throw std::invalid_argument("mccl output pointer must not be null");
    }
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    const auto& left = mlx_array_get_(older);
    const auto& right = mlx_array_get_(newer);
    validate_pairs(left, device);
    validate_pairs(right, device);
    const size_t count =
        static_cast<size_t>(left.shape(0)) + static_cast<size_t>(right.shape(0));
    if (count > kMaxPairRows) {
      throw std::invalid_argument("mccl merged pair row count exceeds 1<<30");
    }
    validate_capacity(count * 2, device);
    mx::array result(
        {static_cast<int>(count), 2}, mx::uint32,
        std::make_shared<PairU32Operation>(s, PairOperation::Merge),
        {mx::contiguous(left, false, s), mx::contiguous(right, false, s)});
    mlx_array_set_(*out, std::move(result));
  });
}

extern "C" int mccl_radix_directory_u32(
    mlx_array* out, mlx_array pairs, mlx_stream stream) {
  return capture_error([&] {
    if (!out) {
      throw std::invalid_argument("mccl output pointer must not be null");
    }
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    const auto& input = mlx_array_get_(pairs);
    validate_pairs(input, device);
    validate_capacity(kDirectoryEntries, device);
    mx::array result(
        {static_cast<int>(kDirectoryEntries)}, mx::uint32,
        std::make_shared<PairU32Operation>(s, PairOperation::Directory),
        {mx::contiguous(input, false, s)});
    mlx_array_set_(*out, std::move(result));
  });
}

namespace {

constexpr size_t kSplineChunk = 256;
constexpr size_t kSplineDirectory = 4098;

size_t spline_capacity(size_t keys) {
  // Must match radix_chunk_points in radix.metal; error32 forces nonterminal
  // anchors >=32 ranks apart, plus the exact first/last point of every chunk.
  const size_t bounded = ceil_div(keys, kSplineChunk) * 9;
  return keys < bounded ? keys : bounded;
}

class RadixOperation final : public mx::Primitive {
 public:
  RadixOperation(mx::Stream stream, bool merging, size_t rows, size_t capacity)
      : mx::Primitive(stream), merging_(merging), rows_(rows), capacity_(capacity) {}
  const char* name() const override { return "McclGroupedRadixU32"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("mccl radix construction requires a GPU stream");
  }
  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& device = validate_index_device(stream());
    for (const auto& input : inputs) {
      if (!input.flags().row_contiguous) {
        throw std::invalid_argument("mccl radix inputs must be contiguous");
      }
    }
    for (auto& output : outputs) allocate_storage(output);
    auto& encoder = mx::metal::get_command_encoder(stream());
    auto& keys = outputs[0];
    auto& offsets = outputs[1];
    auto& items = outputs[2];
    auto& points = outputs[3];
    auto& directory = outputs[4];
    auto& metadata = outputs[5];
    // Empty buffers may have no Metal allocation. Dummy bindings are never
    // dereferenced; all kernels guard reads with their GPU counts.
    const auto bindable = [&](const mx::array& a) -> const mx::array& {
      return a.size() == 0 ? metadata : a;
    };
    mx::array candidates = merging_
        ? allocate_leaf({static_cast<int>(capacity_), 2}, device) : inputs[0];
    mx::array source_meta = merging_ ? allocate_leaf({2}, device) : inputs[1];
    if (merging_) {
      encoder.add_temporary(candidates);
      encoder.add_temporary(source_meta);
      const uint32_t count_args[] = {static_cast<uint32_t>(capacity_), 0};
      encoder.set_compute_pipeline_state(index_kernel(device, "relation_metadata"));
      encoder.set_input_array(metadata, 0);
      encoder.set_output_array(source_meta, 1);
      encoder.set_bytes(count_args, 2, 2);
      dispatch(encoder, 1);
      for (size_t side = 0; side < 2; ++side) {
        const size_t own = side * 3;
        const size_t other = (1 - side) * 3;
        const auto count = static_cast<uint32_t>(inputs[own].size());
        if (count == 0) continue;
        auto deltas = allocate_leaf({static_cast<int>(count)}, device);
        encoder.add_temporary(deltas);
        const uint32_t args[] = {
            count, static_cast<uint32_t>(inputs[other].size()),
            static_cast<uint32_t>(side)};
        encoder.set_compute_pipeline_state(index_kernel(device, "radix_merge_keys"));
        encoder.set_input_array(inputs[own], 0);
        encoder.set_input_array(inputs[own + 1], 1);
        encoder.set_input_array(bindable(inputs[other]), 2);
        encoder.set_input_array(inputs[other + 1], 3);
        encoder.set_output_array(candidates, 4);
        encoder.set_output_array(deltas, 5);
        encoder.set_bytes(args, 3, 6);
        dispatch(encoder, ceil_div(count, kThreads));
        const uint32_t item_args[] = {
            static_cast<uint32_t>(inputs[own + 2].size()), count};
        encoder.set_compute_pipeline_state(index_kernel(device, "radix_merge_items"));
        encoder.set_input_array(inputs[own + 1], 0);
        encoder.set_input_array(inputs[own + 2], 1);
        encoder.set_input_array(deltas, 2);
        encoder.set_output_array(items, 3);
        encoder.set_bytes(item_args, 2, 4);
        dispatch(encoder, ceil_div(item_args[0], kThreads));
      }
    }
    std::vector<mx::array> groups;
    const auto capacity = static_cast<uint32_t>(capacity_);
    if (capacity != 0) {
      groups = allocate_scan(capacity, device, encoder);
      encoder.set_compute_pipeline_state(index_kernel(device, "radix_group_flags"));
      encoder.set_input_array(candidates, 0);
      encoder.set_input_array(source_meta, 1);
      encoder.set_output_array(groups[0], 2);
      encoder.set_bytes(capacity, 3);
      dispatch(encoder, ceil_div(capacity, kThreads));
      encode_scan(groups, device, encoder);
    }
    const uint32_t group_args[] = {
        capacity, uint32_t(merging_), static_cast<uint32_t>(rows_)};
    encoder.set_compute_pipeline_state(index_kernel(device, "radix_group_metadata"));
    encoder.set_input_array(source_meta, 0);
    encoder.set_input_array(groups.empty() ? source_meta : groups.back(), 1);
    encoder.set_output_array(metadata, 2);
    encoder.set_bytes(group_args, 3, 3);
    dispatch(encoder, 1);
    encoder.set_compute_pipeline_state(index_kernel(device, "radix_group_scatter"));
    encoder.set_input_array(bindable(candidates), 0);
    encoder.set_input_array(source_meta, 1);
    encoder.set_input_array(groups.empty() ? source_meta : groups[0], 2);
    encoder.set_input_array(metadata, 3);
    encoder.set_output_array(keys.size() == 0 ? offsets : keys, 4);
    encoder.set_output_array(offsets, 5);
    encoder.set_output_array(items.size() == 0 ? offsets : items, 6);
    encoder.set_bytes(uint32_t(merging_), 7);
    dispatch(encoder, ceil_div(capacity == 0 ? 1 : capacity, kThreads));

    const auto chunks = static_cast<uint32_t>(ceil_div(capacity, kSplineChunk));
    if (chunks != 0) {
      auto scratch = allocate_leaf(points.shape(), device, mx::uint64);
      encoder.add_temporary(scratch);
      auto counts = allocate_scan(chunks, device, encoder);
      encoder.set_compute_pipeline_state(index_kernel(device, "radix_spline_chunks"));
      encoder.set_input_array(keys, 0);
      encoder.set_input_array(metadata, 1);
      encoder.set_output_array(scratch, 2);
      encoder.set_output_array(counts[0], 3);
      encoder.set_bytes(chunks, 4);
      dispatch(encoder, ceil_div(chunks, kThreads));
      encode_scan(counts, device, encoder);
      encoder.set_compute_pipeline_state(index_kernel(device, "radix_spline_metadata"));
      encoder.set_input_array(keys, 0);
      encoder.set_input_array(counts.back(), 1);
      encoder.set_output_array(metadata, 2);
      encoder.set_bytes(chunks, 3);
      dispatch(encoder, 1);
      encoder.set_compute_pipeline_state(index_kernel(device, "radix_spline_compact"));
      encoder.set_input_array(scratch, 0);
      encoder.set_input_array(counts[0], 1);
      encoder.set_input_array(metadata, 2);
      encoder.set_output_array(points, 3);
      encoder.set_bytes(chunks, 4);
      dispatch(encoder, ceil_div(chunks, kThreads));
      // Coalesce exact collinear chains, including redundant chunk endpoints.
      // Reuse the no-longer-needed chunk scratch; never compact in place while
      // neighboring threads still read the original spline.
      const auto point_capacity = static_cast<uint32_t>(points.shape(0));
      auto retained = allocate_scan(point_capacity, device, encoder);
      encoder.set_compute_pipeline_state(index_kernel(device, "radix_spline_keep_flags"));
      encoder.set_input_array(points, 0);
      encoder.set_input_array(metadata, 1);
      encoder.set_output_array(retained[0], 2);
      encoder.set_bytes(point_capacity, 3);
      dispatch(encoder, ceil_div(point_capacity, kThreads));
      encode_scan(retained, device, encoder);
      encoder.set_compute_pipeline_state(index_kernel(device, "radix_spline_coalesce"));
      encoder.set_input_array(points, 0);
      encoder.set_input_array(metadata, 1);
      encoder.set_input_array(retained[0], 2);
      encoder.set_input_array(retained.back(), 3);
      encoder.set_output_array(scratch, 4);
      dispatch(encoder, ceil_div(point_capacity, kThreads));
      encoder.set_compute_pipeline_state(index_kernel(device, "radix_spline_coalesced_copy"));
      encoder.set_input_array(scratch, 0);
      encoder.set_input_array(retained.back(), 1);
      encoder.set_output_array(points, 2);
      encoder.set_output_array(metadata, 3);
      dispatch(encoder, ceil_div(point_capacity, kThreads));
    }
    encoder.set_compute_pipeline_state(index_kernel(device, "radix_spline_directory"));
    encoder.set_input_array(bindable(points), 0);
    encoder.set_input_array(metadata, 1);
    encoder.set_output_array(directory, 2);
    dispatch(encoder, ceil_div(kSplineDirectory, kThreads));
  }
 private:
  bool merging_;
  size_t rows_;
  size_t capacity_;
};

std::vector<mx::array> make_radix_graph(
    const std::vector<mx::array>& inputs, bool merging, size_t rows,
    size_t capacity, mx::Stream stream) {
  auto& device = validate_index_device(stream);
  if (rows > kMaxPairRows || capacity > rows) {
    throw std::invalid_argument("mccl radix row/key capacity exceeds 1<<30");
  }
  validate_capacity(capacity, device);
  validate_capacity(capacity + 1, device, sizeof(uint64_t));
  validate_capacity(rows, device);
  validate_capacity(spline_capacity(capacity) * 2, device, sizeof(uint64_t));
  validate_capacity(kSplineDirectory, device, sizeof(uint64_t));
  return mx::array::make_arrays(
      {{static_cast<int>(capacity)}, {static_cast<int>(capacity + 1)},
       {static_cast<int>(rows)}, {static_cast<int>(spline_capacity(capacity)), 2},
       {static_cast<int>(kSplineDirectory)}, {8}},
      {mx::uint32, mx::uint64, mx::uint32, mx::uint64, mx::uint64, mx::uint64},
      std::make_shared<RadixOperation>(stream, merging, rows, capacity), inputs);
}

std::array<mlx_array*, 5> radix_handles(mccl_radix_index* index) {
  return {&index->keys, &index->offsets, &index->items, &index->points, &index->radix};
}

std::vector<mlx_array> radix_inputs(const mccl_radix_index* index) {
  return {index->keys, index->offsets, index->items, index->points, index->radix};
}

void validate_radix_outputs(mccl_radix_index* out, const std::vector<mlx_array>& inputs) {
  if (!out) throw std::invalid_argument("mccl radix output must not be null");
  const auto handles = radix_handles(out);
  for (size_t i = 0; i < handles.size(); ++i) {
    if (!handles[i]->ctx) continue; // Official mlx_array_new() is a null ctx.
    for (size_t j = 0; j < i; ++j) {
      if (handles[i]->ctx == handles[j]->ctx) {
        throw std::invalid_argument("mccl radix output handles must be distinct");
      }
    }
    for (auto input : inputs) {
      if (handles[i]->ctx == input.ctx) {
        throw std::invalid_argument("mccl radix outputs must not alias input handles");
      }
    }
  }
}

void validate_completed_radix(const mccl_radix_index* index, mx::metal::Device& device) {
  if (!index || index->rows > kMaxPairRows || index->key_count > index->rows ||
      index->point_count > index->key_count ||
      (index->rows != 0 && (index->key_count == 0 || index->point_count == 0)) ||
      (index->key_count > 1 && index->point_count < 2) ||
      index->min_key > index->max_key || index->max_key > UINT32_MAX ||
      index->shift > 20 || index->search_error != (index->rows == 0 ? 32u : 33u) ||
      (index->rows == 0 && (index->min_key != 0 || index->max_key != 0 || index->shift != 0))) {
    throw std::invalid_argument("mccl invalid completed radix metadata");
  }
  const auto handles = radix_inputs(index);
  const mx::Shape shapes[] = {
      {static_cast<int>(index->key_count)}, {static_cast<int>(index->key_count + 1)},
      {static_cast<int>(index->rows)}, {static_cast<int>(index->point_count), 2},
      {static_cast<int>(kSplineDirectory)}};
  const mx::Dtype dtypes[] = {mx::uint32, mx::uint64, mx::uint32, mx::uint64, mx::uint64};
  for (size_t i = 0; i < handles.size(); ++i) {
    if (!handles[i].ctx) throw std::invalid_argument("mccl missing completed radix array");
    const auto& array = mlx_array_get_(handles[i]);
    if (array.shape() != shapes[i] || array.dtype() != dtypes[i] ||
        !array.flags().row_contiguous || array.status() != mx::array::Status::available ||
        (array.size() != 0 && !array.data<void>())) {
      throw std::invalid_argument("mccl completed radix array shape/type/readiness mismatch");
    }
    validate_capacity(array.size(), device, array.itemsize());
  }
}

mx::array radix_prefix(const mx::array& source, mx::Shape shape) {
  mx::array result(std::move(shape), source.dtype(), nullptr, {});
  const mx::array::Flags flags{
      true, true, result.ndim() == 1 || result.shape(0) <= 1};
  result.copy_shared_buffer(source, result.strides(), flags, result.size());
  result.set_status(mx::array::Status::available);
  return result;
}

void publish_radix(mccl_radix_index* out, std::vector<mx::array>& graph) {
  mx::eval(graph);
  const auto* meta = graph[5].data<uint64_t>();
  if (meta[7] != UINT32_MAX) {
    throw std::invalid_argument(
        "mccl selected int64 key/payload is outside uint32 at source row " +
        std::to_string(meta[7]));
  }
  if (meta[0] > graph[2].size() || meta[1] > graph[0].size() ||
      meta[2] > static_cast<size_t>(graph[3].shape(0)) || meta[1] > meta[0] ||
      meta[2] > meta[1] || (meta[1] != 0 && meta[2] == 0) ||
      (meta[1] > 1 && meta[2] < 2) || meta[3] > meta[4] ||
      meta[4] > UINT32_MAX || meta[5] > 20 ||
      meta[6] != (meta[1] == 0 ? 32u : 33u)) {
    throw std::runtime_error("mccl GPU radix metadata exceeds output capacity");
  }
  std::vector<mx::array> result;
  result.reserve(5);
  result.push_back(radix_prefix(graph[0], {static_cast<int>(meta[1])}));
  result.push_back(radix_prefix(graph[1], {static_cast<int>(meta[1] + 1)}));
  result.push_back(radix_prefix(graph[2], {static_cast<int>(meta[0])}));
  result.push_back(radix_prefix(graph[3], {static_cast<int>(meta[2]), 2}));
  result.push_back(radix_prefix(graph[4], {static_cast<int>(kSplineDirectory)}));
  const auto handles = radix_handles(out);
  std::unique_ptr<mx::array> fresh[5];
  // Complete every allocation before any externally visible mutation.
  for (size_t i = 0; i < handles.size(); ++i) {
    if (!handles[i]->ctx) fresh[i] = std::make_unique<mx::array>(std::move(result[i]));
  }
  for (size_t i = 0; i < handles.size(); ++i) {
    if (handles[i]->ctx) *static_cast<mx::array*>(handles[i]->ctx) = std::move(result[i]);
    else handles[i]->ctx = fresh[i].release();
  }
  out->rows = meta[0];
  out->key_count = meta[1];
  out->point_count = meta[2];
  out->min_key = meta[3];
  out->max_key = meta[4];
  out->shift = static_cast<uint32_t>(meta[5]);
  out->search_error = static_cast<uint32_t>(meta[6]);
}

constexpr size_t kJoinTile = 2048; // Must match join_expand in join.metal.

// Probe ranks/counts, then the exact exclusive u64 output offsets in place.
class RadixJoinCountOperation final : public mx::Primitive {
 public:
  explicit RadixJoinCountOperation(mx::Stream stream) : mx::Primitive(stream) {}
  const char* name() const override { return "McclRadixJoinCountU32"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("mccl radix join requires a GPU stream");
  }
  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& device = validate_index_device(stream());
    for (const auto& input : inputs) {
      if (!input.flags().row_contiguous) {
        throw std::invalid_argument("mccl radix join inputs must be contiguous");
      }
    }
    for (auto& output : outputs) allocate_storage(output);
    auto& encoder = mx::metal::get_command_encoder(stream());
    const auto& probes = inputs[0];
    const auto& keys = inputs[1];
    const auto& offsets = inputs[2];
    auto& ranks = outputs[0];
    auto& prefix = outputs[1];
    // offsets always holds its terminal entry; empty arrays bind it as a
    // never-dereferenced dummy.
    const auto bindable = [&](const mx::array& a) -> const mx::array& {
      return a.size() == 0 ? offsets : a;
    };
    const uint32_t args[] = {
        static_cast<uint32_t>(probes.size()), static_cast<uint32_t>(keys.size())};
    encoder.set_compute_pipeline_state(index_kernel(device, "join_probe_counts"));
    encoder.set_input_array(bindable(probes), 0);
    encoder.set_input_array(bindable(keys), 1);
    encoder.set_input_array(offsets, 2);
    encoder.set_output_array(ranks.size() == 0 ? prefix : ranks, 3);
    encoder.set_output_array(prefix, 4);
    encoder.set_bytes(args, 2, 5);
    dispatch(encoder, ceil_div(prefix.size(), kThreads));
    std::vector<mx::array> levels{prefix};
    size_t length = prefix.size();
    do {
      length = ceil_div(length, kTile);
      levels.push_back(allocate_leaf({static_cast<int>(length)}, device, mx::uint64));
      encoder.add_temporary(levels.back());
    } while (length > 1);
    encode_scan(
        levels, device, encoder, "join_scan_blocks_u64", "join_add_block_offsets_u64");
  }
};

// args: total outputs (<= 2^30), probes. inputs: prefix, ranks, offsets, items.
void encode_join_expand(
    const std::vector<mx::array>& inputs, mx::array& pairs, size_t total, size_t probes,
    mx::metal::Device& device, mx::metal::CommandEncoder& encoder) {
  const uint32_t args[] = {static_cast<uint32_t>(total), static_cast<uint32_t>(probes)};
  encoder.set_compute_pipeline_state(index_kernel(device, "join_expand"));
  encoder.set_input_array(inputs[0], 0);
  encoder.set_input_array(inputs[1], 1);
  encoder.set_input_array(inputs[2], 2);
  encoder.set_input_array(inputs[3], 3);
  encoder.set_output_array(pairs, 4);
  encoder.set_bytes(args, 2, 5);
  dispatch(encoder, ceil_div(total, kJoinTile));
}

// A capacity-sized u32 row-ID set read with stride/offset (join pairs read
// their row column). Its live length is count[0], written on the GPU by the
// producing stage, so chained stages never wait on a host readback.
struct RowSet {
  const mx::array* rows;
  uint32_t stride;
  uint32_t offset;
  const mx::array* count;
  uint32_t capacity;
};

// A one-element count written through the shared mapping before encoding.
mx::array count_leaf(
    uint32_t value, mx::metal::Device& device, mx::metal::CommandEncoder& encoder) {
  auto leaf = allocate_leaf({1}, device);
  leaf.data<uint32_t>()[0] = value;
  encoder.add_temporary(leaf);
  return leaf;
}

mx::array flag_prefix(
    uint32_t capacity, mx::metal::Device& device, mx::metal::CommandEncoder& encoder) {
  auto prefix = allocate_leaf({static_cast<int>(capacity + 1)}, device);
  encoder.add_temporary(prefix);
  return prefix;
}

void scan_flags(
    mx::array& prefix, mx::metal::Device& device, mx::metal::CommandEncoder& encoder) {
  std::vector<mx::array> levels{prefix};
  size_t length = prefix.size();
  do {
    length = ceil_div(length, kTile);
    levels.push_back(allocate_leaf({static_cast<int>(length)}, device));
    encoder.add_temporary(levels.back());
  } while (length > 1);
  encode_scan(levels, device, encoder);
}

void clear_invalid(
    mx::array& invalid, mx::metal::Device& device, mx::metal::CommandEncoder& encoder) {
  encoder.set_compute_pipeline_state(index_kernel(device, "distinct_clear"));
  encoder.set_output_array(invalid, 0);
  dispatch(encoder, 1);
}

struct SemiJoinStage {
  uint32_t key_column;
  uint32_t item_column; // UINT32_MAX: none
  uint32_t key_count;
};

// Keep rows whose key (and optional item) occurs in a grouped run. Writes
// kept rows (capacity-sized, input order) and kept_count[0]; invalid[0] is
// set by a row ID outside the relation.
void encode_semijoin(
    const RowSet& in, const mx::array& relation, const mx::array& keys,
    const mx::array& offsets, const mx::array& items, SemiJoinStage stage,
    mx::array& kept, mx::array& kept_count, mx::array& invalid,
    mx::metal::Device& device, mx::metal::CommandEncoder& encoder) {
  const uint32_t capacity = in.capacity;
  auto prefix = flag_prefix(capacity, device, encoder);
  // Empty arrays may have no Metal buffer; their bounds prevent any access.
  const auto bindable = [&](const mx::array& a) -> const mx::array& {
    return a.size() == 0 ? prefix : a;
  };
  const std::array<uint32_t, 8> args{
      capacity, in.stride, in.offset, static_cast<uint32_t>(relation.shape(1)),
      stage.key_column, stage.item_column, stage.key_count,
      static_cast<uint32_t>(relation.shape(0))};
  encoder.set_compute_pipeline_state(index_kernel(device, "join_semi_flags"));
  encoder.set_input_array(bindable(*in.rows), 0);
  encoder.set_input_array(bindable(relation), 1);
  encoder.set_input_array(bindable(keys), 2);
  encoder.set_input_array(offsets, 3);
  encoder.set_input_array(bindable(items), 4);
  encoder.set_output_array(prefix, 5);
  encoder.set_output_array(invalid, 6);
  encoder.set_bytes(args.data(), args.size(), 7);
  encoder.set_input_array(*in.count, 8);
  dispatch(encoder, ceil_div(capacity + 1, kThreads));
  scan_flags(prefix, device, encoder);
  const uint32_t scatter[] = {capacity, in.stride, in.offset};
  encoder.set_compute_pipeline_state(index_kernel(device, "join_semi_scatter"));
  encoder.set_input_array(bindable(*in.rows), 0);
  encoder.set_input_array(prefix, 1);
  encoder.set_output_array(kept.size() == 0 ? kept_count : kept, 2);
  encoder.set_bytes(scatter, 3, 3);
  encoder.set_output_array(kept_count, 4);
  dispatch(encoder, ceil_div(std::max<uint32_t>(capacity, 1), kThreads));
}

constexpr size_t kMaxDistinctColumns = 4; // Must match distinct_* in join.metal.
using DistinctTuple = std::array<uint32_t, 3 + kMaxDistinctColumns>;
// Bit width of each tuple column's field; 32 unless a bound is proven.
using DistinctWidths = std::array<uint32_t, kMaxDistinctColumns>;

constexpr DistinctWidths kFullWidths{32, 32, 32, 32};

// Distinct int64 tuples of the live rows. Columns are packed into one
// composite key (first column most significant, each in its proven width),
// which is sorted by LSD stable passes over 32-bit slices, visiting only the
// bytes the composite occupies. Writes out (capacity-sized) and
// distinct_count[0]; invalid {row outside relation, value outside its field}.
void encode_distinct(
    const RowSet& in, const mx::array& relation, DistinctTuple tuple,
    const DistinctWidths& widths, mx::array& out, mx::array& distinct_count,
    mx::array& invalid, mx::metal::Device& device, mx::metal::CommandEncoder& encoder) {
  const uint32_t capacity = in.capacity;
  tuple[0] = capacity;
  auto prefix = flag_prefix(capacity, device, encoder);
  const auto bindable = [&](const mx::array& a) -> const mx::array& {
    return a.size() == 0 ? prefix : a;
  };
  auto gathered = allocate_leaf({static_cast<int>(capacity), 2}, device);
  auto sorted = allocate_leaf({static_cast<int>(capacity), 2}, device);
  encoder.add_temporary(gathered);
  encoder.add_temporary(sorted);
  // args: capacity, stride, offset, columns, source_rows, slice_lo, count,
  // then {column, width, lo} per column; the last column is least significant.
  std::array<uint32_t, 7 + 3 * kMaxDistinctColumns> args{
      capacity, in.stride, in.offset, static_cast<uint32_t>(relation.shape(1)),
      static_cast<uint32_t>(relation.shape(0)), 0, tuple[2]};
  uint32_t bits = 0;
  for (uint32_t k = tuple[2]; k-- > 0;) {
    args[7 + 3 * k] = tuple[3 + k];
    args[8 + 3 * k] = widths[k];
    args[9 + 3 * k] = bits;
    bits += widths[k];
  }
  if (capacity != 0) {
    auto* gather = index_kernel(device, "distinct_gather_key");
    // At least one slice runs, so a zero-width composite still validates rows.
    for (uint32_t lo = 0; lo == 0 || lo < bits; lo += 32) {
      const bool first = lo == 0;
      args[1] = first ? in.stride : 2u;
      args[2] = first ? in.offset : 1u;
      args[5] = lo;
      encoder.set_compute_pipeline_state(gather);
      encoder.set_input_array(first ? *in.rows : sorted, 0);
      encoder.set_input_array(bindable(relation), 1);
      encoder.set_output_array(gathered, 2);
      encoder.set_output_array(invalid, 3);
      encoder.set_bytes(args.data(), args.size(), 4);
      encoder.set_input_array(*in.count, 5);
      dispatch(encoder, ceil_div(capacity, kThreads));
      const uint32_t slice = std::min<uint32_t>(32, bits - lo);
      // The dynamic sort reads the live length from count[0].
      encode_sort(gathered, sorted, device, encoder, in.count,
          std::max<uint32_t>(1, (slice + 7) / 8));
    }
  }
  encoder.set_compute_pipeline_state(index_kernel(device, "distinct_flags"));
  encoder.set_input_array(bindable(sorted), 0);
  encoder.set_input_array(bindable(relation), 1);
  encoder.set_output_array(prefix, 2);
  encoder.set_bytes(tuple.data(), tuple.size(), 3);
  encoder.set_input_array(*in.count, 4);
  dispatch(encoder, ceil_div(capacity + 1, kThreads));
  scan_flags(prefix, device, encoder);
  encoder.set_compute_pipeline_state(index_kernel(device, "distinct_scatter"));
  encoder.set_input_array(bindable(sorted), 0);
  encoder.set_input_array(prefix, 1);
  encoder.set_input_array(bindable(relation), 2);
  encoder.set_output_array(out.size() == 0 ? distinct_count : out, 3);
  encoder.set_bytes(tuple.data(), tuple.size(), 4);
  encoder.set_output_array(distinct_count, 5);
  dispatch(encoder, ceil_div(std::max<uint32_t>(capacity, 1), kThreads));
}

void require_contiguous(const std::vector<mx::array>& inputs, const char* what) {
  for (const auto& input : inputs) {
    if (!input.flags().row_contiguous) {
      throw std::invalid_argument(std::string("mccl ") + what + " inputs must be contiguous");
    }
  }
}

// inputs: rows, relation, keys, offsets, items.
// outputs: kept rows (capacity n), kept count [1], invalid [2].
class RadixSemiJoinOperation final : public mx::Primitive {
 public:
  RadixSemiJoinOperation(
      mx::Stream stream, uint32_t stride, uint32_t offset, SemiJoinStage stage)
      : mx::Primitive(stream), stride_(stride), offset_(offset), stage_(stage) {}
  const char* name() const override { return "McclRadixSemiJoinU32"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("mccl radix semijoin requires a GPU stream");
  }
  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& device = validate_index_device(stream());
    require_contiguous(inputs, "radix semijoin");
    for (auto& output : outputs) allocate_storage(output);
    auto& encoder = mx::metal::get_command_encoder(stream());
    clear_invalid(outputs[2], device, encoder);
    const auto n = static_cast<uint32_t>(inputs[0].shape(0));
    auto count = count_leaf(n, device, encoder);
    encode_semijoin(
        {&inputs[0], stride_, offset_, &count, n}, inputs[1], inputs[2], inputs[3],
        inputs[4], stage_, outputs[0], outputs[1], outputs[2], device, encoder);
  }
 private:
  uint32_t stride_;
  uint32_t offset_;
  SemiJoinStage stage_;
};

// inputs: rows, relation. outputs: tuples (capacity n), count [1], invalid [2].
class DistinctRowsOperation final : public mx::Primitive {
 public:
  DistinctRowsOperation(
      mx::Stream stream, uint32_t stride, uint32_t offset, DistinctTuple tuple)
      : mx::Primitive(stream), stride_(stride), offset_(offset), tuple_(tuple) {}
  const char* name() const override { return "McclDistinctRowsI64"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("mccl distinct rows requires a GPU stream");
  }
  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& device = validate_index_device(stream());
    require_contiguous(inputs, "distinct rows");
    for (auto& output : outputs) allocate_storage(output);
    auto& encoder = mx::metal::get_command_encoder(stream());
    clear_invalid(outputs[2], device, encoder);
    const auto n = static_cast<uint32_t>(inputs[0].shape(0));
    auto count = count_leaf(n, device, encoder);
    encode_distinct(
        {&inputs[0], stride_, offset_, &count, n}, inputs[1], tuple_, kFullWidths,
        outputs[0], outputs[1], outputs[2], device, encoder);
  }
 private:
  uint32_t stride_;
  uint32_t offset_;
  DistinctTuple tuple_;
};

// Join expansion, chained semijoins and distinct tuples as one GPU graph:
// every intermediate length stays on the device. inputs: join prefix, ranks,
// join offsets, join items, relation, then keys/offsets/items per stage.
// outputs: tuples (capacity total), count [1], invalid [2].
class JoinChainOperation final : public mx::Primitive {
 public:
  JoinChainOperation(
      mx::Stream stream, size_t total, size_t probes,
      std::vector<SemiJoinStage> stages, DistinctTuple tuple, DistinctWidths widths)
      : mx::Primitive(stream), total_(total), probes_(probes),
        stages_(std::move(stages)), tuple_(tuple), widths_(widths) {}
  const char* name() const override { return "McclJoinSemiJoinDistinct"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("mccl join chain requires a GPU stream");
  }
  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& device = validate_index_device(stream());
    require_contiguous(inputs, "join chain");
    for (auto& output : outputs) allocate_storage(output);
    auto& encoder = mx::metal::get_command_encoder(stream());
    clear_invalid(outputs[2], device, encoder);
    const auto capacity = static_cast<uint32_t>(total_);
    auto pairs = allocate_leaf({static_cast<int>(capacity), 2}, device);
    encoder.add_temporary(pairs);
    encode_join_expand(inputs, pairs, total_, probes_, device, encoder);
    // Stable storage for every stage's rows/count: RowSet holds pointers.
    std::vector<mx::array> rows;
    std::vector<mx::array> counts;
    rows.reserve(stages_.size());
    counts.reserve(stages_.size() + 1);
    counts.push_back(count_leaf(capacity, device, encoder));
    RowSet current{&pairs, 2, 1, &counts.back(), capacity};
    const auto& relation = inputs[4];
    for (size_t s = 0; s < stages_.size(); ++s) {
      rows.push_back(allocate_leaf({static_cast<int>(capacity)}, device));
      counts.push_back(allocate_leaf({1}, device));
      encoder.add_temporary(rows.back());
      encoder.add_temporary(counts.back());
      encode_semijoin(
          current, relation, inputs[5 + 3 * s], inputs[6 + 3 * s], inputs[7 + 3 * s],
          stages_[s], rows.back(), counts.back(), outputs[2], device, encoder);
      current = {&rows.back(), 1, 0, &counts.back(), capacity};
    }
    encode_distinct(current, relation, tuple_, widths_, outputs[0], outputs[1],
        outputs[2], device, encoder);
  }
 private:
  size_t total_;
  size_t probes_;
  std::vector<SemiJoinStage> stages_;
  DistinctTuple tuple_;
  DistinctWidths widths_;
};

// Fixed output tiles: fanout skew spreads across threadgroups.
class RadixJoinExpandOperation final : public mx::Primitive {
 public:
  RadixJoinExpandOperation(mx::Stream stream, size_t total, size_t probes)
      : mx::Primitive(stream), total_(total), probes_(probes) {}
  const char* name() const override { return "McclRadixJoinExpandU32"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("mccl radix join requires a GPU stream");
  }
  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& device = validate_index_device(stream());
    for (const auto& input : inputs) {
      if (!input.flags().row_contiguous) {
        throw std::invalid_argument("mccl radix join inputs must be contiguous");
      }
    }
    allocate_storage(outputs[0]);
    if (total_ == 0) return;
    encode_join_expand(inputs, outputs[0], total_, probes_, device,
        mx::metal::get_command_encoder(stream()));
  }
 private:
  size_t total_;
  size_t probes_;
};

// Probe ranks and exact u64 output offsets, evaluated: the one host readback
// that sizes every later buffer. Returns {ranks, prefix}; total is prefix[n].
std::vector<mx::array> count_join(
    const mccl_radix_index* index, const mx::array& keys, mx::Stream s,
    mx::metal::Device& device, uint64_t& total) {
  if (keys.ndim() != 1 || keys.dtype() != mx::uint32) {
    throw std::invalid_argument("mccl radix join probes must have shape [N] and dtype uint32");
  }
  const size_t n = keys.size();
  if (n > kMaxPairRows) {
    throw std::invalid_argument("mccl radix join probe count exceeds 1<<30");
  }
  validate_capacity(n, device);
  validate_capacity(n + 1, device, sizeof(uint64_t));
  auto counted = mx::array::make_arrays(
      {{static_cast<int>(n)}, {static_cast<int>(n + 1)}}, {mx::uint32, mx::uint64},
      std::make_shared<RadixJoinCountOperation>(s),
      {keys, mlx_array_get_(index->keys), mlx_array_get_(index->offsets)});
  mx::eval(counted);
  // The exclusive scan's terminal entry is the exact pair count.
  total = counted[1].data<uint64_t>()[n];
  if (total > kMaxPairRows) {
    throw std::invalid_argument(
        "mccl radix join output exceeds 1<<30 pairs: " + std::to_string(total));
  }
  validate_capacity(total * 2, device);
  return counted;
}

void validate_relation_i64(const mx::array& source, const char* what) {
  if (source.ndim() != 2 || source.dtype() != mx::int64) {
    throw std::invalid_argument(std::string("mccl ") + what +
        " relation must be int64 [rows,columns]");
  }
  if (static_cast<size_t>(source.shape(0)) > kMaxPairRows) {
    throw std::invalid_argument(std::string("mccl ") + what + " relation exceeds 1<<30 rows");
  }
}

// Row IDs [N] or join pairs [N,2] (row column read) over an int64 relation.
struct RowsInput {
  uint32_t stride;
  uint32_t offset;
};

RowsInput validate_rows_input(const mx::array& ids, const mx::array& source, const char* what) {
  const bool pairs = ids.ndim() == 2 && ids.shape(1) == 2;
  if (ids.dtype() != mx::uint32 || !(ids.ndim() == 1 || pairs)) {
    throw std::invalid_argument(std::string("mccl ") + what +
        " rows must be uint32 [N] or [N,2] pairs");
  }
  validate_relation_i64(source, what);
  if (static_cast<size_t>(ids.shape(0)) > kMaxPairRows) {
    throw std::invalid_argument(std::string("mccl ") + what + " row count exceeds 1<<30");
  }
  return {pairs ? 2u : 1u, pairs ? 1u : 0u};
}

SemiJoinStage semijoin_stage(
    const mx::array& source, int key_column, int item_column,
    const mccl_radix_index* index) {
  const int columns = source.shape(1);
  if (key_column < 0 || key_column >= columns || item_column < -1 ||
      item_column >= columns) {
    throw std::invalid_argument("mccl radix semijoin column is outside the relation");
  }
  return {static_cast<uint32_t>(key_column),
          item_column < 0 ? UINT32_MAX : static_cast<uint32_t>(item_column),
          static_cast<uint32_t>(index->key_count)};
}

DistinctTuple distinct_tuple(
    const mx::array& source, const int* columns, int column_count, size_t capacity,
    mx::metal::Device& device) {
  if (!columns || column_count < 1 ||
      column_count > static_cast<int>(kMaxDistinctColumns)) {
    throw std::invalid_argument("mccl distinct rows needs 1 to 4 columns");
  }
  DistinctTuple tuple{};
  for (int k = 0; k < column_count; ++k) {
    if (columns[k] < 0 || columns[k] >= source.shape(1)) {
      throw std::invalid_argument("mccl distinct rows column is outside the relation");
    }
    tuple[3 + k] = static_cast<uint32_t>(columns[k]);
  }
  validate_capacity(capacity + 1, device);
  validate_capacity(capacity * 2, device);
  validate_capacity(capacity * column_count, device, sizeof(int64_t));
  tuple[1] = static_cast<uint32_t>(source.shape(1));
  tuple[2] = static_cast<uint32_t>(column_count);
  return tuple;
}

// Deferred GPU validation flags: {row ID outside relation, value outside u32}.
void check_invalid(const mx::array& invalid, const char* what) {
  const auto* flags = invalid.data<uint32_t>();
  if (flags[0] != 0) {
    throw std::invalid_argument(std::string("mccl ") + what +
        " row ID is outside the relation");
  }
  if (flags[1] != 0) {
    throw std::invalid_argument(std::string("mccl ") + what +
        " value is outside the u32 domain");
  }
}

// Install only after successful completion; errors leave *out unchanged.
void install(mlx_array* out, mx::array result) {
  if (out->ctx) *static_cast<mx::array*>(out->ctx) = std::move(result);
  else out->ctx = new mx::array(std::move(result));
}

// Evaluated distinct-tuple graph {tuples, count, invalid} → published prefix.
mx::array publish_distinct(std::vector<mx::array>& graph, int column_count, const char* what) {
  mx::eval(graph);
  check_invalid(graph[2], what);
  const uint32_t distinct = graph[1].data<uint32_t>()[0];
  return radix_prefix(graph[0], {static_cast<int>(distinct), column_count});
}

} // namespace

extern "C" int mccl_build_relation_radix_u32(
    mccl_radix_index* out, mlx_array relation, int key_column,
    int payload_column, mlx_array selection, mlx_stream stream) {
  return capture_error([&] {
    validate_radix_outputs(out, {relation, selection});
    const auto s = validate_stream(stream);
    auto sorted = mccl::native::make_relation_index_graph(
        mlx_array_get_(relation), key_column, payload_column,
        selection.ctx ? &mlx_array_get_(selection) : nullptr, s, false);
    const auto capacity = static_cast<size_t>(sorted[0].shape(0));
    auto graph = make_radix_graph({sorted[0], sorted[2]}, false, capacity, capacity, s);
    publish_radix(out, graph);
  });
}

extern "C" int mccl_merge_radix_u32(
    mccl_radix_index* out, const mccl_radix_index* older,
    const mccl_radix_index* newer, mlx_stream stream) {
  return capture_error([&] {
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    validate_completed_radix(older, device);
    validate_completed_radix(newer, device);
    if (out == older || out == newer) {
      throw std::invalid_argument("mccl radix merge output must not alias its inputs");
    }
    auto handles = radix_inputs(older);
    const auto right = radix_inputs(newer);
    handles.insert(handles.end(), right.begin(), right.end());
    validate_radix_outputs(out, handles);
    const size_t rows = older->rows + newer->rows;
    const size_t capacity = older->key_count + newer->key_count;
    auto graph = make_radix_graph(
        {mlx_array_get_(older->keys), mlx_array_get_(older->offsets),
         mlx_array_get_(older->items), mlx_array_get_(newer->keys),
         mlx_array_get_(newer->offsets), mlx_array_get_(newer->items)},
        true, rows, capacity, s);
    publish_radix(out, graph);
  });
}

extern "C" int mccl_radix_join_u32(
    mlx_array* pairs, const mccl_radix_index* index, mlx_array probes,
    mlx_stream stream) {
  return capture_error([&] {
    if (!pairs) throw std::invalid_argument("mccl radix join output must not be null");
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    validate_completed_radix(index, device);
    if (!probes.ctx) throw std::invalid_argument("mccl radix join probes are missing");
    if (pairs->ctx) {
      auto inputs = radix_inputs(index);
      inputs.push_back(probes);
      for (auto input : inputs) {
        if (pairs->ctx == input.ctx) {
          throw std::invalid_argument("mccl radix join output must not alias its inputs");
        }
      }
    }
    uint64_t total = 0;
    auto counted = count_join(index, mlx_array_get_(probes), s, device, total);
    std::vector<mx::array> expanded{mx::array(
        mx::Shape{static_cast<int>(total), 2}, mx::uint32,
        std::make_shared<RadixJoinExpandOperation>(s, total, counted[0].size()),
        {counted[1], counted[0], mlx_array_get_(index->offsets),
         mlx_array_get_(index->items)})};
    mx::eval(expanded);
    install(pairs, std::move(expanded[0]));
  });
}

extern "C" int mccl_radix_semijoin_u32(
    mlx_array* out, mlx_array rows, mlx_array relation, int key_column,
    int item_column, const mccl_radix_index* index, mlx_stream stream) {
  return capture_error([&] {
    if (!out) throw std::invalid_argument("mccl radix semijoin output must not be null");
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    validate_completed_radix(index, device);
    if (!rows.ctx || !relation.ctx) {
      throw std::invalid_argument("mccl radix semijoin inputs are missing");
    }
    if (out->ctx) {
      auto inputs = radix_inputs(index);
      inputs.push_back(rows);
      inputs.push_back(relation);
      for (auto input : inputs) {
        if (out->ctx == input.ctx) {
          throw std::invalid_argument("mccl radix semijoin output must not alias its inputs");
        }
      }
    }
    const auto& ids = mlx_array_get_(rows);
    const auto& source = mlx_array_get_(relation);
    const auto input = validate_rows_input(ids, source, "radix semijoin");
    const auto stage = semijoin_stage(source, key_column, item_column, index);
    const size_t n = ids.shape(0);
    validate_capacity(n + 1, device);
    auto graph = mx::array::make_arrays(
        {{static_cast<int>(n)}, {1}, {2}}, {mx::uint32, mx::uint32, mx::uint32},
        std::make_shared<RadixSemiJoinOperation>(s, input.stride, input.offset, stage),
        {ids, source, mlx_array_get_(index->keys), mlx_array_get_(index->offsets),
         mlx_array_get_(index->items)});
    mx::eval(graph);
    check_invalid(graph[2], "radix semijoin");
    const uint32_t kept = graph[1].data<uint32_t>()[0];
    install(out, radix_prefix(graph[0], {static_cast<int>(kept)}));
  });
}

extern "C" int mccl_distinct_rows_i64(
    mlx_array* out, mlx_array rows, mlx_array relation, const int* columns,
    int column_count, mlx_stream stream) {
  return capture_error([&] {
    if (!out) throw std::invalid_argument("mccl distinct rows output must not be null");
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    if (!rows.ctx || !relation.ctx) {
      throw std::invalid_argument("mccl distinct rows inputs are missing");
    }
    if (out->ctx && (out->ctx == rows.ctx || out->ctx == relation.ctx)) {
      throw std::invalid_argument("mccl distinct rows output must not alias its inputs");
    }
    const auto& ids = mlx_array_get_(rows);
    const auto& source = mlx_array_get_(relation);
    const auto input = validate_rows_input(ids, source, "distinct rows");
    const size_t n = ids.shape(0);
    const auto tuple = distinct_tuple(source, columns, column_count, n, device);
    auto graph = mx::array::make_arrays(
        {{static_cast<int>(n), column_count}, {1}, {2}},
        {mx::int64, mx::uint32, mx::uint32},
        std::make_shared<DistinctRowsOperation>(s, input.stride, input.offset, tuple),
        {ids, source});
    install(out, publish_distinct(graph, column_count, "distinct rows"));
  });
}

extern "C" int mccl_radix_join_semijoin_distinct_i64(
    mlx_array* out, const mccl_radix_index* join_index, mlx_array probes,
    mlx_array relation, const mccl_semijoin_step* steps, int step_count,
    const int* columns, int column_count, mlx_stream stream) {
  return capture_error([&] {
    if (!out) throw std::invalid_argument("mccl join chain output must not be null");
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    validate_completed_radix(join_index, device);
    if (!probes.ctx || !relation.ctx) {
      throw std::invalid_argument("mccl join chain inputs are missing");
    }
    if (step_count < 0 || (step_count > 0 && !steps)) {
      throw std::invalid_argument("mccl join chain steps are missing");
    }
    std::vector<mlx_array> handles = radix_inputs(join_index);
    handles.push_back(probes);
    handles.push_back(relation);
    const auto& source = mlx_array_get_(relation);
    validate_relation_i64(source, "join chain");
    std::vector<SemiJoinStage> stages;
    std::vector<mx::array> inputs;
    for (int k = 0; k < step_count; ++k) {
      validate_completed_radix(steps[k].index, device);
      stages.push_back(
          semijoin_stage(source, steps[k].key_column, steps[k].item_column, steps[k].index));
      const auto step = radix_inputs(steps[k].index);
      handles.insert(handles.end(), step.begin(), step.end());
    }
    for (auto handle : handles) {
      if (out->ctx && out->ctx == handle.ctx) {
        throw std::invalid_argument("mccl join chain output must not alias its inputs");
      }
    }
    uint64_t total = 0;
    auto counted = count_join(join_index, mlx_array_get_(probes), s, device, total);
    const auto tuple = distinct_tuple(source, columns, column_count, total, device);
    // A kept row's key column holds a key of every step run keyed on it, so
    // that run's max_key bounds the column; the field needs only its bits.
    DistinctWidths widths = kFullWidths;
    for (int c = 0; c < column_count; ++c) {
      uint64_t bound = UINT32_MAX;
      for (int k = 0; k < step_count; ++k) {
        if (steps[k].key_column == columns[c] && steps[k].index->rows != 0) {
          bound = std::min(bound, steps[k].index->max_key);
        }
      }
      widths[c] = bound == 0 ? 0 : 64 - static_cast<uint32_t>(__builtin_clzll(bound));
    }
    validate_capacity(total + 1, device);
    if (total == 0) {
      // No pairs: the published empty prefix needs no GPU work.
      mx::array empty({0, column_count}, mx::int64, nullptr, {});
      allocate_storage(empty);
      empty.set_status(mx::array::Status::available);
      install(out, std::move(empty));
      return;
    }
    inputs = {counted[1], counted[0], mlx_array_get_(join_index->offsets),
              mlx_array_get_(join_index->items), source};
    for (int k = 0; k < step_count; ++k) {
      inputs.push_back(mlx_array_get_(steps[k].index->keys));
      inputs.push_back(mlx_array_get_(steps[k].index->offsets));
      inputs.push_back(mlx_array_get_(steps[k].index->items));
    }
    auto graph = mx::array::make_arrays(
        {{static_cast<int>(total), column_count}, {1}, {2}},
        {mx::int64, mx::uint32, mx::uint32},
        std::make_shared<JoinChainOperation>(
            s, total, counted[0].size(), std::move(stages), tuple, widths),
        std::move(inputs));
    install(out, publish_distinct(graph, column_count, "join chain"));
  });
}

namespace {

// CUDA DeviceRadixRun layout (GPU-RadixRun): keys of `lanes` u64 words.
constexpr uint32_t kMaxWordLanes = 8;      // Must match words.metal.
constexpr uint32_t kMaxWordRadixBits = 30; // Directory entries index a uint grid.
constexpr uint32_t kMaxWordItems = 12;     // CUDA item tuple arity.
// Merge-path slots per thread; measured on M3 Max, 16M + 10% random keys:
// 2-4 fastest (~7.8 ms), 16: 11.6 ms, 32: 12.9 ms (strided tiles coalesce poorly).
constexpr uint32_t kWordMergeTile = 4;

struct WordRunShape {
  uint32_t lanes, ms_first, active_bits, radix_bits, offset_bytes;
};

mx::Dtype offset_dtype(uint32_t offset_bytes) {
  return offset_bytes == 4 ? mx::uint32 : mx::uint64;
}

size_t word_directory_entries(uint32_t radix_bits) {
  return radix_bits == 0 ? 0 : (size_t{1} << radix_bits) + 1;
}

// Directory over the published distinct keys outputs[0] with meta outputs[4].
void encode_word_directory(
    std::vector<mx::array>& outputs, const WordRunShape& shape,
    mx::metal::Device& device, mx::metal::CommandEncoder& encoder) {
  if (shape.radix_bits == 0) return;
  const uint32_t args[] = {shape.lanes, shape.ms_first, shape.radix_bits};
  encoder.set_compute_pipeline_state(index_kernel(device,
      shape.offset_bytes == 8 ? "words_directory_o64" : "words_directory_o32"));
  encoder.set_input_array(outputs[0], 0);
  encoder.set_input_array(outputs[4], 1);
  encoder.set_output_array(outputs[3], 2);
  encoder.set_bytes(args, 3, 3);
  dispatch(encoder, ceil_div(word_directory_entries(shape.radix_bits), kThreads));
}

// inputs: keys uint64 [n, lanes], then item columns [n]. outputs: distinct
// keys [n, lanes] (prefix key_count), offsets [n + 1] (prefix key_count + 1),
// stable order uint32 [n], directory [entries], meta uint64 [2] =
// {key_count, n}, then each item column gathered through the stable order.
class WordRunOperation final : public mx::Primitive {
 public:
  WordRunOperation(mx::Stream stream, WordRunShape shape)
      : mx::Primitive(stream), shape_(shape) {}
  const char* name() const override { return "McclWordRun"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("mccl word runs require a GPU stream");
  }
  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& device = validate_index_device(stream());
    require_contiguous(inputs, "word run");
    for (auto& output : outputs) allocate_storage(output);
    auto& encoder = mx::metal::get_command_encoder(stream());
    const auto& keys = inputs[0];
    const auto n = static_cast<uint32_t>(keys.shape(0));
    const bool wide = shape_.offset_bytes == 8;
    auto gathered = allocate_leaf({static_cast<int>(n), 2}, device);
    auto sorted = allocate_leaf({static_cast<int>(n), 2}, device);
    encoder.add_temporary(gathered);
    encoder.add_temporary(sorted);
    // LSD over the active 32-bit slices: each pass gathers one slice through
    // the previous stable order and sorts it stably, so ties keep source order.
    const uint32_t slices = (shape_.active_bits + 31) / 32;
    auto* gather = index_kernel(device, "words_gather_slice");
    for (uint32_t slice = 0; slice == 0 || slice < slices; ++slice) {
      const uint32_t args[] = {n, shape_.lanes, shape_.ms_first, slice, slice == 0};
      encoder.set_compute_pipeline_state(gather);
      encoder.set_input_array(keys, 0);
      encoder.set_input_array(sorted, 1);
      encoder.set_output_array(gathered, 2);
      encoder.set_bytes(args, 5, 3);
      dispatch(encoder, ceil_div(n, kThreads));
      if (slices == 0) {
        // No active bits: one key, identity order.
        sorted = gathered;
        break;
      }
      const uint32_t bits = std::min<uint32_t>(32, shape_.active_bits - slice * 32);
      encode_sort(gathered, sorted, device, encoder, nullptr, (bits + 7) / 8);
    }
    auto prefix = flag_prefix(n, device, encoder);
    const uint32_t group_args[] = {n, shape_.lanes};
    encoder.set_compute_pipeline_state(index_kernel(device, "words_group_flags"));
    encoder.set_input_array(keys, 0);
    encoder.set_input_array(sorted, 1);
    encoder.set_output_array(prefix, 2);
    encoder.set_bytes(group_args, 2, 3);
    dispatch(encoder, ceil_div(n + 1, kThreads));
    scan_flags(prefix, device, encoder);
    encoder.set_compute_pipeline_state(index_kernel(device,
        wide ? "words_group_scatter_o64" : "words_group_scatter_o32"));
    encoder.set_input_array(keys, 0);
    encoder.set_input_array(sorted, 1);
    encoder.set_input_array(prefix, 2);
    encoder.set_output_array(outputs[0], 3);
    encoder.set_output_array(outputs[1], 4);
    encoder.set_output_array(outputs[2], 5);
    encoder.set_output_array(outputs[4], 6);
    encoder.set_bytes(group_args, 2, 7);
    dispatch(encoder, ceil_div(n, kThreads));
    encode_word_directory(outputs, shape_, device, encoder);
    for (size_t column = 1; column < inputs.size(); ++column) {
      encoder.set_compute_pipeline_state(index_kernel(device,
          inputs[column].itemsize() == 8 ? "words_gather_b8" : "words_gather_b4"));
      encoder.set_input_array(inputs[column], 0);
      encoder.set_input_array(outputs[2], 1);
      encoder.set_output_array(outputs[4 + column], 2);
      encoder.set_bytes(n, 3);
      dispatch(encoder, ceil_div(n, kThreads));
    }
  }
 private:
  WordRunShape shape_;
};

// inputs: older {keys, offsets, order}, newer {keys, offsets, order}, then
// the older and the newer item columns. outputs as WordRunOperation, with
// key and offset capacity slots = older keys + newer keys.
class WordMergeOperation final : public mx::Primitive {
 public:
  WordMergeOperation(mx::Stream stream, WordRunShape shape)
      : mx::Primitive(stream), shape_(shape) {}
  const char* name() const override { return "McclWordMerge"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("mccl word runs require a GPU stream");
  }
  void eval_gpu(
      const std::vector<mx::array>& inputs,
      std::vector<mx::array>& outputs) override {
    auto& device = validate_index_device(stream());
    require_contiguous(inputs, "word run merge");
    for (auto& output : outputs) allocate_storage(output);
    auto& encoder = mx::metal::get_command_encoder(stream());
    const bool wide = shape_.offset_bytes == 8;
    const auto older_keys = static_cast<uint32_t>(inputs[0].shape(0));
    const auto newer_keys = static_cast<uint32_t>(inputs[3].shape(0));
    const auto older_rows = static_cast<uint32_t>(inputs[2].shape(0));
    const auto newer_rows = static_cast<uint32_t>(inputs[5].shape(0));
    const uint32_t slots = older_keys + newer_keys, rows = older_rows + newer_rows;
    auto tags = allocate_leaf({static_cast<int>(slots)}, device);
    auto positions = allocate_leaf(
        {static_cast<int>(slots + 1)}, device, offset_dtype(shape_.offset_bytes));
    auto sources = allocate_leaf({static_cast<int>(rows)}, device);
    encoder.add_temporary(tags);
    encoder.add_temporary(positions);
    encoder.add_temporary(sources);
    auto prefix = flag_prefix(slots, device, encoder);
    const uint32_t slot_args[] = {
        older_keys, newer_keys, shape_.lanes, shape_.ms_first, kWordMergeTile,
        older_rows, newer_rows};
    encoder.set_compute_pipeline_state(index_kernel(device,
        wide ? "words_merge_slots_o64" : "words_merge_slots_o32"));
    encoder.set_input_array(inputs[0], 0);
    encoder.set_input_array(inputs[1], 1);
    encoder.set_input_array(inputs[3], 2);
    encoder.set_input_array(inputs[4], 3);
    encoder.set_output_array(tags, 4);
    encoder.set_output_array(positions, 5);
    encoder.set_output_array(prefix, 6);
    encoder.set_bytes(slot_args, 7, 7);
    dispatch(encoder, ceil_div(ceil_div(slots, kWordMergeTile), kThreads));
    scan_flags(prefix, device, encoder);
    const uint32_t scatter_args[] = {slots, shape_.lanes};
    encoder.set_compute_pipeline_state(index_kernel(device,
        wide ? "words_merge_scatter_o64" : "words_merge_scatter_o32"));
    encoder.set_input_array(inputs[0], 0);
    encoder.set_input_array(inputs[3], 1);
    encoder.set_input_array(tags, 2);
    encoder.set_input_array(positions, 3);
    encoder.set_input_array(prefix, 4);
    encoder.set_output_array(outputs[0], 5);
    encoder.set_output_array(outputs[1], 6);
    encoder.set_output_array(outputs[4], 7);
    encoder.set_bytes(scatter_args, 2, 8);
    dispatch(encoder, ceil_div(slots, kThreads));
    const uint32_t source_args[] = {slots, rows, older_rows};
    encoder.set_compute_pipeline_state(index_kernel(device,
        wide ? "words_merge_sources_o64" : "words_merge_sources_o32"));
    encoder.set_input_array(inputs[1], 0);
    encoder.set_input_array(inputs[4], 1);
    encoder.set_input_array(tags, 2);
    encoder.set_input_array(positions, 3);
    encoder.set_output_array(sources, 4);
    encoder.set_bytes(source_args, 3, 5);
    dispatch(encoder, ceil_div(rows, kThreads));
    // Column 0 is the stable order: newer rows rebase after the older rows.
    const size_t columns = (inputs.size() - 6) / 2;
    for (size_t column = 0; column <= columns; ++column) {
      const auto& older = column == 0 ? inputs[2] : inputs[5 + column];
      const auto& newer = column == 0 ? inputs[5] : inputs[5 + columns + column];
      const uint32_t args[] = {rows, older_rows, column == 0 ? older_rows : 0};
      encoder.set_compute_pipeline_state(index_kernel(device,
          older.itemsize() == 8 ? "words_merge_b8" : "words_merge_b4"));
      encoder.set_input_array(older, 0);
      encoder.set_input_array(newer, 1);
      encoder.set_input_array(sources, 2);
      encoder.set_output_array(column == 0 ? outputs[2] : outputs[4 + column], 3);
      encoder.set_bytes(args, 3, 4);
      dispatch(encoder, ceil_div(rows, kThreads));
    }
    encode_word_directory(outputs, shape_, device, encoder);
  }
 private:
  WordRunShape shape_;
};

void validate_word_params(
    uint32_t lanes, uint32_t active_bits, uint32_t radix_bits, uint32_t offset_bytes) {
  if (lanes == 0 || lanes > kMaxWordLanes) {
    throw std::invalid_argument("mccl word run lanes must be 1..8");
  }
  if (active_bits > lanes * 64) {
    throw std::invalid_argument("mccl word run active bits exceed the key word");
  }
  if (radix_bits > kMaxWordRadixBits) {
    throw std::invalid_argument("mccl word run directory bits must be at most 30");
  }
  if (offset_bytes != 4 && offset_bytes != 8) {
    throw std::invalid_argument("mccl word run offsets must be 4 or 8 bytes");
  }
}

// Run outputs then item outputs: non-null, pairwise distinct, none an input.
std::vector<mlx_array*> word_run_handles(
    mccl_word_run* run, mlx_array* items, uint32_t item_count,
    const std::vector<mlx_array>& inputs) {
  if (item_count > kMaxWordItems) {
    throw std::invalid_argument("mccl word run item columns must be at most 12");
  }
  if (item_count != 0 && !items) {
    throw std::invalid_argument("mccl word run item outputs must not be null");
  }
  std::vector<mlx_array*> handles{&run->keys, &run->offsets, &run->order, &run->directory};
  for (uint32_t column = 0; column < item_count; ++column) handles.push_back(items + column);
  for (size_t i = 0; i < handles.size(); ++i) {
    if (!handles[i]->ctx) continue;
    for (const auto& input : inputs) {
      if (handles[i]->ctx == input.ctx) {
        throw std::invalid_argument("mccl word run outputs must not alias the inputs");
      }
    }
    for (size_t j = 0; j < i; ++j) {
      if (handles[i]->ctx == handles[j]->ctx) {
        throw std::invalid_argument("mccl word run output handles must be distinct");
      }
    }
  }
  return handles;
}

const mx::array& word_item(mlx_array item, size_t rows) {
  if (!item.ctx) throw std::invalid_argument("mccl word run item column is missing");
  const auto& column = mlx_array_get_(item);
  const auto dtype = column.dtype();
  if (dtype != mx::uint32 && dtype != mx::int32 && dtype != mx::uint64 && dtype != mx::int64) {
    throw std::invalid_argument("mccl word run item columns must be 4- or 8-byte integers");
  }
  if (column.ndim() != 1 || static_cast<size_t>(column.shape(0)) != rows) {
    throw std::invalid_argument("mccl word run item columns must be [rows]");
  }
  return column;
}

// A merge input as published by a build or merge: arrays match metadata.
std::array<mx::array, 3> word_run_arrays(const mccl_word_run& run) {
  if (!run.keys.ctx || !run.offsets.ctx || !run.order.ctx || !run.directory.ctx) {
    throw std::invalid_argument("mccl word run merge input arrays are missing");
  }
  if (run.key_count == 0 || run.key_count > run.rows || run.rows > kMaxPairRows) {
    throw std::invalid_argument("mccl word run merge input has invalid counts");
  }
  const auto offsets = offset_dtype(run.offset_bytes);
  const auto keys = static_cast<int>(run.key_count);
  const auto expect = [](const mx::array& array, mx::Dtype dtype, const mx::Shape& shape) {
    if (array.dtype() != dtype || array.shape() != shape) {
      throw std::invalid_argument("mccl word run merge input arrays do not match their metadata");
    }
    return array;
  };
  expect(mlx_array_get_(run.directory), offsets,
      {static_cast<int>(word_directory_entries(run.radix_bits))});
  return {
      expect(mlx_array_get_(run.keys), mx::uint64, {keys, static_cast<int>(run.lanes)}),
      expect(mlx_array_get_(run.offsets), offsets, {keys + 1}),
      expect(mlx_array_get_(run.order), mx::uint32, {static_cast<int>(run.rows)})};
}

// Evaluates {keys, offsets, order, directory, meta, items...} with one
// count readback and installs primitive-free prefixes; *out changes only on
// success.
void publish_word_run(
    mccl_word_run* out, const std::vector<mlx_array*>& handles,
    std::vector<mx::array>& graph, size_t rows, const WordRunShape& shape) {
  mx::eval(graph);
  const auto* meta = graph[4].data<uint64_t>();
  if (meta[1] != rows || meta[0] == 0 ||
      meta[0] > static_cast<uint64_t>(graph[0].shape(0))) {
    throw std::runtime_error("mccl GPU word run metadata exceeds output capacity");
  }
  const auto key_count = static_cast<int>(meta[0]);
  const auto n = static_cast<int>(rows);
  std::vector<mx::array> result{
      radix_prefix(graph[0], {key_count, static_cast<int>(shape.lanes)}),
      radix_prefix(graph[1], {key_count + 1}),
      radix_prefix(graph[2], {n}),
      radix_prefix(graph[3], graph[3].shape())};
  for (size_t column = 5; column < graph.size(); ++column) {
    result.push_back(radix_prefix(graph[column], {n}));
  }
  for (size_t i = 0; i < handles.size(); ++i) install(handles[i], std::move(result[i]));
  out->rows = rows;
  out->key_count = meta[0];
  out->lanes = shape.lanes;
  out->ms_first = shape.ms_first;
  out->active_bits = shape.active_bits;
  out->radix_bits = shape.radix_bits;
  out->offset_bytes = shape.offset_bytes;
}

} // namespace

extern "C" int mccl_build_word_run_items(
    mccl_word_run* out, mlx_array* items_out, mlx_array keys, const mlx_array* items,
    uint32_t item_count, uint32_t lanes, int ms_first, uint32_t active_bits,
    uint32_t radix_bits, uint32_t offset_bytes, mlx_stream stream) {
  return capture_error([&] {
    if (!out) throw std::invalid_argument("mccl word run output must not be null");
    if (!keys.ctx) throw std::invalid_argument("mccl word run keys are missing");
    if (item_count != 0 && !items) {
      throw std::invalid_argument("mccl word run item columns must not be null");
    }
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    validate_word_params(lanes, active_bits, radix_bits, offset_bytes);
    std::vector<mlx_array> sources{keys};
    for (uint32_t column = 0; column < std::min(item_count, kMaxWordItems); ++column) {
      sources.push_back(items[column]);
    }
    const auto handles = word_run_handles(out, items_out, item_count, sources);
    const auto& source = mlx_array_get_(keys);
    if (source.ndim() != 2 || source.dtype() != mx::uint64 ||
        static_cast<uint32_t>(source.shape(1)) != lanes) {
      throw std::invalid_argument("mccl word run keys must be uint64 [rows, lanes]");
    }
    const auto rows = static_cast<size_t>(source.shape(0));
    if (rows == 0) {
      throw std::invalid_argument("mccl word run needs rows; an empty build publishes nothing");
    }
    if (rows > kMaxPairRows) {
      throw std::invalid_argument("mccl word run rows exceed 1<<30");
    }
    validate_capacity(rows * lanes, device, sizeof(uint64_t));
    validate_capacity(rows + 1, device, offset_bytes);
    const auto entries = word_directory_entries(radix_bits);
    validate_capacity(entries, device, offset_bytes);
    const auto n = static_cast<int>(rows);
    std::vector<mx::Shape> shapes{
        {n, static_cast<int>(lanes)}, {n + 1}, {n}, {static_cast<int>(entries)}, {2}};
    std::vector<mx::Dtype> dtypes{
        mx::uint64, offset_dtype(offset_bytes), mx::uint32, offset_dtype(offset_bytes), mx::uint64};
    std::vector<mx::array> inputs{mx::contiguous(source, false, s)};
    for (uint32_t column = 0; column < item_count; ++column) {
      const auto& item = word_item(items[column], rows);
      shapes.push_back({n});
      dtypes.push_back(item.dtype());
      inputs.push_back(mx::contiguous(item, false, s));
    }
    const WordRunShape shape{lanes, uint32_t(ms_first != 0), active_bits, radix_bits, offset_bytes};
    auto graph = mx::array::make_arrays(
        std::move(shapes), std::move(dtypes),
        std::make_shared<WordRunOperation>(s, shape), std::move(inputs));
    publish_word_run(out, handles, graph, rows, shape);
  });
}

extern "C" int mccl_build_word_run(
    mccl_word_run* out, mlx_array keys, uint32_t lanes, int ms_first,
    uint32_t active_bits, uint32_t radix_bits, uint32_t offset_bytes,
    mlx_stream stream) {
  return mccl_build_word_run_items(
      out, nullptr, keys, nullptr, 0, lanes, ms_first, active_bits, radix_bits,
      offset_bytes, stream);
}

extern "C" int mccl_merge_word_runs(
    mccl_word_run* out, mlx_array* items_out, const mccl_word_run* older,
    const mlx_array* older_items, const mccl_word_run* newer,
    const mlx_array* newer_items, uint32_t item_count, mlx_stream stream) {
  return capture_error([&] {
    if (!out) throw std::invalid_argument("mccl word run output must not be null");
    if (!older || !newer) throw std::invalid_argument("mccl word run merge inputs must not be null");
    if (item_count != 0 && (!older_items || !newer_items)) {
      throw std::invalid_argument("mccl word run item columns must not be null");
    }
    const auto s = validate_stream(stream);
    auto& device = validate_index_device(s);
    if (older->lanes != newer->lanes || older->ms_first != newer->ms_first ||
        older->active_bits != newer->active_bits || older->radix_bits != newer->radix_bits ||
        older->offset_bytes != newer->offset_bytes) {
      throw std::invalid_argument("mccl word run merge inputs must have identical parameters");
    }
    const auto lanes = older->lanes, offset_bytes = older->offset_bytes;
    validate_word_params(lanes, older->active_bits, older->radix_bits, offset_bytes);
    const auto first = word_run_arrays(*older), second = word_run_arrays(*newer);
    if (older->rows > kMaxPairRows - newer->rows) {
      throw std::invalid_argument("mccl word run merged rows exceed 1<<30");
    }
    std::vector<mlx_array> sources{
        older->keys, older->offsets, older->order, older->directory,
        newer->keys, newer->offsets, newer->order, newer->directory};
    for (uint32_t column = 0; column < std::min(item_count, kMaxWordItems); ++column) {
      sources.push_back(older_items[column]);
      sources.push_back(newer_items[column]);
    }
    const auto handles = word_run_handles(out, items_out, item_count, sources);
    const auto rows = older->rows + newer->rows;
    const auto slots = older->key_count + newer->key_count;
    validate_capacity(slots * lanes, device, sizeof(uint64_t));
    validate_capacity(slots + 1, device, offset_bytes);
    const auto entries = word_directory_entries(older->radix_bits);
    validate_capacity(entries, device, offset_bytes);
    const auto n = static_cast<int>(rows);
    std::vector<mx::Shape> shapes{
        {static_cast<int>(slots), static_cast<int>(lanes)}, {static_cast<int>(slots + 1)},
        {n}, {static_cast<int>(entries)}, {2}};
    std::vector<mx::Dtype> dtypes{
        mx::uint64, offset_dtype(offset_bytes), mx::uint32, offset_dtype(offset_bytes), mx::uint64};
    std::vector<mx::array> inputs{first[0], first[1], first[2], second[0], second[1], second[2]};
    for (uint32_t column = 0; column < item_count; ++column) {
      const auto& item = word_item(older_items[column], older->rows);
      if (word_item(newer_items[column], newer->rows).dtype() != item.dtype()) {
        throw std::invalid_argument("mccl word run merge item columns must have identical types");
      }
      shapes.push_back({n});
      dtypes.push_back(item.dtype());
      inputs.push_back(mx::contiguous(item, false, s));
    }
    for (uint32_t column = 0; column < item_count; ++column) {
      inputs.push_back(mx::contiguous(mlx_array_get_(newer_items[column]), false, s));
    }
    const WordRunShape shape{
        lanes, older->ms_first, older->active_bits, older->radix_bits, offset_bytes};
    auto graph = mx::array::make_arrays(
        std::move(shapes), std::move(dtypes),
        std::make_shared<WordMergeOperation>(s, shape), std::move(inputs));
    publish_word_run(out, handles, graph, rows, shape);
  });
}
