# Hand-written retail_h05 GPU experiment

This standalone workspace implements the de-risk experiment in
`../../doc/GPU-DESIGN.md`. It does not change MiniLinq's compiler pipeline.
All kernels are Rust compiled by cuda-oxide. The approved de-risk workaround
uses dense counting-sort builds and exact atomic-CAS dedup instead of RCCL's
device algorithms, whose merged PTX hit the compiler bug documented below.
RCCL's host-only `rccl-memory` allocator is used without a device bundle.
No CUDA C, NVRTC, OptiX or CPU join is in the measured path.

## Reproduce

Run from `experiments/gpu` in the `gpu` fork:

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
The resident integration also exposed a host-linker collision when two device
libraries exported a kernel named `decode_column`: both emitted
`cuda_oxide_codegen_v1_cuda_oxide_kernel_246e25db_decode_column`.
The FastLanes consumer kernel is therefore named `fastlanes_decode_column`;
kernel names must be distinct across these linked libraries. No toolchain
source was changed.

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
Partially initialized histogram, cursor and permutation scratch retains
`DeviceUninitBuffer` ownership; kernels receive raw pointers to only the regions
they initialize and consume, rather than claiming the entire capacity is valid.

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
in Plan B**, in warmup and all measured iterations. The stable date-filter
compaction now also preserves the input order for `orders_by_order`;
`orders_by_customer` remains unsorted. The optimization checks device data
rather than assuming dbgen ordering. See [the H05 measurements](h05/README.md)
for the newer stable-compaction and merge-mode results.

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

## Allocation and build optimization measurements (2026-10-18)

Measured on the same RTX 6000 Ada / sm_89 machine with the pinned toolchain.
`rccl-memory` built unchanged against the pin; no RCCL or cuda-oxide source
was modified. All four stages were applied and measured in order. At every
stage, SF1 and SF10, both plans, the warmup and all five measured iterations
reported `exact=true` (7,242 and 72,983 distinct triples respectively).
The final implementation is `bf34e9f`; the staged commits and complete samples
are retained in `compare/gpu_h05.results.json` under `optimization_history`.

### Before / after

Build time is the median unprofiled CUDA-event interval, including host-caused
idle time. Kernel sum is the median **build-only** sum from a separate nsys
capture, including filtering but excluding query and dedup kernels. It is not
the existing `kernel_ms` field, which measures the fused query alone.
All arrows compare the fresh original-build baseline to the final code.

| SF | Plan | Build ms | Build kernel sum ms | cuMemAlloc / cuMemFree per iteration | DtoH timed / whole iteration | Scan-pipeline DRAM MB |
|---:|:---:|---:|---:|---:|---:|---:|
| 1 | A | 3.044064 → 1.426496 | 0.242370 → 0.287329 | 60/60 → 0/0 | 12/17 → 2/7 | 50.440 → 26.416 |
| 1 | B | 4.937056 → 2.199872 | 1.088487 → 0.674179 | 77/77 → 0/0 | 15/20 → 2/7 | 51.687 → 3.084 |
| 10 | A | 13.001888 → 4.884064 | 3.683762 → 3.567031 | 63/63 → 0/0 | 12/17 → 2/7 | 864.474 → 460.751 |
| 10 | B | 22.511841 → 9.477088 | 12.061921 → 7.181522 | 80/80 → 0/0 | 15/20 → 2/7 | 876.934 → 29.904 |

Allocation/free counts include iteration-local verification and teardown, but
exclude process startup/shutdown. The final **timed** counts are also 0/0 for
every measured run. The final whole-process SF10 `both` trace has 15 allocations
and 15 frees: fourteen input uploads plus one pre-warmup pool reservation, not
timed rebuilding. The original process had eighteen persistent allocations;
its fresh SF10 A iteration count is 63, rather than the earlier reported 66.
The scope here explicitly excludes startup allocations; whole-process totals
must not be presented as timed per-run counts.

Build DtoH calls decrease from **10 (A) / 13 (B) to one**, independent of scale.
One additional timed readback obtains the query output count. The five other
DtoH transfers validate dedup metadata, the hash table and three materialized
columns **after** the final event: seven whole-iteration readbacks, not seven
build synchronizations. The final SF10 `both` trace has 84 DtoH calls for twelve
iterations. These counts come from nsys CUDA API records, not source estimates.

Scan traffic is ncu `dram__bytes.sum`, summed over the first complete build
iteration under kernel replay, in decimal MB. Before includes `scan_blocks`
and `add_prefix`; after additionally includes the necessary `word_offsets`
local-bin/directory pass, rather than hiding that traffic. Histogram and sorted
adjacency passes are not part of this scan metric. Exact byte counts are in
the JSON. On this SF10 dataset, sparse order keys span approximately **60 M
positions** despite having 15 M order rows; the dense u32 array is approximately
240 MB, not a 15 M-position array.

### Measurement after each change

| SF | Plan | Original build ms | 1: pool | 2: device sizes | 3: word scan | 4: sorted path |
|---:|:---:|---:|---:|---:|---:|---:|
| 1 | A | 3.044064 | 1.409280 | 1.291168 | 1.193888 | 1.426496 |
| 1 | B | 4.937056 | 2.595488 | 2.342464 | 2.247040 | 2.199872 |
| 10 | A | 13.001888 | 5.557216 | 5.486016 | 4.233536 | 4.884064 |
| 10 | B | 22.511841 | 14.608064 | 13.343232 | 12.388096 | 9.477088 |

The sortedness check is not free: it regresses Plan A relative to step 3
because filtered orders are not ordered. It improves SF10 B by bypassing the
lineitem histogram/scatter/provenance sort. This is measured behavior, not an
assumption that all dbgen-derived columns remain sorted after compaction.

Before enabling the sorted path, step 3 alone reduced SF10 scan-pipeline
traffic from 864,474,496 to 466,995,072 bytes (A), and from 876,934,144 to
473,732,224 bytes (B). Of those after totals, the global word scans moved only
15,568,128 / 15,977,472 bytes; the remainder is the fused local-offset pass.
Thus the reduction does not depend on sorted input.

| SF | Plan | Logical allocations per run | Peak live requested bytes |
|---:|:---:|---:|---:|
| 1 | A | 57 | 270,048,720 |
| 1 | B | 73 | 439,924,608 |
| 10 | A | 59 | 2,699,859,640 |
| 10 | B | 75 | 4,397,928,900 |

These `StatisticsMemoryResource` counters include iteration-local exactness
copies; peaks include persistent outputs but exclude uploaded inputs. The
fixed pools reserve 926,003,712 bytes at SF1 and 8,721,138,688 bytes at SF10.
Logical suballocation is still performed; **CUDA allocation/free is absent
from the timed region**. No allocator-growth fallback is allowed.

### Profiling and correctness evidence

Run unprofiled timings separately from profiling:

