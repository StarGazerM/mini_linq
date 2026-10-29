// Diagnostic only: observe the production shared-relation multi-output graph's
// actual command buffers. Source ingestion happens once per size, never per run.
#include "mccl.h"
#include "relation_index.h"
#include "mlx/backend/metal/device.h"
#include "mlx/c/private/array.h"
#include "mlx/c/private/stream.h"
#include "mlx/c/error.h"
#include "mlx/c/memory.h"
#include "mlx/primitives.h"
#include <algorithm>
#include <chrono>
#include <cmath>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <memory>
#include <mutex>
#include <sstream>
#include <stdexcept>
#include <string>
#include <sys/resource.h>
#include <thread>
#include <vector>

struct rusage usage() {
  struct rusage value{};
  if (getrusage(RUSAGE_SELF, &value)) throw std::runtime_error("getrusage failed");
  return value;
}
double time_ms(timeval value) {
  return value.tv_sec * 1000.0 + value.tv_usec / 1000.0;
}

namespace mx = mlx::core;
using Clock = std::chrono::steady_clock;
double ms(Clock::time_point begin) {
  return std::chrono::duration<double, std::milli>(Clock::now() - begin).count();
}
void require(int status) {
  if (status) throw std::runtime_error(mccl_last_error());
}
struct ArrayHandle {
  mlx_array raw = mlx_array_new();
  ~ArrayHandle() { mlx_array_free(raw); }
};
struct Row { uint32_t key, value; };
struct Trace {
  std::mutex mutex;
  std::vector<NS::SharedPtr<MTL::CommandBuffer>> buffers;
  double build_encode_ms = 0;
  void record(MTL::CommandBuffer* buffer, double elapsed) {
    std::lock_guard lock(mutex);
    if (std::none_of(buffers.begin(), buffers.end(), [buffer](auto& b) { return b.get() == buffer; }))
      buffers.push_back(NS::RetainPtr(buffer));
    build_encode_ms += elapsed;
  }
};
class Observed final : public mx::Primitive {
 public:
  Observed(std::shared_ptr<mx::Primitive> inner, std::shared_ptr<Trace> trace)
      : mx::Primitive(inner->stream()), inner_(std::move(inner)), trace_(std::move(trace)) {}
  const char* name() const override { return "ObservedMcclRelationIndex"; }
  void eval_cpu(const std::vector<mx::array>&, std::vector<mx::array>&) override {
    throw std::runtime_error("GPU-only diagnostic");
  }
  void eval_gpu(const std::vector<mx::array>& inputs, std::vector<mx::array>& outputs) override {
    auto* cb = mx::metal::get_command_encoder(stream()).get_command_buffer();
    const auto start = Clock::now();
    inner_->eval_gpu(inputs, outputs);
    trace_->record(cb, ms(start));
  }
 private:
  std::shared_ptr<mx::Primitive> inner_;
  std::shared_ptr<Trace> trace_;
};
std::vector<mx::array> observe(
    const std::vector<mx::array>& original, const std::shared_ptr<Trace>& trace) {
  std::vector<mx::Shape> shapes;
  std::vector<mx::Dtype> dtypes;
  for (const auto& output : original) {
    shapes.push_back(output.shape());
    dtypes.push_back(output.dtype());
  }
  // Preserve the sibling relationship: evaluating any output executes the
  // same production primitive exactly once, with all outputs present.
  return mx::array::make_arrays(std::move(shapes), dtypes,
      std::make_shared<Observed>(original[0].primitive_ptr(), trace), original[0].inputs());
}

