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
column uploads, pooled blocked-run builds, output allocation, device
canonicalization and downloads. `h05-compiled` authors the retail clauses
twice, with fact-first and customer-first written orders. It does not call
the hand-written H05 kernels.

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

The three printer arguments are optional; the displayed values are their
defaults. `frontier(0)` disables descriptor publication. The supported tile
shape is 256 threads × four items, and the supported directory is `rank`;
other shapes/directories are rejected rather than silently ignored.

The extra recorded passes are necessary. The shorter
`order(le)` + `sorted_runs` + `retain_lookups` pipeline still records host
fallback indexes for unfiltered Supplier/Lineitem and a CPU HashSet at
canon. `build_host_reads` is a CPU-neutral Physical mode rewrite: eligible
integral-key host renames become retained `let` results, without changing
the graph, keys, predicates or their order. `prefix_runs` explicitly retypes
an existing multi-level Trie with an integral single-column first level to
`cq_gpu_runtime::PrefixRun`. It does not change its key levels. Its abstraction
is the first-key clustered range filtered by all remaining key equalities,
preserving duplicates and source order. This device-only provider is not
silently substituted for an arbitrary CPU index.

| Checked Structures/Physical fact | GPU realization |
|---|---|
| Host rename and relational select/point/range | Uploaded column scan and device comparisons; string equality uses per-run dictionary IDs. |
| `let` with one-level `SortedRun` | One blocked run, built on device using the recorded integral key. |
| Explicit `PrefixRun` key levels | One prefix run, with exact residual comparisons at full-key reads. |
| Integral membership-only provider | Budgeted exact bitmap, not an inferred payload run. |
| Eager semi without a witness | Exact membership test at the recorded keyed read. |
| `share` plus matching `repr witness` | Retained `(begin,end)` descriptor, consumed at the recorded forcing probe without reprobe. |
| Probe | Block scan of range lengths, output-parallel owner search, bounded in-tile expansion loops. `first` is a traversal choice, **not** a primary-key/cardinality promise. |
| `stage = batch N` / `expand` | Bounds cooperative work by the recorded batch; only explicit expand stages publish large descriptors to the bounded frontier. |
| Project | Carries only demanded values to output. |
| Recorded `DeviceSet<_>` canon | Exact device dedup; hash collisions never establish equality. |
| Returned stream | Capacity-checked device output; overflow counts all rows, grows and relaunches the pure stream. |

Each generated kernel records its Structures keyed-read provenance. PrefixRun
witnesses are not fabricated: the existing share contract admits only
one-level payload providers, so multi-level Supplier reads retain their
ordinary lookup behavior. Inputs must remain immutable during execution.
Host tuple slices are columnized with a cache keyed by slice pointer, length,
Rust row type and schema; the cache and shared string dictionary last for one
execution, never across unrelated calls. Answers are owned typed tuples.

### Running the compiled pipeline

```sh
source /data/rccl/scripts/oxide-env.sh
cargo oxide build --arch sm_89 -- --release -p gpu-h05-compiled --bin printer-smoke
./target/release/printer-smoke
cargo oxide build --arch sm_89 -- --release -p gpu-h05-compiled --bin gpu-h05-compiled
./target/release/gpu-h05-compiled /data/tpch-gpu/sf1 both compare/gpu_h05_compiled.results.json
./target/release/gpu-h05-compiled /data/tpch-gpu/sf10 both compare/gpu_h05_compiled.results.json
```

The standalone smoke compares GPU answers with the pull printer, including
empty input. Each H05 plan checks exact sorted answers against the independent
`answer.tsv` after one warmup and five measured runs. The input dictionary is
decoded to host Strings before compilation's ordinary slice-binding path;
the runtime constructs its own per-run dictionary. Date columns use integral
days and the manifest is checked against the authored 1994 window.
`total_ms` is GPU build + query kernel + dedup time, excluding host
columnization, upload and answer download; `wall_ms` includes the full call.
Compare this scope with the hand-written SF10 totals, A 6.2 ms and B 10.1 ms,
in `compare/gpu_h05.results.json`.

Measured on the RTX 6000 Ada, 2026-10-18, both compiled plans are exact at SF1
(7,242 answers) and SF10 (72,983). These are medians of five runs after one
warmup; phase medians need not sum to the median total.

| Plan | SF1 total ms | SF10 build ms | SF10 kernels ms | SF10 dedup ms | SF10 total ms | Hand-written SF10 total ms |
|---|---:|---:|---:|---:|---:|---:|
| Compiled A, fact-first | 11.619 | 9.435 | 8.980 | 0.644 | 19.039 | 6.2 |
| Compiled B, customer-first | 14.681 | 24.371 | 92.602 | 0.730 | 117.689 | 10.1 |

The compiler reproduces the answers, **not the hand-written performance**.
It preserves the public pipeline's recorded reads, including LE semijoins
and the Supplier prefix selected by Structures. In B that prefix is
`nation_key`, with `supplier_key` tested as a residual; it is not the
hand-written supplier-key point lookup. The runtime also materializes/builds
every recorded retained stream rather than specializing the small dimensions.
No timing claim attributes the entire gap to one cause without profiling.
Full-call median wall times at SF10 are 1,149.596 ms (A) and 1,242.539 ms (B),
including repeated per-run columnization and transfers.

`compare/gpu_h05_compiled.ptxas.txt` and `.resources.json` contain the actual
`ptxas -v -arch=sm_89` audit. Final streams `cq_gpu_stream_0_r21` (A) and
`cq_gpu_stream_1_r19` (B) each use 40 registers, a 40-byte stack frame,
44 bytes spill stores and 36 bytes spill loads. The other ten generated
streams use 32 or 34 registers with zero stack/spills. The spills are
reported, not hidden by an unsupported representation or plan change.

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
and a 20,000-answer fanout using a retained witness, batch-32 expansion,
frontier publication and output grow/relaunch.


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
- No aggregates, recursion, host fallback algorithms, RCCL device algorithms,
  CUDA C or NVRTC. The existing cuda-oxide PTX/shared-memory and package-loader
  restrictions described above still apply. The consumer is a separate
  package from both device libraries.
- Finite device output cardinalities must fit the runtime's row-index domain.
  Grow/relaunch preserves exhaustive answers, but repeats pure GPU work on
  overflow; it does not assume unrecorded PK constraints.