```sh
./target/release/gpu-h05 /data/tpch-gpu/sf10 both /tmp/h05-timings.json
nsys profile -o /tmp/h05 --force-overwrite true \
  ./target/release/gpu-h05 /data/tpch-gpu/sf10 both /tmp/h05-nsys.json
nsys stats --report cuda_api_sum,cuda_gpu_kern_sum /tmp/h05.nsys-rep
nsys export --type sqlite --force-overwrite true -o /tmp/h05.sqlite /tmp/h05.nsys-rep
ncu --kernel-name 'regex:word_offsets|scan_blocks|add_prefix' --launch-count 48 \
  --metrics dram__bytes.sum --csv --log-file /tmp/h05-ncu.csv \
  ./target/release/gpu-h05 /data/tpch-gpu/sf10 A /tmp/h05-ncu.json
```

Repeat for SF1 and Plan B. For the original scan, omit `word_offsets` from the
kernel regex. Use CPU launch timestamps/correlation IDs to identify iterations
at `filter_orders`, the build end at `plan_a`/`plan_b`, and the final timing
event after `dedup_insert`; do not confuse pool free-range events with timing
events. The ncu first-iteration scan launch counts are 22/30 (SF1 A/B) and
28/36 (SF10 A/B) before, and 21/28 and 25/32 after. Extra sampled iterations
are excluded. Captures are `/tmp/gpu-h05-before-*`, `/tmp/gpu-h05-after-*` and
the intermediate `pool`, `device-sizes`, `word-scan` variants on this host.
Persisted JSON includes the capture paths and metric scopes.

Executed final checks: pinned release build of all h05 binaries; blocked-run
oracle (40 data cases plus invalid-input checks); dedup oracle (12 cases);
all SF1/SF10 A/B iterations, including under nsys/ncu. New-kernel coverage
includes device-count tails and zero active rows, invalid counts/domains,
word and recursive prefix boundaries, sorted duplicates and provenance,
and single inversions at warp/block/large-domain boundaries. No
compute-sanitizer result is claimed.

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

## SRDatalog variable/clause-order sweep (2026-10-18)

SRDatalog is a variable-order/WCOJ-style engine, not a fixed binary-join
baseline. On RTX 6000 Ada, driver 580.178.04, engine revision
`fab849f672d43af2a6a1e8a1727ec065201e3720`, the **best exact measured**
configuration is `cnrods_clauses341205_filters`: **5.169698 ms at SF1,
176.812205 ms at SF10**, versus rerun `fact` medians of 38.139930 and
574.001685 ms. This is a bounded sweep, not a claim of global optimality.

Each variant compiles once (SF10 reuses the SF1 native build cache), then
uses a fresh initialized/loaded/prepared DB for one warmup and five timed
runs. Load and prepare are excluded; `run(0)` plus synchronization is timed.
Dimension-filter derivation and any materialized semijoins are **inside**
the timed query, not precomputed on the CPU. Whole-device VRAM is sampled
at 1 Hz: these peaks are lower bounds, not allocator high-water marks.
Clocks were not locked and exclusive device ownership was not established.

| SF1 requested order / variant | Median ms | Exactness |
|---|---:|---|
| `o,d,c,n,r,s` | rejected | 125 rows, expected 7,242 |
| `o,c,d,n,r,s` | 19.387561 | count + tuple diff |
| `o,d,s,c,n,r` | rejected | 125 rows |
| `d,o,c,n,r,s` | rejected | 0 rows |
| `c,n,r,o,d,s` | 11.226063 | count + tuple diff |
| `n,r,c,o,d,s` | 697.755019 | count + tuple diff |
| `s,n,r,o,d,c` | rejected | 125 rows |
| engine default | 37.974203 | count + tuple diff |
| `fact`: `o,s,c,d,n,r` | 38.139930 | count + tuple diff |
| `c,n,r,o,d,s` + `.with_semi_join()` | 11.279425 | count + tuple diff |
| `c,n,r,o,d,s` + clauses `3,4,5,6,1,2,0,7` | 10.552553 | count + tuple diff |
| `c,n,r,o,d,s` + dimension filters | 5.490582 | count + tuple diff |
| `c,n,r,o,d,s` + dimension filters + semijoin rewrite | 5.221426 | count + tuple diff |
| **`c,n,r,o,d,s` + dimension filters + clauses `3,4,1,2,0,5`** | **5.169698** | **count + tuple diff** |

All accepted SF1 variants return 7,242 rows in all six runs and match the
complete reference tuple set in the warmup export. **Not all attempted
orders are correct:** four requested orders compile but fail the first
count check, so their timed medians are null and they are excluded from
ranking. In particular, the planner accepts filtered-only `d` first but
the resulting query returns zero rows. No engine patch or count-check
suppression was used. The “all variants exact” criterion therefore remains
unmet for these rejected engine configurations.

The planner's own default chooses `o,s,c,n,r,d,_c0` (the last variable is
introduced for the ASIA constant); its heuristic is structural, not
cardinality/cost-based. Chosen HIR plans and emitted MIR are retained.
The plain-rule semijoin opt-in leaves its plan unchanged; unary dimension
filters create bound-key membership probes, while adding `.with_semi_join()`
also materializes filtered Customer/Supplier relations. Further exploration
of families exceeding 3× the best was stopped; the historically slow `dim`
order was not rerun. `fact` was retained as the required SF10 control.

The three fastest exact SF1 variants and `fact` were run at SF10:

| SF10 variant | Median ms | Min ms | Sampled peak MiB | Exact count, all six runs |
|---|---:|---:|---:|---:|
| **dimension filters + clauses `3,4,1,2,0,5` (best)** | **176.812205** | **176.762880** | **3,230** | **72,983** |
| dimension filters + semijoin rewrite | 192.410878 | 192.069845 | 3,230 | 72,983 |
| dimension filters, derived clause order | 198.356330 | 198.161938 | 3,230 | 72,983 |
| `fact` | 574.001685 | 573.087575 | 3,230 | 72,983 |

SF1 accepted variants all sampled 2,206 MiB. SF10 correctness here is a
count check, **not** a full tuple comparison. The winning DSL is below
(using the runner's input declarations and unary `AsiaRegion`/`AsiaNation`
relations). Removing dimension joins from H05 removes `r` from its actual
variable order; the CLI's six-variable order is explicitly projected to
`c,n,o,d,s`. No materialized semijoin opt-in is enabled for the winner.

```python
Program(rules=[
    (AsiaRegion(r) <= Region(r, 2)).named('AsiaRegion'),
    (AsiaNation(n) <= Nation(n, r) & AsiaRegion(r)).named('AsiaNation'),
    (Answer(o, s, n) <= Lineitem(o, s) & Orders(o, c, d)
     & Filter(('d',), 'return d >= 8766 && d < 9131;')
     & Customer(c, n) & AsiaNation(n) & Supplier(s, n))
    .named('H05')
    .with_plan(var_order=['c', 'n', 'o', 'd', 's'],
               clause_order=[3, 4, 1, 2, 0, 5]),
])
```

Reproduce the winner from this fork's root:

```sh
uv run --project /home/stargazermiao/workspace/srdatalog-python python -u \
  experiments/gpu/compare/srdatalog_h05.py --sf 1 10 \
  --var-order c,n,r,o,d,s --dimension-filters --clause-order 3,4,1,2,0,5 \
  --output experiments/gpu/compare/srdatalog_h05.winner-rerun.json
```

