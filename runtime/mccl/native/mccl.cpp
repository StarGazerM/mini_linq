#include "mccl.h"
#include "relation_index.h"

#include <array>
#include <climits>
#include <cstdint>
#include <cstdio>
#include <dlfcn.h>
#include <exception>
#include <filesystem>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#include "mlx/allocator.h"
#include "mlx/backend/metal/device.h"
#include "mlx/c/private/array.h"
#include "mlx/c/private/stream.h"
#include "mlx/ops.h"
#include "mlx/primitives.h"

namespace mx = mlx::core;

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
    mx::metal::CommandEncoder& encoder) {
  auto* scan = index_kernel(device, "sort_scan_blocks");
  auto* add = index_kernel(device, "sort_add_block_offsets");
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

void encode_sort(
    const mx::array& input, mx::array& out,
    mx::metal::Device& device, mx::metal::CommandEncoder& encoder,
    const mx::array* metadata = nullptr) {
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
  for (uint32_t pass = 0; pass < 4; ++pass) {
    const auto& source = pass == 0 ? input : (pass % 2 == 0 ? out : scratch);
    auto& destination = pass % 2 == 0 ? scratch : out;
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
