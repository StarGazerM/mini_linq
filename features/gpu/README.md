# Hand-written retail_h05 GPU experiment

This standalone workspace implements the de-risk experiment in
`../../doc/GPU-DESIGN.md`. It does not change MiniLinq's compiler pipeline.
All kernels are Rust compiled by cuda-oxide. The approved de-risk workaround
uses dense counting-sort builds and exact atomic-CAS dedup instead of RCCL's
device algorithms, whose merged PTX hit the compiler bug documented below.
RCCL's host-only `rccl-memory` allocator is used without a device bundle.
No CUDA C, NVRTC, OptiX or CPU join is in the measured path.

## Reproduce

Run from `features/gpu` in the `gpu` fork:

```sh
source /data/rccl/scripts/oxide-env.sh
cargo oxide build --arch sm_89 -- --release -p gpu-h05 --bin smoke
./target/release/smoke
cargo oxide build --arch sm_89 -- --release -p gpu-tiles --bin tiles-oracle
./target/release/tiles-oracle
cargo oxide build --arch sm_89 -- --release -p gpu-h05 --bin blocked-run-oracle
./target/release/blocked-run-oracle
cargo oxide build --arch sm_89 -- --release -p gpu-h05 --bin dedup-oracle
./target/release/dedup-oracle
cargo oxide build --arch sm_89 -- --release -p gpu-h05 --bin gpu-h05
./target/release/gpu-h05 --synthetic /data/tpch-gpu/synthetic
./target/release/gpu-h05 /data/tpch-gpu/synthetic both
./target/release/gpu-h05 /data/tpch-gpu/sf1 both
./target/release/gpu-h05 /data/tpch-gpu/sf10 both
python3 audit.py
```

The blocked-run oracle source stays under `blocked-run/src/bin`, but its Cargo
target belongs to the consuming `gpu-h05` package. This avoids the pinned
non-generic module loader's package-name ambiguity between a library and a
binary built from that same package; the initial arrangement produced CUDA
error 500 (`named symbol not found`) while loading the run kernels.
The input directory contains the column files described by `manifest.json`:
raw little-endian i64 keys and i32 dates/nation/region/name dictionary IDs.
`answer.tsv` is the sorted distinct three-column DuckDB answer. Without it, the
runner computes an independent CPU reference. `--synthetic DIR` creates a small,
deterministic, near-uniform dataset with dense primary keys and date boundaries,
and writes its CPU answer in the same format. The benchmark validates both the
device-deduplicated key set and the fully materialized three-column result.

One warmup and five measured runs rebuild all runs, execute the fused kernel,
and deduplicate. Inputs upload once, before timing. CUDA events delimit build,
kernel, dedup and total. Build includes pool suballocation, GPU filtering,
construction, clustering and one batched metadata readback. Dedup includes the
output-count readback, pool suballocation and exact GPU hash insertion.
Event intervals include GPU idle time caused by host synchronization and launch
boundaries. Final answer
transfers and CPU comparison are outside timing. The runner updates matching
SF/plan entries in `compare/gpu_h05.results.json`; `audit.py` fills resource fields
from actual `ptxas -v -arch=sm_89` output and saves that output verbatim.

## Implementation

- `tiles`: cross-crate `#[device]` helpers for 256-thread exclusive scan, stable
  shared-memory compaction, upper-bound owner search, shared bitonic sorting of
  up to 1024 u32 keys, and stable merge-path partition. Oracle kernels invoke
  the library across a crate boundary, including repeated offsets, empty inputs,
  partial tiles, random masks, MAX keys and every merge diagonal.
- `blocked-run`: a device sortedness test selects adjacent-difference
  construction or dense counting sort with bitmap-word prefix scans, atomic
  provenance scatter and per-bin heapsort for stable duplicates.
  Payloads are gathered in stable input-row order. Runs expose CSR offsets, unique
  keys, 2048-row fences, a dense-domain u64 bitmap and u32 popcount prefixes.
  `point` returns a half-open `(begin,end)` range.