`--var-order` is repeatable and can be combined with named `--order`
variants. Clause indices address the body after constant rewriting:
ordinary H05 uses eight clauses (`0` Lineitem, `1` Orders, `2` date filter,
`3` Customer, `4` Nation, `5` Region, `6` ASIA filter, `7` Supplier);
dimension-filter H05 uses six (`4` AsiaNation, `5` Supplier).
`compare/srdatalog_h05.sweep.json` retains all 18 measured/attempted entries,
including errors, requested/chosen plans, five samples for each accepted
variant, counts, and paths to logs/MIR. `srdatalog_h05.results.json` labels
the winners `best` and retains the named historical baselines.

## Directed triangle lookup-expand experiment

`triangles` is a separate device-library package. Its benchmark and oracle are
in the sibling `triangles-runner` package, avoiding cuda-oxide's same-package
library/binary module-name ambiguity. It depends on `tiles` and `blocked-run`;
neither library is forked or copied.

The query is exactly `Triangle(a,b,c) <= E(a,b) & E(b,c) & E(a,c)`.
Input ingestion sorts and deduplicates directed edges once on the host; it
does not add reverse edges, remove self-loops, or impose `a < b < c`. This
canonicalization and upload are outside CUDA-event timing, and ingestion time
is recorded separately. The GPU builds the source-keyed blocked run with
sorted neighbors. The rank-domain budget includes histogram scratch and is
64 MiB; domains exceeding that budget or the density bound use 2048-row fences
and equal-range binary search instead. Sparse signed-i64 domains, including
both extrema and repeated fences, are covered by the oracle.

Each 1024-edge tile builds exclusive expansion offsets with four cooperative
256-thread scans. Intersection expands the smaller of `N(a)` and `N(b)` and
binary-searches the other sorted range. Parent bindings retain the original
`(a,b)`, including when the expansion side is swapped. T-sized virtual chunks
are consumed immediately; counting uses a block reduction and one u64 atomic
per consumed chunk. Optional materialization emits exact triples and reports
insufficient output capacity instead of silently dropping rows.

The 14,340-byte descriptor contains 1024 edge-row bindings, 1024 range begins,
1025 exclusive offsets, and 1024 packed u16 parent slots/side flags. Pool-on
uses `K=8`, 64 slots, and a persistent 568-block grid. Owners publish when
`M > K*T`, consume head chunks, and free the slot after all chunks have been
claimed. Blocks without root tiles steal tail chunks. A slot's acquire/release
lock protects its cursors and descriptor lifetime: a thief copies the complete
descriptor to its own shared memory before releasing the lock. Consequently
slot reuse cannot invalidate an in-flight chunk. A full pool falls back to the
owner's in-tile loop. A global root cursor and remaining-owner counter terminate
the grid; stolen chunks finish before their blocks test termination. `K=0`
disables publication. Both modes use the same fixed RCCL allocation reserve,
primed before warmup, with no CUDA allocation/free in timed regions.

The descriptor uses u32 offsets. Expansion overflow is detected rather than
wrapped into an answer; each range is currently limited to `u32::MAX/256`
items so each block scan is exact, and each descriptor total must fit u32.
Triangle totals and pool statistics use u64. These representation limits do
not constrain either measured input or the degree-1,000,000 oracle.

The BFS baseline first materializes **all** `E(a,b) & E(b,c)` wedges as
three-i64 tuples, then launches the membership filter. It does not use the
smaller-range optimization. Its required allocation is `24 * wedge_count`
bytes; results also report the `12 * wedge_count` lower demand for a possible
u32-column implementation. An allocation exceeding available VRAM is recorded
as `oom_preflight`, not as an executed BFS timing.

Reproduce from `experiments/gpu`:

```sh
source /data/rccl/scripts/oxide-env.sh
cargo oxide build --arch sm_89 -- --release -p gpu-triangles-runner --bins
./target/release/triangles-oracle
# Supply the independently obtained DuckDB count:
./target/release/gpu-triangles EDGE_FILE DUCKDB_COUNT RESULT_JSON
/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9/bin/ptxas \
  -v -arch=sm_89 target/oxide-ir/gpu_triangles.ptx -o /dev/null
```

The oracle compares pool capacities 0, 1, and 64, thresholds 0, 1, and 8,
with three repetitions of each combination. Cases include empty graphs,
duplicate input edges, self-loops, random partial tiles, a clique, sparse
repeated fences, output-capacity exhaustion, and a geometric power-law graph
whose first node has exactly 1,000,000 outgoing neighbors. It checks counts,
materialized triple membership/uniqueness, termination, released slots,
actual stealing, and pool-full fallback.

### Triangle measurements (2026-10-18)

Both inputs were exact against DuckDB on every warmup and measured GPU run:
**DBLP: 2,224,385 triangles from 1,049,866 edges**;
**LiveJournal: 946,400,853 triangles from 68,993,773 edges**.
Neither input contained duplicate edges. Both source domains fit the rank
budget. Results and the comparison table are retained in
[`compare/gpu_triangles.results.json`](compare/gpu_triangles.results.json);
per-run samples, reference SQL, console logs, and the independent comparison
runner remain alongside it.

These are medians of five runs after one warmup on an RTX 6000 Ada (sm_89).
GPU columns separate the fused query from run-build plus query. SRDatalog's
run includes `KnowsLoad` and its index builds; DuckDB's 24-thread self-join runs
on an already loaded distinct-edge table. All exclude file loading/H2D.
Our host ingestion additionally sorts the neighbor lists before upload:
67.013 ms for DBLP and 4,859.487 ms for LiveJournal, excluded from these GPU
event intervals. Thus this table is **not an end-to-end ingestion comparison**.

| Dataset | Method | Query/kernel ms | Build + query / engine run ms | Device memory / BFS demand | Stolen chunks |
|---|---|---:|---:|---|---:|
| DBLP | GPU pool off | 0.715 | 1.877 | 128 MiB sampled delta | 0 |
| DBLP | GPU pool on (`K=8`, 64 slots) | 0.711 | 1.866 | 128 MiB sampled delta | 153–171 |
| DBLP | BFS materialize, then filter | 1.293 | — | 161.699 MiB wedge allocation; 398 MiB sampled delta | — |
| DBLP | SRDatalog best: `b,a,c` | — | 5.857 | 2,206 MiB whole-device sampled lower bound | — |
| DBLP | DuckDB, 24 threads | 77 | — | CPU | — |
| LiveJournal | GPU pool off | 88.255 | 95.885 | 5,412 MiB sampled delta | 0 |
| LiveJournal | GPU pool on (`K=8`, 64 slots) | 89.431 | 101.773 | 5,412 MiB sampled delta | 1,740–3,607 |
| LiveJournal | BFS materialize, then filter | OOM preflight | — | **132.225 GiB** i64 wedges; **66.112 GiB** even with u32 triples | — |
| LiveJournal | SRDatalog best: `b,a,c` | — | 1,760.952 | 9,374 MiB whole-device sampled lower bound | — |
| LiveJournal | DuckDB, 24 threads | 35,368 | — | CPU | — |