int main(int argc, char** argv) {
  size_t old_cache = 0;
  bool restore = false;
  mlx_stream stream{nullptr};
  try {
    std::vector<size_t> sizes = {1048576, 16777216, 67108864};
    size_t runs = 5;
    size_t idle_ms = 0;
    std::string path;
    std::string cache_policy = "paired";
    for (int i = 1; i < argc; i += 2) {
      if (i + 1 == argc) throw std::invalid_argument("missing option value");
      const std::string flag = argv[i], value = argv[i + 1];
      if (flag == "--out") path = value;
      else if (flag == "--runs") runs = std::stoull(value);
      else if (flag == "--idle-ms") idle_ms = std::stoull(value);
      else if (flag == "--cache") cache_policy = value;
      else if (flag == "--sizes") {
        sizes.clear();
        std::stringstream values(value);
        for (std::string part; std::getline(values, part, ',');) sizes.push_back(std::stoull(part));
      } else throw std::invalid_argument("unknown option " + flag);
    }
    if (path.empty() || sizes.empty() || runs < 3 || runs % 2 == 0 || idle_ms > 10000 ||
        std::any_of(sizes.begin(), sizes.end(), [](size_t n) { return !n || n > (size_t{1} << 30); }))
      throw std::invalid_argument("use --out FILE, nonzero sizes <=1<<30 and odd runs >=3");
    if (cache_policy != "pooled" && cache_policy != "cleared" && cache_policy != "paired")
      throw std::invalid_argument("--cache must be pooled, cleared or paired");
    const std::vector<bool> variants = cache_policy == "pooled" ? std::vector<bool>{false} :
        cache_policy == "cleared" ? std::vector<bool>{true} : std::vector<bool>{false, true};
    std::ofstream out(path);
    if (!out) throw std::runtime_error("cannot create " + path);
    out << std::setprecision(12);
    out << "rows,source_columns,source_bytes,stage,round,cache_cleared,cache_policy,idle_ms,source_ingestion_once_ms,source_allocation_once_ms,source_population_once_ms,source_ingestion_minor_faults,source_ingestion_system_ms,build_host_reads,graph_ms,eval_and_publish_wall_ms,completion_wait_ms,build_native_encode_ms,command_buffers,gpu_command_buffer_ms,gpu_span_ms,kernel_scheduling_ms,eval_minor_faults,eval_system_ms,validation_ms\n";
    mlx_set_error_handler([](const char* message, void*) { std::cerr << message << '\n'; }, nullptr, nullptr);
    stream = mlx_default_gpu_stream_new();
    if (!stream.ctx) throw std::runtime_error("create GPU stream failed");
    if (mlx_set_cache_limit(&old_cache, *std::max_element(sizes.begin(), sizes.end()) * 32 + 65536))
      throw std::runtime_error("set cache limit failed");
    restore = true;
    for (size_t n : sizes) {
      ArrayHandle source;
      const int shape[] = {static_cast<int>(n), 1};
      const auto ingestion_before = usage();
      const auto ingestion_start = Clock::now();
      auto start = Clock::now();
      uint32_t* data = nullptr;
      require(mccl_allocate_u32(&source.raw, shape, 2, &data, stream));
      const double allocation = ms(start);
      start = Clock::now();
      for (size_t i = 0; i < n; ++i)
        data[i] = ((uint32_t(i) * 40503u) & 65535u) * 65537u;
      const double population = ms(start);
      const double ingestion = ms(ingestion_start);
      const auto ingestion_after = usage();
      // The mutable ingestion mapping is not used again. All repeated builds
      // retain this same immutable source, including across cache clearing.
      data = nullptr;
      std::cout << "ingested shared relation once: rows=" << n << " bytes=" << n * sizeof(uint32_t)
                << " wall_ms=" << ingestion << '\n';
      std::vector<Row> expected(n);
      for (size_t i = 0; i < n; ++i)
        expected[i] = {((uint32_t(i) * 40503u) & 65535u) * 65537u, uint32_t(i)};
      std::stable_sort(expected.begin(), expected.end(), [](Row a, Row b) { return a.key < b.key; });
      for (size_t round = 0; round < runs + 3; ++round) {
        for (size_t order = 0; order < variants.size(); ++order) {
          const bool cleared = variants[(round + order) % variants.size()];
          // Policy boundary excludes construction; the live source remains owned.
          if (cleared && mlx_clear_cache()) throw std::runtime_error("clear cache failed");
          if (idle_ms) std::this_thread::sleep_for(std::chrono::milliseconds(idle_ms));
          const char* stage = round == 0 ? "first" : round < 3 ? "warmup" : "sample";
          auto trace = std::make_shared<Trace>();
          start = Clock::now();
          auto graph = observe(mccl::native::make_relation_index_graph(
              mlx_array_get_(source.raw), 0, -1, nullptr, mlx_stream_get_(stream)), trace);
          const double graph_ms = ms(start);
          const auto eval_before = usage();
          start = Clock::now();
          auto result = mccl::native::finish_relation_index_graph(graph);
          const double eval = ms(start);
          const auto eval_after = usage();
          start = Clock::now();
          double gpu_ms = 0;
          double gpu_first = std::numeric_limits<double>::infinity(), gpu_last = 0;
          double scheduling_ms = 0;
          bool scheduling_available = true;
          for (auto& cb : trace->buffers) {
            cb->waitUntilCompleted();
            const double begin = cb->GPUStartTime(), end = cb->GPUEndTime();
            if (cb->status() != MTL::CommandBufferStatusCompleted || !std::isfinite(begin) ||
                !std::isfinite(end) || begin <= 0 || end < begin)
              throw std::runtime_error("invalid GPU command-buffer completion/timestamps");
            gpu_ms += (end - begin) * 1000;
            gpu_first = std::min(gpu_first, begin);
            gpu_last = std::max(gpu_last, end);
            const double schedule_begin = cb->kernelStartTime(), schedule_end = cb->kernelEndTime();
            if (std::isfinite(schedule_begin) && std::isfinite(schedule_end) &&
                schedule_begin > 0 && schedule_end >= schedule_begin)
              scheduling_ms += (schedule_end - schedule_begin) * 1000;
            else scheduling_available = false;
          }
          const double completion_wait = ms(start);
          start = Clock::now();
          if (result.count != n || result.rows.shape() != mx::Shape{static_cast<int>(n), 2} ||
              result.directory.shape() != mx::Shape{4097})
            throw std::runtime_error("published shape/count mismatch");
          const auto* rows = result.rows.data<uint32_t>();
          for (size_t i = 0; i < n; ++i)
            if (rows[2 * i] != expected[i].key || rows[2 * i + 1] != expected[i].value)
              throw std::runtime_error("complete stable row mismatch");
          const auto* offsets = result.directory.data<uint32_t>();
          size_t offset = 0;
          for (size_t p = 0; p <= 4096; ++p) {
            while (offset < n && (expected[offset].key >> 20) < p) ++offset;
            if (offsets[p] != offset) throw std::runtime_error("directory mismatch");
          }
          const auto& input = mlx_array_get_(source.raw);
          if (input.shape() != mx::Shape{static_cast<int>(n), 1})
            throw std::runtime_error("source shape changed");
          const auto* unchanged = input.data<uint32_t>();
          for (size_t i = 0; i < n; ++i)
            if (unchanged[i] != ((uint32_t(i) * 40503u) & 65535u) * 65537u)
              throw std::runtime_error("immutable source changed");
          const double validation = ms(start);
          // One-time ingestion values are context repeated on each CSV record,
          // not per-build work; validation reads are separately timed below.
          out << n << ",1," << n * sizeof(uint32_t) << ',' << stage << ',' << round << ','
              << cleared << ',' << cache_policy << ',' << idle_ms << ',' << ingestion << ','
              << allocation << ',' << population << ','
              << ingestion_after.ru_minflt - ingestion_before.ru_minflt << ','
              << time_ms(ingestion_after.ru_stime) - time_ms(ingestion_before.ru_stime)
              << ",scalar_count_status," << graph_ms << ',' << eval << ',' << completion_wait << ','
              << trace->build_encode_ms << ',' << trace->buffers.size() << ',' << gpu_ms << ','
              << (gpu_last - gpu_first) * 1000 << ',';
          if (scheduling_available) out << scheduling_ms;
          out << ',' << eval_after.ru_minflt - eval_before.ru_minflt << ','
              << time_ms(eval_after.ru_stime) - time_ms(eval_before.ru_stime) << ','
              << validation << '\n';
          out.flush();
          if (!out) throw std::runtime_error("write failed");
        }
      }
      std::cout << "validated observed shared-relation GPU builds for " << n << " rows\n";
    }
    size_t discarded;
    if (mlx_set_cache_limit(&discarded, old_cache)) throw std::runtime_error("restore cache failed");
    restore = false;
    mlx_stream_free(stream);
    std::cout << "Complete; cache restored. CSV: " << path << '\n';
  } catch (const std::exception& error) {
    std::cerr << error.what() << '\n';
    if (restore) {
      size_t discarded;
      if (mlx_set_cache_limit(&discarded, old_cache)) std::cerr << "cache restoration failed\n";
    }
    if (stream.ctx) mlx_stream_free(stream);
    return 1;
  }
}