- Plan A: dynamically scheduled 1024-row lineitem tiles (256 threads, four
  striped items each), order rank probe after date filtering, customer rank
  probe, nation direct-array/region bitmap check, supplier rank probe and
  nation equality. One atomic reservation per nonempty tile, shared packing,
  then coalesced columnar stores.
- Plan B: customer tiles restricted to ASIA; CSR expansion into date-filtered
  orders by customer, then into lineitems by order. Each stage retains begins,
  exclusive offsets and parent nation in shared memory. Output-parallel owner
  search resolves every virtual row. Nested in-tile loops consume all `M/T`
  chunks without any global frontier or cross-block stealing. Counters record
  the maximum expansion at each stage. No frontier spill is necessary for
  correctness, regardless of the observed expansion ratio.
- Both plans allocate output capacity equal to lineitem cardinality, a strict
  bound under the TPC-H PK/FK schema. Thus overflow/relaunch is unnecessary for
  this query, rather than silently dropping output. Dedup packs all three fields
  losslessly into u64 (26-bit order, 17-bit supplier, 5-bit nation); actual
  manifest-column domains are checked before upload. Zero marks an empty slot,
  so stored keys are encoding+1. AtomicCAS linear probing compares the complete
  encoded key; successful insertions count distinct triples. Capacity is at
  least four times the emitted row count, rounded to a power of two.

## Contracts and measurement caveats

The toolchain is pinned to nightly-2026-08-28 and cuda-oxide commit
`5f7c34fddadcf250f2e45d0cffe83a3d27f2bea3`. CUDA defaults come from RCCL's
configuration. `rccl-memory` is a path dependency at `/data/rccl/rccl-memory`;
it uses the same pinned cuda-oxide revision and does not introduce device PTX.
Device artifacts are workspace-local under `target/oxide-ir`.
The release profile disables LTO.

The first tile build failed MIR verification when passing the pointer returned
by `DynamicSharedArray::<u32>::get()` into a helper: the call carried address
space 3 while the helper signature carried a generic pointer. Moving helpers
into a `#[path]` module did not fix it. The supported RCCL pattern,
`DynamicSharedArray::<u8>::get_raw() as *mut u32`, fixes this type-lowering
boundary without modifying the compiler. The module fallback is not retained.
Rust also warns that `#[inline(always)]` is ignored on exported `#[device]`
functions; the device backend's documented inline behavior and resource audit
remain the relevant evidence, rather than suppressing that host warning.

Cooperative tile helpers require exactly 256 converged threads. Scan needs eight
shared u32 words and uses wrapping arithmetic; callers ensure expansion totals
fit u32. Compaction's public helper accepts four consecutive indices per thread;
the query's striped TileStore uses the same scan with its own row-ID packing.
Bitonic sort is key-only and not stable. Merge-path is stable A-before-B.
Pointer validity and shared-memory ownership are explicit caller obligations.
Rank and row positions use u32; unsupported large domains are rejected rather
than truncated. The query's direct nation array rejects negative nation keys;
region keys must fit its 64-bit bitmap. These are workload representation
contracts, not general-purpose SQL limitations.

`scan_gbps` is the logical 16-byte-per-lineitem input divided by Plan A kernel
time, compared with the 960 GB/s model; it is not a hardware DRAM counter and
includes probe/compaction work. Plan B reports null for that metric because its
access pattern is not a fact-table scan. Device memory uses CUDA Driver
`cuMemGetInfo` (the counterpart of runtime `cudaMemGetInfo`), sampled while run
construction scratch is live, after build, and after dedup against a pre-upload
baseline. It includes persistent inputs/outputs, run data, and sampled build
scratch. This is sampled device usage, not a hardware allocator high-water mark.
Tiny nation/region representations are prepared once outside timed rebuilds.