The GPU memory delta is measured with `cuMemGetInfo` relative to the
post-module/pre-upload baseline. It includes inputs and the entire fixed RCCL
pool reserve, not just live logical suballocations. The same reserve is used
with publication disabled and enabled. DBLP's BFS peak also includes the
retained lazy-query pool, explicitly recorded in its JSON. SRDatalog's 1 Hz
whole-device sampling is a different metric and must not be treated as a
comparable allocator high-water mark.

LiveJournal has 5,915,633,945 wedges: 141,975,214,680 bytes for the implemented
three-i64 BFS layout, or 70,987,607,340 bytes for three u32 columns before any
index/scratch storage. Available device memory at preflight was
43,942,019,072 bytes; the baseline was rejected without allocating or launching.
DBLP's 7,064,738 wedges were actually materialized and filtered, with exact
counts on all six runs. The host-only wedge-demand diagnostic is separate
from query execution (112.362 ms DBLP, 48,809.696 ms LiveJournal).

Maximum `M/T` was **18.782** on DBLP and **388.771** on LiveJournal. On
LiveJournal the 64-slot pool published 7,158–7,316 descriptors per measured run
and fell back depth-first 56,863–57,021 times. Pool stealing is exercised and
correct, but **does not improve this LiveJournal measurement**: its kernel is
about 1.3% slower than the in-tile-only loop. Build medians also varied
(7.600 ms pool-off versus 11.887 ms pool-on), so total-time differences cannot
all be attributed to descriptor stealing.

Final GPU timings were taken after the reference and H05 GPU runs finished;
there was no intentional concurrent GPU benchmark. The machine was **not
quiet**: recorded 1/5/15-minute CPU load averages were `21.94/21.39/12.85`
(DBLP) and `22.31/21.20/13.29` (LiveJournal). Concurrent CPU gates and host
scheduling can affect CUDA-event intervals that span host submissions.

SRDatalog's actual `examples/lsqb_triangle_count.py` is imported by
`compare/srdatalog_triangles.py`; only the requested variable plan is replaced.
Its default selected `a,b,c`. All six dataset/order combinations completed and
matched DuckDB on the warmup and every measured run:

| SRDatalog order | DBLP ms | LiveJournal ms |
|---|---:|---:|
| engine default (`a,b,c`) | 6.618 | 2,576.524 |
| explicit `a,b,c` | 6.547 | 2,575.829 |
| explicit `b,a,c` | **5.857** | **1,760.952** |

```sh
uv run --project /home/stargazermiao/workspace/srdatalog-python python \
  compare/srdatalog_triangles.py --dataset dblp livejournal \
  --order default abc bac
./target/release/gpu-triangles /data/dataset/graph/com-dblp.ungraph.txt \
  2224385 compare/triangles_dblp.gpu.json
./target/release/gpu-triangles /data/dataset/graph/soc-LiveJournal/edge.facts \
  946400853 compare/triangles_livejournal.gpu.json
```

`compare/triangles.oracle.txt` records the passing oracle, including 32,394
stolen chunks across the degree-1,000,000 graph's protocol variants and
3,999,859 exact triangles. `compare/triangles.ptxas.txt` records the actual
resource audit: the persistent kernel uses **40 registers**, **14,468 bytes
dynamic shared memory**, a **104-byte stack frame**, and **zero spill stores
or loads**. The BFS filter uses 18 registers, 32 bytes dynamic shared memory,
and zero stack/spills. No CUDA C, NVRTC, or upstream checkout changes were used.

## Structures GPU terminal printer

`printer` is the external proc-macro crate `cq-gpu`; `printer-runtime` owns
typed resident hosts, pooled blocked-run builds, output allocation and device
canonicalization. `device-columns` (`gpu-device-columns`, re-exported through
`cq_gpu_runtime`) owns the pooled column buffers and device codecs.
`h05-compiled` authors the retail clauses twice, with fact-first and
customer-first written orders. It does not call the hand-written H05 kernels.

```rust,ignore
pipeline {
    plan = ::cq_planners::order(le);
    physical = [::cq_rewrites::build_host_reads];
    structures = [
        ::cq_rewrites::sorted_runs,
        ::cq_gpu::prefix_runs,
        ::cq_rewrites::retain_lookups,
        ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>),
    ];
    print = ::cq_gpu::print(tile(256,4), frontier(8), directory(rank));
}
```

The displayed printer arguments are optional; these are their defaults.
`frontier(0)` disables descriptor publication. The supported tile shape is
256 threads × four items, and the supported directory is `rank`; other
shapes/directories are rejected rather than silently ignored.
`capacity(Relation)` additionally records an explicit output allocation bound
from a named input relation. H05 records `capacity(Lineitem)`; its chained
identity query records `capacity(Input)`. These are allocation contracts, not
inferred functional dependencies, uniqueness or sortedness.

The extra recorded passes are necessary. The shorter
`order(le)` + `sorted_runs` + `retain_lookups` pipeline still records host
fallback indexes for unfiltered Supplier/Lineitem and a CPU HashSet at
canon. `build_host_reads` is a CPU-neutral Physical mode rewrite: eligible
integral-key host renames become retained `let` results, without changing
the graph, keys, predicates or their order. `prefix_runs` explicitly retypes
an existing multi-level Trie to `cq_gpu_runtime::PrefixRun`. Without arguments
it records the existing first singleton integral level. An explicit
`prefix_runs(Supplier => supplier_key)` selects a singleton level by base
relation or retained-result name and moves it to the directory position.
The provider's fourth const argument records that column's Structures layout
index; the printer rejects disagreement with the first level. Other levels
remain exact residual equalities. Displaced singleton-prefix reads receive
additional recorded one-level SortedRuns and retained Vec rows, rather than
silently losing semijoins. B therefore preserves its nation-key semijoin while
its full Supplier lookup uses the supplier-key directory.

| Checked Structures/Physical fact | GPU realization |
|---|---|
| Host rename and relational select/point/range | Resident column scan and device comparisons; string equality uses the host's shared dictionary IDs. Ordinary tuple bindings explicitly take the upload path. |
| `let` with one-level `SortedRun` | One blocked run, built on device using the recorded integral key. |
| Explicit `PrefixRun` key levels | One prefix run, with exact residual comparisons at full-key reads. |
| Integral membership-only provider | Budgeted exact bitmap, not an inferred payload run. |
| Recorded `join_filter` | Budgeted bitmap prefilter on its recorded scan column, retaining the covering exact keyed read. |
| Eager semi without a witness | Exact membership test at the recorded keyed read. |
| `share` plus matching `repr witness` | Retained `(begin,end)` descriptor, consumed at the recorded forcing probe without reprobe. |
| Probe | Unstaged exhaustive lane-local range traversal; recorded batch/expand streams retain block scans, owner search and bounded cooperative expansion. `first` never implies uniqueness. |
| `stage = batch N` / `expand` | Bounds cooperative work by the recorded batch; only explicit expand stages publish large descriptors to the bounded frontier. |
| Project | Carries only demanded values to output. |
| Recorded `DeviceSet<_>` canon | Exact device dedup; hash collisions never establish equality. |
| Returned stream | Owned resident device columns, with an explicitly recorded capacity contract or bounded grow-and-resume execution. Tuple transfer requires `.to_host()`. |