Scratch, run columns and output buffers use one `rccl-memory`
`PoolMemoryResource<CudaMemoryResource>` wrapped in `StatisticsMemoryResource`.
The fixed-capacity pool is primed from input cardinalities before warmup;
stream-ordered suballocation and return never invoke synchronous device free.
Only one-time input uploads remain on `cuda_core::DeviceBuffer`. Per-iteration
allocation counts are logical suballocations, not CUDA allocation calls, and
peak bytes count live requested bytes rather than reserved pool capacity.

Key-domain bounds are prepared once from manifest-validated input columns.
Filtered cardinalities, scan totals and run metadata remain on device; run
buffers have bounded capacity rather than requiring a downloaded unique count.
`read_stats` combines all run records into one end-of-build DtoH transfer.
The query output count is read once to size the dedup table, whose kernel reads
the device counter. Dedup status and exact-answer downloads happen after the
final timing event. No extrema or per-stage total readback remains in a build.

The histogram sets a presence bit only on the atomic count's 0→1 transition.
A coalesced warp per 64-key word computes local bin offsets, its row total,
its bitmap popcount and its maximum multiplicity. Only the two word-sized
arrays (row totals and popcounts) are globally scanned. A bin's final row
offset is its local offset plus the word's row prefix; its CSR rank is the
word's popcount prefix plus the lower bits' popcount. Row totals are necessary
in addition to popcounts because duplicate keys contribute multiple rows.
This removes the full-domain multi-level scan and the old serial directory
kernel. Oracles cover partial words and recursive word-prefix boundaries
through a 4,194,305-position sparse domain, with stable duplicate payloads.

A one-pass adjacent-key check records sortedness on device (warp ballots reduce
flag updates). Nondecreasing keys use adjacent run starts/ends to build the
bitmap, CSR offsets, unique keys and maximum multiplicity; copying the already
ordered columns yields fences directly. Histogram initialization, counting,
scatter and provenance heapsort return without work on this path. Their
device-gated launches remain, avoiding an extra sortedness readback.
SF10 takes this path for **customer and supplier in both plans, plus lineitem
in Plan B**, in warmup and all measured iterations. Filtered orders do **not**
take it: the original orders column is ordered, but the date filter's atomic
block reservations do not preserve block order; Plan B's customer-key ordering
is also unsorted. The optimization does not assume dbgen ordering.

## RCCL compiler bug and approved workaround

The initial RCCL-based blocked-run oracle built and passed its two empty-input
cases. Its first nonempty case reached RCCL's program loader and failed with
CUDA error 218 (`a PTX JIT compilation failed`). Assembling the concatenated
emitted PTX identified the actual cause:

```text
ptxas target/diagnostic-merged.ptx, line 6987; error   : Duplicate definition of variable '__shared_mem_0'
ptxas fatal   : Ptx assembly aborted due to errors
```

The initial `gpu_blocked_run.ptx` contained RCCL radix kernels monomorphized into
our library: their histogram had `.visible .shared ... __shared_mem_0[8192]`.
`rccl_cub.ptx` also declared `__shared_mem_0[8192]` for its nongeneric u32 scan.
RCCL's generic program loader merges these modules, and duplicate visible
definitions are invalid PTX. All handwritten kernels in this workspace already
use only dynamic shared memory; changing our shared allocations therefore
cannot remove this collision in dependency-generated kernels.

The compiler source confirms the prerequisite fix:
`mir-lower/src/convert/ops/memory/shared.rs` creates `__shared_mem_{counter}`
with a module-local counter, while `cuda-host/src/embedded.rs` concatenates
PTX bundles without renaming these visible globals. The compiler must namespace
generated shared globals by module, or the merger must safely namespace them.
Neither RCCL nor the cuda-oxide checkout has been modified.

The experiment was explicitly redirected to use handwritten dense counting
sort, dynamic hierarchical scans and an exact u64 CAS set. RCCL is removed
from the device dependency graph rather than silently patched, and the original
diagnostic is retained in `compare/rccl-ptx-collision.txt`. Consequently the
results measure the fused execution design, **not RCCL sort/scan performance**.