Each generated kernel records its Structures keyed-read provenance. PrefixRun
witnesses are not fabricated: the existing share contract admits only
one-level payload providers, so multi-level Supplier reads retain their
ordinary lookup behavior. Inputs must remain immutable during execution.
Resident `DeviceColumns<'data, Row, C = Plain>` hosts own their encoded columns,
row metadata and string dictionary domain. Column allocations retain the
shared pool, CUDA stream and context through reference-counted ownership:
neither inputs nor answers borrow a temporary execution `Runtime`. Passing
`&DeviceColumns` to another compiled query shares the allocation rather than
columnizing or uploading it again. The terminal returns an owned device host,
not a host iterator; `.to_host()?` is the explicit tuple-transfer boundary.
The H05 loader deliberately drops its construction runtime before executing
either plan. Ordinary tuple bindings still upload for that execution; they do
not create a persistent cache keyed by host pointer.

### Resident host codecs and recorded derived codecs

Host compression is a property of the supplied value, not a printer decision
and not an extra IR host-codec entry. The loader supplies `Codec::Plain`,
`Codec::For`, `Codec::Delta` or `Codec::Dictionary` to `Runtime::upload_owned_rows`;
device encoders prepare 1024-value FastLanes vectors. Generated loads consume
the column/vector metadata through the device decoder. They do not first
download or expand the whole relation to host tuples. H05 calls
`detect_integer_codec` on each numeric column during loading; this estimates
the actual vector widths and selects delta only when at most 1/16 adjacent
transitions descend and delta is smaller including its base metadata.
Region names use dictionary IDs. All six relations are loaded once using
one `Dictionary` and runtime, so string equality has one compatible ID domain.
`upload_owned_rows` uses `OwnedRowCodec` tuples and an owning string dictionary,
returning `DeviceColumns<'static, Row>` without retaining a borrow of the
source tuples. The separate `upload_rows` path preserves existing borrowed
string hosts. The `'data` parameter tracks host string/binding compatibility,
never an execution runtime lifetime.

The 64-bit FastLanes layout has sixteen lanes and the `04261537` unified
transposed ordering. A packed vector of width W consumes W × 128 data bytes.
Each vector has a 32-byte `VectorHeader` recording its word offset, base,
width, codec and valid-row count. Delta uses sixteen independent 64-value
sequences with sixteen 64-bit restart bases, avoiding a whole-vector serial
prefix dependency; its signed deltas are frame-of-reference packed.
Host loading obtains the exact encoded extent before allocation. Derived
outputs retain a conservative device allocation rather than reading their
extent back merely to resize the answer.

Resident execution retains allocation capacity separately from the device live
count. Result columns and decode scratch are allocated without clearing unused
tails; every consumer is count-gated. Plain results adopt their native column
buffers and count owner without encoding or copying. Recorded compressed results
build headers, offsets and payloads only for live vectors, including the partial
last vector. A runtime retains dedup scratch capacity across calls, but clears
the device-count-sized active set on each call; no relation data is reused.
Membership bitmap allocations are capped by the recorded key bounds as well as
the recorded byte budget. Runtime and generated code-object caches retain the
actual context owner, not just a potentially recycled context identifier.
Sorted-run starts/directory construction and per-bin ordering use bounded grids
with device-count/domain grid strides, avoiding capacity-sized grids whose
threads immediately exit on sparse filtered relations.

Derived storage is different: its codec is an explicit Structures provider
fact, selected by a rewrite rather than guessed from a query or Rust host
value. The default recorded device canon is plain:

```rust,ignore
::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)
```

Both H05 plans instead record packed frame-of-reference answers:

```rust,ignore
::cq_rewrites::retype(dedup =>
    ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::For>)
```

Their return type is `DeviceColumns<'_, (i64, i64, i64), For>`. A second compiled
identity query binds this device host and records an ordinary plain
`DeviceSet<_>` answer. Only that second result is downloaded. This makes the
packed-output-to-resident-input boundary part of every H05 exactness run,
without changing either H05 clause order or its keyed-read facts.

The design follows the register/tile decoding motivation in Afroozeh, Felius
and Boncz, [*Accelerating GPU Data Processing using FastLanes Compression*,
DaMoN 2024](https://doi.org/10.1145/3662010.3663450)
([paper](https://ir.cwi.nl/pub/34260/34260.pdf)): keep compressed columns in
VRAM and decode values where the query consumes them, rather than writing
an entire decompressed relation back to global memory. The paper's speedups
are not measurements of this Rust implementation. The 1024-value storage
vector is not a requirement to keep 1024 decoded values live per thread.

Root decoding keeps two named scalar values at a time rather than an indexed
local-memory array. Each warp owns paired positions in two 64-value Delta sequences; one
warp-prefix sum per sequence replaces duplicate lower-half decoding. Narrow
residuals use exact 32-bit sums with the signed frame base added separately;
wider residuals retain wrapping 64-bit arithmetic. Selection kernels decode
convergently before validity filtering. These paths use lane shifts and warp
shuffles, not shared-memory decode buffers or block barriers.
Root column descriptors are passed as immutable kernel parameters, retaining
their device count pointers; there is no per-tile descriptor-table round trip.
Serial scan kernels request four resident blocks per multiprocessor to limit
register pressure. Packed payload accesses remain scalar/coalesced UTL loads:
paired logical values are not contiguous packed words, so blindly substituting
vector loads would fetch unrelated lanes.

### Lifetime, VRAM and capacity errors

All device buffers, including codec headers, payloads and scratch space, use
the `rccl-memory` pool. No RCCL device algorithm, managed-memory spill, CPU
fallback or out-of-core partitioner is introduced. The working set must fit
VRAM: resident inputs plus indexes, transient encoding/build storage and
output capacity all count. Compression of final columns alone does not bound
peak memory. Allocation failure is reported as `CQ_GPU_VRAM_EXCEEDED`;
reducing the dataset or explicitly chosen capacity is a caller decision,
not an implicit change of query semantics.

Pool/stream/context ownership follows the columns, so a device result can
outlive the runtime that produced it and feed another query. Explicit capacity
overflow is `CQ_GPU_OUTPUT_CAPACITY`, retained as device status and surfaced
at the explicit download boundary; it is not a truncated successful answer.
No tuple readback is required between the two compiled H05 calls.

### Running the compiled pipeline

```sh
source /data/rccl/scripts/oxide-env.sh
cargo oxide build --arch sm_89 -- --release -p gpu-h05-compiled --bin printer-smoke
./target/release/printer-smoke
cargo oxide build --arch sm_89 -- --release -p gpu-h05-compiled --bin gpu-h05-compiled
./target/release/gpu-h05-compiled /data/tpch-gpu/sf1 both compare/gpu_h05_compiled.resident.optimized.results.json
./target/release/gpu-h05-compiled /data/tpch-gpu/sf10 both compare/gpu_h05_compiled.resident.optimized.results.json
```

The standalone smoke compares GPU answers with the pull printer, including
empty input. Each H05 plan and its chained compiled identity query check exact
sorted answers against independent `answer.tsv` after one warmup and five
measured runs. Region names are decoded from the input file dictionary once
and assigned IDs in the shared resident dictionary during loading. Dates are
integral days; the manifest must match the authored 1994 window.

The resident JSON records, separately:

- `one_time.host_load_ms`: projected-file and oracle loading.
- `one_time.load_encode_ms`: runtime initialization, row preparation, upload
  and device encoding, once for all six relations. Each relation also records
  its `load_encode_ms` around `upload_owned_rows`, chosen codecs, row count,
  logical `raw_bytes` and `fastlanes_bytes` including vector headers and
  extent metadata. These byte counts are not allocator high-water marks,
  PCIe counters or the size of host string allocations. Codec selection and
  tuple preparation are in the all-relations load interval, not the narrower
  per-relation `upload_owned_rows` interval.
- `build_ms`, `kernel_ms`, `dedup_ms`, `total_ms`: runtime GPU timings for
  the first compiled query; `wall_ms` is that resident call's host elapsed
  time, excluding initial loading, the chained call and final download.
- `calls`: individual warmup and measured call facts, including separate
  wall/build/kernel/dedup/total values for the second compiled query, packed
  answer allocation bytes, exactness and the final `download_ms`.
  Resident result `raw_bytes` describes uncompressed capacity, not an implicit
  readback of live row count; encoded answer storage may be conservative.

The repaired resident results are in `gpu_h05_compiled.resident.optimized.results.json`;
the before/after comparison and evidence ledger are in
`gpu_h05_compiled.resident.summary.json`. Profile separately from unprofiled samples:

```sh
CQ_GPU_PROFILE=1 nsys profile --trace=cuda --capture-range=cudaProfilerApi \
  --capture-range-end=stop --force-overwrite=true -o compare/compiled_resident_optimized_A \
  ./target/release/gpu-h05-compiled /data/tpch-gpu/sf10 A \
  compare/gpu_h05_compiled.resident.optimized.profile.results.json
nsys stats --report cuda_gpu_kern_sum,cuda_api_sum --format csv \
  compare/compiled_resident_optimized_A.nsys-rep
ncu --target-processes all --kernel-name 'regex:cq_gpu_stream_0_r21' \
  --launch-skip 1 --launch-count 1 \
  --metrics dram__bytes.sum,gpu__time_duration.sum,dram__throughput.avg.pct_of_peak_sustained_elapsed \
  --force-overwrite -o compare/compiled_resident_optimized \
  ./target/release/gpu-h05-compiled /data/tpch-gpu/sf10 A \
  compare/gpu_h05_compiled.resident.optimized.ncu.results.json
/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9/bin/ptxas \
  -v -arch=sm_89 target/oxide-ir/gpu_h05_compiled.ptx -o /dev/null
```

`CQ_GPU_PROFILE` brackets the first measured resident call and its chained
query with CUDA profiler control APIs. Loading and the final explicit download
are outside that capture range, so transfer absence can be checked directly
rather than inferred from whole-process transfer totals.

The codec oracle is a separate binary-only package to avoid the pinned
cuda-oxide package-loader ambiguity:

```sh
cargo oxide build --arch sm_89 -- --release -p gpu-device-columns-oracle
./target/release/gpu-device-columns-oracle
```

It compares independently constructed paper-layout bits and headers, not only
encode/decode round trips, and exercises empty/partial vectors, signed extrema,
all widths, per-lane delta restarts, and device-count-below-capacity encoding.

#### Resident phase repair measurements (2026-10-18)

RTX 6000 Ada, sm_89; one warmup and five unprofiled measured calls. Both plans
and their chained queries were exact in every SF1/SF10 call (7,242 / 72,983
answers). The final codec oracle passed **595** cases, the blocked-run oracle
passed **42** data cases plus invalid-input checks, printer smoke/expansion
oracles passed, and **32** printer tests including UI fixtures passed.

SF10 A medians, from `gpu_h05_compiled.results.json`,
`gpu_h05_compiled.resident.before.results.json` and
`gpu_h05_compiled.resident.optimized.results.json`, respectively:

| Per-call median | Pre-resident re-upload | Initial resident | Repaired resident |
|---|---:|---:|---:|
| Build | 1.972 ms | 14.512 ms | **1.880 ms** |
| Kernel phase | 1.044 ms | 11.338 ms | **1.004 ms** |
| Dedup phase | 0.069 ms | 12.459 ms | **0.039 ms** |
| Total | 3.087 ms | 38.319 ms | **2.924 ms** |
| Wall | 894.274 ms | 39.554 ms | **3.054 ms** |

All five repaired A totals are below 3.5 ms; the largest wall-minus-total
difference is **0.143 ms**. Phase medians need not sum to the median total.
Warmup is retained in JSON but excluded from these medians; code loading and
initial pool growth are not steady-state query costs.

| Repaired plan | SF | Build ms | Kernel ms | Dedup ms | Total ms | Wall ms | Exact |
|---|---:|---:|---:|---:|---:|---:|---|
| A | 1 | 0.491 | 0.130 | 0.033 | 0.656 | 0.778 | yes |
| B | 1 | 1.408 | 0.115 | 0.035 | 1.558 | 1.699 | yes |
| B | 10 | 15.090 | 0.488 | 0.037 | 15.620 | 15.776 | yes |

Fresh nsys captures bracket the first measured compiled query and its compiled
consumer, not just the main kernel. The following complete operation list sums
the first query's build/kernel/dedup work, excluding the consumer. Full per-phase
lists, launch counts, memset bytes, timelines and event residuals are recorded
in `gpu_h05_compiled.resident.before.attribution.json` and
`gpu_h05_compiled.resident.optimized.{A,B}.attribution.json`.

| SF10 A operation | Before repair GPU ms | After repair GPU ms |
|---|---:|---:|
| `memset` | 13.403 | 0.203 |
| `offsets` | 11.951 | 0.009 |
| `encode` | 5.044 | 0.071 |
| `cq_gpu_stream_0_r8` (Orders selection, two passes) | 2.940 | 0.568 |
| `headers` | 1.797 | 0.020 |
| `cq_gpu_stream_0_r21` (main) | 1.790 | **0.988** |
| `gather` | 0.258 | 0.262 |
| `sorted_starts` | 0.156 | 0.121 |
| `sorted_directory` | 0.142 | 0.104 |
| `order_bins` | 0.128 | 0.005 |
| `word_offsets` | 0.074 | 0.074 |
| `decode_column` | 0.065 | 0.006 |
| `sorted_check` | 0.041 | 0.041 |
| `set_insert` | 0.040 | 0.013 |
| `scatter` | 0.038 | 0.038 |
| `histogram` | 0.038 | 0.038 |
| `cq_gpu_stream_0_r5` | 0.036 | 0.038 |
| `scan_blocks` | 0.035 | 0.035 |
| `initialize_pointer` | 0.022 | 0.021 |
| `initialize_view` | 0.020 | 0.019 |
| `add_prefix` | 0.015 | 0.015 |
| `initialize_count` | 0.010 | 0.010 |
| `pack` | 0.010 | 0.004 |
| `fences` | 0.008 | 0.007 |
| `validate_output` | 0.007 | 0.006 |
| `cq_gpu_stream_0_r3` | 0.006 | 0.006 |
| `cq_gpu_stream_0_r2` | 0.005 | 0.005 |
| `clear_histogram` | 0.005 | 0.005 |
| `complete_stats` | 0.005 | 0.005 |
| `initialize` | 0.004 | 0.004 |
| `count_u32` | 0.004 | 0.004 |
| `run_status` | 0.004 | 0.004 |
| `membership_insert` | 0.002 | 0.002 |
| `membership_bounds` | 0.002 | 0.002 |
| `clear_set` | 0.000 | 0.003 |

There was **no individual 10-ms kernel**. The regressions were aggregates:
initialization plus codec work consumed **10.271 ms in build**, **9.549 ms in
the kernel phase**, and **12.373 ms in dedup**. The main kernel added 1.790 ms
to the kernel phase; actual set insertion took only 0.040 ms. Clearing 5.398 GB
and encoding capacity-sized Plain columns, not set probing, caused the dedup
regression. The repaired set clears only its device-count-sized active prefix;
recorded For result encoding processes 72,983 rows, not 59,986,052 capacity rows.

| nsys A phase | Before GPU work ms | After GPU work ms | Before event residual ms | After event residual ms |
|---|---:|---:|---:|---:|
| Build | 14.322 | 1.729 | 0.577 | 0.541 |
| Kernel | 11.348 | 0.996 | 0.031 | 0.019 |
| Dedup | 12.431 | 0.027 | 0.037 | 0.028 |

Residuals are event-interval time not occupied by the listed device operations:
launch gaps and host/profiler overhead, not attributed to invented kernels.
Profiled timings are not substituted for the unprofiled table. Plan B's separate
trace reports a 0.475-ms main kernel; its larger build includes 5.773 ms of
per-bin stable ordering and 1.532 ms of Lineitem decoding, retained explicitly
in the B attribution rather than hidden in A's improvement.

One-time SF10 load/encode is **1,893.642 ms**, including runtime setup,
host column preparation, upload and completed GPU encoding; file/oracle loading
is separately **1,578.936 ms**. Input storage remains:

| Relation | Raw column bytes | FastLanes bytes |
|---|---:|---:|
| customer | 24,000,000 | 1,218,896 |
| orders | 360,000,000 | 74,534,136 |
| lineitem | 959,776,832 | 176,211,664 |
| supplier | 1,600,000 | 81,552 |
| nation | 400 | 592 |
| region | 80 | 592 |

Small relations expand because vectors retain headers, lane bases and padded
payloads. Byte accounting includes encoded data, per-vector headers and extent
metadata, not pool reserve or host dictionary strings.

Separate ncu kernel-replay captures measure main-kernel DRAM traffic falling
from **532,209,024** pre-resident bytes to **202,497,920 bytes** (**61.95%**
lower). The replay duration is **2.360 ms**, not the unprofiled timing.
`compiled_resident_optimized.ncu.csv` and `.resources.json` retain the evidence:
64 registers/thread, no block barriers in the main kernel, a 64-byte stack,
88 bytes of spill stores and 104 bytes of spill loads. The occupancy bound
deliberately trades these spills for the measured 0.988-ms nsys main kernel;
this is not a zero-spill claim.

Both `compiled_resident_optimized_A` and `_B` nsys captures include the first
query and its compiled consumer, excluding loading and final `.to_host()`.
Each records **zero HtoD and zero DtoH calls**, no memcpy activity and no module
loads; A/B contain 30/33 device memsets respectively. Their `_cuda_api_sum.csv`,
`_cuda_gpu_kern_sum.csv` and attribution JSON retain the measured evidence.
The explicit-capacity path has no control readback; generic grow-and-resume's
permitted four-byte control flag remains a separate execution contract.


The following tables and `gpu_h05_compiled.results.json`, `.profile.json`,
`.resources.json` and `compiled_final.nsys.csv` are **historical,
pre-resident** evidence, not results for the current resident implementation.
The historical JSON entries are labeled accordingly; their recorded samples
are preserved. The original hand-written SF10 totals (A 6.2 ms, B 10.1 ms)
remain in `compare/gpu_h05.results.json`; the subsequent hand-written
comparison is `compare/gpu_h05_compiled.handwritten.results.json`.

Historical measurements on RTX 6000 Ada, 2026-10-18: every warmup and measured
run was exact at SF1 (7,242 answers) and SF10 (72,983). Medians of five
unprofiled runs after one warmup; phase medians need not sum to median total:

| SF | Plan | Implementation | Build ms | Kernel ms | Dedup ms | Total ms |
|---:|:---:|---|---:|---:|---:|---:|
| 1 | A | Compiled | 0.512 | 0.114 | 0.058 | 0.686 |
| 1 | A | Hand-written | 0.274 | 0.093 | 0.461 | 0.830 |
| 1 | B | Compiled | 1.328 | 0.109 | 0.059 | 1.495 |
| 1 | B | Hand-written | 0.831 | 0.043 | 0.463 | 1.337 |
| 10 | A | Compiled | 1.972 | 1.044 | 0.069 | 3.087 |
| 10 | A | Hand-written | 2.336 | 0.943 | 0.475 | 3.760 |
| 10 | B | Compiled | 14.201 | 0.506 | 0.069 | 14.767 |
| 10 | B | Hand-written | 8.417 | 0.304 | 0.480 | 9.202 |

Separate final `nsys` per-kernel medians (six launches, including warmup):

| SF10 kernel | Compiled ms | Hand-written ms |
|---|---:|---:|
| A | 1.031799 | 0.942022 |
| B | 0.503507 | 0.300995 |

Complete build/query/dedup kernel summaries and CUDA API counts are retained in
`compare/compiled_final.nsys.csv` and `handwritten_final.nsys.csv`.

Compiled SF10 A is **1.11×** the fresh hand-written kernel, below the requested
1.3× bound against 0.94 ms, with zero spills. Its build is below both the fresh
hand-written build and the original 4.9 ms reference. B's kernel falls from
92.602 to 0.506 ms; its remaining build and traversal overhead is not hidden.
In these historical captures, host columnization and upload occurred on every
execution; only allocator capacity and kernel code objects were reused.

#### A diagnosis and emitter changes

The old A kernel emitted a cooperative descriptor, two block scans (including
overflow checking), shared parent/witness save/restore and owner search at every
probe. Its four items per thread were four serial 256-row stripes, each paying
the nested synchronization costs. PTX contained **37 barrier sites**, versus
eight in the hand-written implementation. There was no uninlined generic trait
dispatch: generated A PTX contained no calls and no explicit local-memory
instructions. `ptxas` introduced eleven 32-bit spill stores and nine loads
(44/36 bytes), with a 40-byte frame, while allocating the live nested scheduler
state. SASS additionally outlines warp-sync/shuffle sequences as local calls;
these are not Rust trait dispatch.

Unstaged probes now exhaust their full ranges in lane-local nested loops.
This is a cardinality-independent lowering, not a PK inference from `first`.
Recorded batch/expand streams retain cooperative descriptors and frontier
publication. Witness `(begin,end)` values stay in registers and their forcing
read still does not reprobe. Root-column loads are demand-driven, so supplier
keys are read after order/customer filters. Output capacity checking and
grow/relaunch remain exact; the dedup set is touched only by the later dedup
kernel, not by each input row. Integer predicates and dictionary IDs remain
device integer comparisons.

The generated scan retains static grid-stride scheduling; the hand-written
scan uses an atomic dynamic tile cursor and packs four results per thread
before block-coalesced output. Neither examined PTX uses vectorized read-only
`ld.global.nc` loads: both use scalar generic 64-bit loads with coalesced lane
addresses. The speedup is therefore not attributed to vectorization or an
invented read-only-cache optimization. Static PTX/SASS evidence is retained in
`compare/gpu_h05_compiled.assembly.json`.

#### Hardware counters

Separate SF10 A `ncu` kernel-replay captures, one profiled launch each; these
are hardware counters, not the README's logical `scan_gbps` estimate:

| A kernel | DRAM bytes | DRAM % peak | Achieved occupancy % | Barrier stall % | Long-scoreboard stall % | Wait stall % |
|---|---:|---:|---:|---:|---:|---:|
| Compiled before | 994,048,256 | 8.01 | 42.40 | 54.97 | 11.62 | 9.76 |
| Compiled after | 532,209,024 | 36.14 | 58.36 | 0.00 | 62.32 | 16.74 |
| Hand-written | 573,238,912 | 39.65 | 66.48 | 26.01 | 52.49 | 10.44 |

Replay durations were 13.612, 1.616 and 1.587 ms respectively; use the
unprofiled table above for performance ratios. Raw metric names/values are in
`compare/compiled_a.before.ncu.csv`, `compiled_a.after.ncu.csv` and
`handwritten_a.ncu.csv`.

The final B captures report 87,557,504 DRAM bytes at 13.21% peak for compiled,
versus 87,058,816 bytes at 28.81% for hand-written. Achieved occupancy is
52.82% versus 32.81%; long-scoreboard stalls are 75.89% versus 52.04%.
Their replay durations are 0.727488 versus 0.331648 ms. Full B counters are in
`compare/compiled_b.after.ncu.csv` and `handwritten_b.ncu.csv`; all parsed
profiles, original results and intermediate stages are in
`compare/gpu_h05_compiled.profile.json`.

#### Build/runtime changes and resources

In the pre-resident implementation, pure projections shared execution-owned allocations.
Filtered nonexpanding streams, including semijoins, use stable count/scan/scatter
and inherit conservative source bounds. Runs allocate/gather only the selected
logical prefix. Output count/error metadata uses one readback. Disabled
frontiers allocate no counters and launch no initializer.

Profiling also exposed a dominant host-side cause: the intermediate compiled A
capture loaded and unloaded CUDA modules **120 times over six executions**,
with 33.359 ms in `cuModuleLoadData`. The pinned loader does not cache those
modules. The experimental runtime now retains its module, and blocked-run
caches its module by actual CUDA-context identity. These are code-object
lifetimes, not changes to Structures or upstream cuda-oxide. Those historical
executions still rebuilt every recorded run and dictionary.
The final compiled **both-plan** capture has 14 module loads across twelve
executions, including per-call generated query modules and startup owners,
versus 120 across six A executions before code-object retention. These are
whole-process counts, not falsely normalized timed-region allocation counts.

`compare/gpu_h05_compiled.ptxas.txt` and `.resources.json` contain the final
`ptxas -v -arch=sm_89` audit:

| Kernel | Registers | Stack bytes | Spill stores / loads bytes |
|---|---:|---:|---:|
| Compiled A `cq_gpu_stream_0_r21` | 48 | 0 | 0 / 0 |
| Compiled B `cq_gpu_stream_1_r19` | 40 | 24 | 28 / 20 |
| Hand-written `plan_a` | 36 | 0 | 0 / 0 |
| Hand-written `plan_b` | 34 | 0 | 0 / 0 |

The seven other generated kernels use 22–26 registers and zero stack/spills.
B's residual spills are reported explicitly; the zero-spill acceptance is
met for A, not claimed for B.

Additional acceptance commands:

```sh
cargo +stable test -p cq-gpu
cargo oxide build --arch sm_89 -- --release -p gpu-h05-compiled --bin printer-oracle
./target/release/printer-oracle
```

The compiler tests check exact keyed-read coverage and witness no-reprobe;
four public-macro compile-fail fixtures name `CQ_GPU_MERGE`,
`CQ_GPU_UNION`, `CQ_GPU_NON_INTEGRAL_KEY` and `CQ_GPU_OPAQUE_GUARD`.
The GPU oracle exercises full-width signed multiword dedup with strings,
and a 20,000-answer fanout using a recorded scan join filter, retained witness,
batch-32 expansion, frontier publication and output grow/relaunch.
The smoke additionally checks an unstaged 4,097-row fanout (including output
growth) and mutates its input in place before another execution, verifying that
pool/module reuse does not retain stale input data. The historical run passed
21 printer unit tests and all four compile-fail fixtures; rerun the commands
above for current resident-code verification.


### Supported domain and explicit rejections

- No computed head expressions, union/fallback blocks, anti, merge, smallest traversal, opaque Rust
  binds/guards, or host keyed fallback execution. Total comparisons and
  Boolean connectives are lowered; `#[total]` is not permission to emit an
  arbitrary Rust function on device.
- Keys must be integral. Strings support dictionary equality, not lexical
  ordering or arbitrary string methods. Integer encoding is checked to fit
  signed 64 bits. A blocked rank domain cannot exceed 2^32 positions.
- Ordinary multi-level providers are rejected; only explicitly recorded
  `PrefixRun` has the residual-filter interpretation above. A recorded CPU
  canon provider is rejected; `retype(dedup => DeviceSet<_>)` is required.
- Positional carriers, positional selection/read adapters and standalone keyset
  annotations are rejected. Multiple recorded
  run providers are selected by their exact served key prefixes. Recorded join
  filters support the exact `AdaptiveKeySet` bitmap
  provider and its budget; Bloom or arbitrary filter implementations are not
  silently substituted. Unknown provider aliases and non-Vec retained-row
  representations are rejected.
- No aggregates, recursion, host fallback algorithms, RCCL device algorithms,
  CUDA C or NVRTC. The existing cuda-oxide PTX/shared-memory and package-loader
  restrictions described above still apply. The consumer is a separate
  package from both device libraries.
- Finite device output cardinalities must fit the runtime's row-index domain.
  An explicit `capacity(Relation)` is a checked allocation contract and never
  establishes an unrecorded PK constraint. The default bounded-output path
  saves each lane's program position, live values and expansion cursors,
  shared descriptors and claimed frontier-task identity. A failed output
  reservation leaves the output count unchanged. The runtime reads one
  32-bit control word per attempt, doubles capacity, preserves completed
  rows with a device-to-device copy, and resumes only unfinished work.
  It does not count then replay the stream or recompute completed rows.
  Explicit `capacity(Relation)` uses the direct bounded kernel without this
  per-attempt control readback; tuple transfer remains explicit in both paths.