## Historical original measurements (2026-10-16)

GPU: NVIDIA RTX 6000 Ada Generation, sm_89, driver 580.178.04, 49140 MiB
reported device memory. Clocks were not locked and exclusive-GPU ownership was
not established; these are local event measurements, not a controlled hardware
bandwidth study. Upload and final downloads are excluded as described above.

| SF | Plan | Build median ms | Kernel median / min ms | Dedup median ms | Total median / min ms | Distinct answers | Sampled peak MiB |
|---:|:---:|---:|---:|---:|---:|---:|---:|
| 1 | A | 2.937024 | 0.088064 / 0.088032 | 0.315744 | 3.338592 / 3.326016 | 7,242 | 360 |
| 1 | B | 4.932288 | 0.034816 / 0.033920 | 0.320704 | 5.288992 / 5.260768 | 7,242 | 490 |
| 10 | A | 12.337152 | 0.905216 / 0.903424 | 1.375360 | 14.613600 / 14.597376 | 72,983 | 3,526 |
| 10 | B | 22.318176 | 0.302240 / 0.300032 | 0.932320 | 23.552608 / 23.508928 | 72,983 | 4,820 |

Both plans matched the complete DuckDB answer set at SF1 **and SF10**, in the
warmup and all five measured runs. Synthetic input produced 579 exact triples
for both plans. The table and discussion below describe the original allocator
and full-domain scan; current samples are in `compare/gpu_h05.results.json`.

The SF10 Plan A stream is below the 10 ms design gate. Plan B is 0.334 times
Plan A's stream time and 1.612 times its build+stream+dedup time. Plan B has no
global frontier: its descriptor/working storage stays at 33,808 bytes per block.
The sampled global-memory increase is for input/output buffers, indexes and
counting-sort scratch, not a fanout-sized intermediate.

Maximum virtual expansion totals were:

| Input | Customer→orders M | Orders→lineitem M | Maximum M/1024 |
|---|---:|---:|---:|
| synthetic | 1,192 | 6,146 | 6.001953 |
| SF1 | 413 | 1,660 | 1.621094 |
| SF10 | 407 | 1,643 | 1.604492 |

All were consumed by in-tile loops; no frontier spill was needed. Cross-block
stealing remains deliberately outside this experiment.

SF10 Plan A's nominal 16-byte-per-lineitem rate is **1,060.274 GB/s** against the
960 GB/s model. This is not measured DRAM bandwidth: supplier loads are
predicate-gated and caches can serve probes. Counting only the obligatorily
scanned 8-byte order-key column gives **530.137 GB/s**. Do not interpret the
nominal rate above the model as exceeding the GPU's physical memory bandwidth.

`ptxas -v -arch=sm_89` reports:

| Kernel | Registers | Stack bytes | Spill stores / loads | Dynamic shared bytes |
|---|---:|---:|---:|---:|
| Plan A | 32 | 0 | 0 / 0 | 9,216 |
| Plan B | 34 | 0 | 0 / 0 | 33,808 |
| Dedup insertion | 18 | 0 | 0 / 0 | 0 |
| Run scan | 18 | 0 | 0 / 0 | 32 |

Static shared memory is zero for these kernels. Full assembly output is in
`compare/ptxas.txt`, with all parsed entries in `compare/ptxas.resources.json`.
The initial fixed-four-item loops produced 40-byte local stack frames despite
zero register spills; explicit `#[unroll]` on those loops removed the frames.
No Segment split is needed for the measured 32/34-register streams.

Executed correctness checks: vector-add smoke (1,025 elements); tile oracle
(82 scan, 92 compact, 246 sort, 65 owner, 114 merge-path cases); blocked-run
oracle (24 data cases plus invalid-input checks, including 131,073-row skew and
recursive scan boundaries); dedup oracle (12 cases including all-zero and
maximum encodings); complete synthetic/SF1/SF10 results for both plans.
No compute-sanitizer result is claimed.
