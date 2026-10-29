# MCCL: MLX-backed GPU operations and indexes

Stable ascending merge, multiset difference, a blocked Bloom filter, an
ordered-bulk u32 hash map, and immutable u32-key/u32-payload indexes with GPU
radix/spline construction, stable merge and CPU lookup. Rust calls a C ABI backed
by MLX primitives and offline-compiled Metal kernels. CPU comparison/radix index
construction is explicitly selectable; there is no automatic CPU fallback or
Python runtime dependency.

## Build and run

Requires Apple silicon, Xcode with the Metal Toolchain component, CMake >= 3.27,
and Rust. The first CMake configure downloads dependencies. Run these commands
from the MCCL crate directory: the standalone repository root, or
`experiments/mac/mccl` inside mini_linq. The same source tree is published in
both repositories; mini_linq keeps it outside its portable core workspace.

```sh
cmake -S . -B build/native -DCMAKE_BUILD_TYPE=Release
cmake --build build/native --parallel 8
cargo test --release
cargo run --release -- check
cargo run --release -- bench --sizes 1K,64K,1M,4M,16M,32M --reps 50 --warmup 10
```

`MCCL_NATIVE_DIR` overrides the native library directory when building Rust.
It defaults to `build/native` relative to the MCCL manifest directory; relative
overrides resolve against that directory, not the caller's working directory.
Libraries and metallibs must remain together. CMake records the configured native
directory in the dylib install names, so downstream Rust dependencies load them
without per-executable rpaths or `DYLD_*` settings. The extension finds its
metallib relative to its own dylib. Relocating the bundle requires reconfiguring
and rebuilding the native target, then relinking consumers.

Dependencies are pinned in CMake: mlx-c `a341b4925024b88b2c593468f16e12f5e17315da`
and MLX v0.32.2, commit `1f8e74e3f12f31365464a6867c6579f0e9b29d85`.
Their upstream license notices remain in the fetched source trees.

## Optimized profiling build

Build profiling artifacts separately so the executable and the native libraries
all contain debugging information while retaining release optimization:

```sh
cmake -S . -B build/profile -DCMAKE_BUILD_TYPE=Release -DMCCL_PROFILE=ON
cmake --build build/profile --parallel 8
MCCL_NATIVE_DIR=build/profile cargo build --profile profiling
xcrun dsymutil target/profiling/mccl
xcrun dsymutil build/profile/libmccl_native.dylib
./target/profiling/mccl check
```

`MCCL_PROFILE` enables C/C++ `-g` and Metal
`-O3 -gline-tables-only -frecord-sources`. Cargo's `profiling` profile inherits
release optimization, retains full debug info, and disables stripping. Do not
accidentally profile `target/release/mccl` linked to the normal native build.

The profiling executable passed all 456 GPU correctness cases. Loader diagnostics
confirmed it loads `build/profile` libraries. Recording it with Metal System Trace
still reported the log-archive error; symbols do not by themselves enable GPU
counters. The resulting trace metadata still reported `Counter Set: (null)` and
`Shader Timeline: Disabled`. Source attribution, recording health, and counter
configuration must be checked separately.

## Sorted-array API and semantics

```rust
use mccl::Gpu;

let gpu = Gpu::new()?;
let a = gpu.upload(&[1, 3, 5])?;
let b = gpu.upload(&[2, 4, 6])?;
let merged = gpu.merge(&a, &b)?;
assert_eq!(merged.as_slice()?, &[1, 2, 3, 4, 5, 6]);
# Ok::<(), mccl::Error>(())
```

The C entry points in `include/mccl.h` are `mccl_merge_sorted_u32`,
`mccl_difference_sorted_u32`, `mccl_copy_u32`, and `mccl_last_error`. They accept
official mlx-c handles and build lazy graphs. `mlx_array_eval` waits for the
output and its dependencies before host access; use stream synchronization
explicitly for a whole-stream barrier. Native graph-construction errors return
nonzero and preserve the previous
output; evaluation errors follow mlx-c's error-handler mechanism.

- Inputs must be sorted ascending; this is not checked by scanning the host.
- Merge is A-before-B for equal keys. Key-only results cannot expose equal-key
  provenance; stability follows the asymmetric co-rank and left-first merge rule.
- Empty inputs, unequal lengths, duplicate keys, and `UINT_MAX` are supported.
- Merge's combined length must fit `INT_MAX`; difference limits each input to `INT_MAX`. Only GPU streams and 1D u32 arrays are accepted.
- C inputs may be strided; MLX makes them contiguous, potentially adding a copy.
- Rust uploads copy host values once into MLX-owned shared storage. They do not
  force a second GPU copy or hide a GPU warmup. Merge and copy remain lazy.
  MLX retains graph dependencies and in-flight buffers; arrays retain their
  thread-affine Rust context.
- `U32Array::eval()` waits for that array's data/dependencies, not unrelated work
  on its stream. `Gpu::synchronize()` is the explicit whole-stream barrier and
  does not evaluate unsubmitted lazy graphs.
- `U32Array::as_slice()` waits, then borrows immutable CPU-accessible data without
  allocating a host copy. The borrow cannot outlive the array. `to_vec()` remains
  the explicit owned-copy convenience. Later operations never donate/overwrite
  a still-live input or result.
- Unsorted inputs have unspecified results but bounded kernel accesses. They are
  not an alternative sorting operation.
- `Gpu`/`U32Array` are neither Send nor Sync. Rust installs a process-wide mlx-c
  error handler; callers sharing mlx-c must account for that handler ownership.
- CPU execution, other dtypes, payload merging, autodiff, and vectorized transforms
  are not implemented by this operation.

## Sorted multiset difference

`gpu.difference(&a, &b)?` subtracts multiplicities: each occurrence in B cancels
the earliest equal occurrence in A. For `[1,1,1,2,4] - [1,1,3]`, the result is
`[1,2,4]`. This is not a membership/anti-join filter.

The returned `Difference` owns private storage of capacity `a.len()` plus a
device-resident uint64 count. `capacity()` is metadata-only; `eval()` completes
the storage/count graphs; `count()` and `to_vec()` wait for those arrays and
read back. They do not impose a whole-stream barrier. `to_vec()` reads only the
initialized survivor prefix, never the tail. No host count read is needed to
construct or execute the GPU graph.

Implementation: three MLX lower/upper-bound searches determine occurrence rank
and B multiplicity, followed by a binary keep mask, exclusive scan, count reduction,
and stable Metal scatter. Empty inputs are handled explicitly. This implementation
is a correct multi-stage baseline, not a claimed optimal tiled set algorithm.

```sh
cargo run --release -- difference-bench --sizes 64K,1M --reps 20 --warmup 5
```

Sizes denote combined A+B length. CSV reports each input length, survivor count,
and completed-call latency; no single-pass bandwidth formula is applied.
The initial release run passed 100 difference cases and all 456 merge cases with
Metal Shader Validation enabled. At 524,288 elements per input, the initial
20-sample median was approximately 0.74–0.93 ms across disjoint, interleaved, and
equal-multiplicity duplicate inputs. These are wall timings, not GPU-counter
measurements; no memory-boundness conclusion is established for difference.


## Blocked Bloom filter

```rust
let mut filter = gpu.bloom(65_536, 0)?; // block count, xxHash64 seed
let keys = gpu.upload(&[1, 2, 3, u32::MAX])?;
filter.add(&keys)?;
assert_eq!(filter.contains(&keys)?.to_vec()?, [1, 1, 1, 1]);
filter.clear()?;
assert_eq!(filter.contains(&keys)?.to_vec()?, [0, 0, 0, 0]);
```

This implements RCCL's eight-word blocked policy for **four-byte little-endian
u32 keys**: seeded xxHash64, multiply-shift block selection, and one salted bit
in each of eight uint32 words. Storage is `num_blocks * 32` bytes. All u32 keys
are valid, including zero and `UINT_MAX`; inputs need not be sorted.

`BloomFilter` provides `add`, `contains`, `clear`, `words`, `num_blocks`, and
`seed`. Construction, mutation, and queries complete before returning. Insert
uses device atomic OR in place, with no full-filter copy. Queries return independent
completed u32 flag arrays, unaffected by subsequent mutation or filter destruction.
`words()` is an explicit host snapshot for inspection. The filter retains its GPU
context and is neither Clone, Send, nor Sync. Cross-context keys are rejected.

Zero means absent; one means **possibly present**, never exact membership.
Completed insertion has no false negatives until clear, but false positives are
expected. There is no per-key erase, u64-key API, automatic FPR sizing, concurrent
insert/query contract in the Bloom API. Block count must be
positive with `num_blocks * 8 <= INT_MAX`. GPU mutation errors are not transactional.

The C ABI exposes `mccl_bloom_create/free/clear/add/contains/read_words`; see
`include/mccl.h` for ownership and error requirements. Direct encoding uses
MLX's stream encoder and its commit/wait/error-check synchronization path.

```sh
cargo run --release -- bloom-check
cargo run --release -- bloom-bench --keys 1M --queries 2M \
  --blocks 65536 --seed 0 --reps 30 --warmup 5
```

Verification: 84 Bloom cases passed with Metal Shader Validation, including
exact CPU/GPU words, contention, duplicate inserts, incremental additions,
clear, empty/extreme keys, seeds, snapshot isolation and lifetime/context checks.
Six additional GPU bit-layout fixtures matched independent Python `xxhash==3.6.0`
results, including strided C inputs. Existing merge/difference regressions passed.

Initial data is in `benchmarks/bloom-filter-2026-10-15.csv`. Inserting 1,048,576
distinct keys into 65,536 blocks (2 MiB), then querying 1,048,576 members and
1,048,576 known nonmembers, produced 1,382 false positives (0.1318% of nonmembers)
and no false negatives. GPU bits and query flags matched the CPU oracle.
The observed rate is empirical, not a bound on arbitrary data.
Clear/insert/query medians were 0.604/0.732/0.762 ms. These include CPU submission
and completion; timing variability was substantial. No GPU bottleneck claim is
made from these wall timings.

## Ordered-bulk hash map

```rust
let mut map = gpu.hash_map(1024, 0)?; // exact slot capacity, xxHash32 seed
let keys = gpu.upload(&[7, 8, 7])?;
let values = gpu.upload(&[10, 20, 30])?;
let report = map.insert(&keys, &values)?;
assert_eq!(report.inserted, 2);
let lookup = map.find(&gpu.upload(&[7, 9])?)?;
assert_eq!(lookup.values.to_vec()?, [30, 0]);
assert_eq!(lookup.found.to_vec()?, [1, 0]);
```

The `HashMap` provides insert/update, find, erase, clear, rehash, capacity, len,
is_empty and seed. Every operation completes before returning. Query results are
independent snapshots. Within an admitted duplicate-key batch, the last input
value wins. Values are arbitrary u32, including zero and MAX; explicit found
flags distinguish absent entries. Keys MAX and MAX-1 are reserved for empty and
tombstone states. Insert reports unique newly inserted entries, per-input full
failures and per-input invalid keys; full or invalid inputs can partially succeed.
Which distinct new keys fit an oversubscribed batch is unspecified.

The Metal implementation uses seeded xxHash32, modulo exact capacity and linear
probing. It claims keys with 32-bit CAS, selects winning input ordinals with atomic
maximum, then publishes values in a separate ordered dispatch. No 64-bit CAS or
simultaneous lookup/publication is assumed. Weak-CAS retry only handles spurious
failure; no thread spins waiting for another thread's value publication.

Deletion leaves counted tombstones. With a small tombstone population, insertion
first locates existing keys in a read-only pass, then lets absent keys claim empty
or tombstone slots. This preserves uniqueness even when an existing key lies
beyond a hole. Maintenance rebuilds only if tombstones reach `max(1,capacity/16)`
or live+tombstone occupancy reaches `ceil(15*capacity/16)`. Empty tables clear
existing key storage in place. These are conservative maintenance thresholds.

Winner storage is initialized once and remains zero between completed insertions:
publication atomically clears each winning ordinal. Losing duplicates atomically
observe either the winner or zero and cannot publish. Only four small counters
are reset per call. Sparse batches (<= capacity/8) publish via recorded input slots;
dense batches scan winners by capacity. Slot scratch also supports tombstone
prelookup. Storage is three capacity-sized u32 arrays, counters, and reusable
scratch sized for the largest batch needing slots.

Lookup uses cached Metal function-constant specializations. When divisible by 16,
tables at >=75% live+tombstone occupancy use 16-slot aligned probes. Otherwise,
capacity divisibility selects 8-slot, 4-slot, or scalar probes. Masks preserve
scalar first-key-or-empty semantics across hash offsets, tombstones and wraparound.
These policies were measured on the M5 Pro, not tuned universally. Rehash preserves
entries and rejects capacity below live size. GPU mutation failure is
nontransactional: discard the map afterward. Handles remain thread-affine.

```sh
cargo run --release -- hashmap-check
cargo run --release -- hashmap-bench --keys 64K --queries 64K \
  --capacity 128K --reps 30 --warmup 5
```

GPU checks passed under Metal Shader Validation against a CPU map oracle,
including collisions, duplicate updates, full-map updates, partial admission,
reserved keys, all value bits, erase/reinsert, rehash, snapshots and lifetimes.
The initial 50%-load benchmark is in `benchmarks/hash-map-2026-10-16.csv`:
insert/find/erase medians 0.587/0.259/0.298 ms. Input arrays are resident;
each insertion follows erasing all keys, so tombstone maintenance is included.
Queries alternate hits with a repeated missing key; this is not a broad
random-miss workload. Timings include host submission/completion, not just GPU
time. No saturation or throughput-optimality claim is made.

### Refinement measurements (2026-10-18)

`hashmap-bench` separates `cold-insert`, `update`, `churn`, `steady-update`, and
`persistent-churn` scenarios, with optional `--batch` (defaults to `--keys`).
The first three restore their starting state per sample. Steady update populates
once; persistent churn alternates preuploaded original/replacement batches while
maintaining the live count. Lookup tests cover uniform random hits, varied
provably absent misses, and shuffled 50/50 mixtures. Reports and final entries
are checked against the CPU oracle outside timing.

```sh
cargo run --release -- hashmap-bench --scenario update \
  --capacity 16M --keys 1M --batch 1K --queries 64K --reps 40 --warmup 10
```

The all-input-sized publication experiment regressed the initial dense-insert
measurement, so it was replaced by the sparse/dense policy above. Complete
exploratory and final workload data is in `hash-map-refinement-2026-10-18.csv`
under `benchmarks/`. Smaller-table results varied substantially; no broad speedup
is inferred from those measurements.

`benchmarks/hash-map-sparse-2026-10-18.csv` records reversed-order comparisons
against the preserved old native binary: 16,777,216 slots, 1,048,576 live keys,
1,024-key updates, 10 warmups and 40 repetitions. Baseline/refined medians were
788.520/452.396 microseconds, then 790.395/451.938 microseconds with order reversed:
about 43% lower completed-call latency for this sparse workload. These are CPU
wall timings, not hardware-counter results. That earlier publication-only version
still reset winners by capacity; the later refinement below removes that reset.
Regression coverage includes exact publication-threshold boundaries, duplicate
winners, scratch reuse, and alternating sparse/dense batches.

Further results are preserved in `benchmarks/hash-map-steady-2026-10-18.csv`:
a steady 1,024-key update in a 16M-slot table changed from 438.875 to 190.812 us
after winner cleanup and vector lookup. At 90% load in a 1M-slot table, random
miss lookup changed from 580.730 to 348.208 us (completed-call wall time).
`hash-map-vector-probe-2026-10-18.txt` isolates the lookup kernels using GPU
timestamps and identical valid tables: at 90% load, random misses changed from
219.813 to 100.500 us. These are workload-specific results, not peak claims.

`hash-map-churn-2026-10-18.csv` compares persistent 1K-key replacement cycles in
a 1M-slot table. Insert medians changed from 625.854 to 249.584 us at 50% load,
and 1042.208 to 529.375 us at 90% load after duplicate-safe tombstone reuse.
Other timings varied between runs, and tombstones can still lengthen lookups.

An experiment aggregating insertion report counters within SIMD groups did not
show a consistent improvement at high load and was rejected. Its raw results
are in `hash-map-statistics-2026-10-18.csv`; production retains per-key report
atomics. Kernel changes are checked against the full CPU oracle suite and Metal
Shader Validation, including partial SIMD groups, full maps, duplicate updates,
cross-batch winner reuse, wrapping collisions and maintenance transitions.

The 8/16/32-slot probe experiments are retained in
`benchmarks/hash-map-vector{8,16,32}-2026-10-18.txt`. Wider probes continue to help
high-load lookup, but 32-slot probes regress low-load workloads; production stops
at 16. The final public-API comparison is `hash-map-final-2026-10-18.csv`.
Against the preserved prior-refinement native binary, median completed-call times
changed as follows (40 samples,10 warmups):

| Workload | Before (us) | Final (us) |
|---|---:|---:|
| 1K existing-key updates,16M slots,1M live |505.312|188.958|
| 64K random misses,1M slots,90% live |366.854|216.938|
| 64K mixed queries,1M slots,90% live |326.646|208.042|
| 1K replacement insert after erase,1M slots,50% live |436.500|197.396|

The first improvement mostly removes avoidable reset traffic; the second reduces
dependent probe steps; the fourth avoids rebuilding the whole live table for
each small deletion. No public semantics or full-capacity admission were weakened.

`hash-map-counters-2026-10-18.json` records an intermediate vector4-stage trace,
not the final wider-probe binary. It includes multiple benchmark operations and
device-wide samples. A separate isolated capture timed out; the successful trace
still reported the known logging error. It does not prove final hardware saturation.
The tested synchronous workloads approach a roughly 0.16–0.20 ms completed-call
floor, but high-load/adversarial probes, bulk insertion, and different hardware
can have headroom. This is a measured local tuning result, not a universal optimum.



## Completed pair-index API

```rust
use mccl::{Gpu, relation::{IndexSpec, Payload}};

let gpu = Gpu::new()?;
// The loader/generator writes directly into shared relation storage once.
let source = gpu.relation_u32::<2>(4, |row| [(row % 2) as u32, 10 - row as u32])?;
let by_first = gpu.build_index(&source, IndexSpec {
    key_column: 0, payload: Payload::RowId,
}, None)?;
let by_second = gpu.build_index(&source, IndexSpec {
    key_column: 1, payload: Payload::RowId,
}, None)?;
assert_eq!(by_first.lookup(&1).collect::<Vec<_>>(), [1, 3]);
assert_eq!(by_second.lookup(&8).collect::<Vec<_>>(), [2]);
# Ok::<(), mccl::Error>(())
```

- `build_index`, `build_index_on_cpu`, `build_index_on_cpu_radix` and
  `merge_indexes` return completed immutable `index::Index` values. Unlike the
  key-only lazy API, publication waits for both sorted rows and their directory.
- Constructors take `&Relation`, `IndexSpec` and an optional selection relation;
  the old host-tuple-slice constructors are removed, not retained as wrappers.
- `relation_u32`/`relation_i64` populate immutable row-major shared storage once.
  There is no intermediate vector. Loading from an existing host collection is
  explicit ingestion, not free; a loader can instead write directly from the
  beginning. `relation_from_u32` retains/reshapes an existing contiguous
  `U32Array`, including lazy GPU-produced input, without evaluating or copying it.
- GPU construction does not call CPU data views or scan/pack source rows.
  GPU kernels extract columns, generate original source row IDs, stably compact
  an optional nonzero u32 `[N,1]` selection, sort and build the directory in one
  dependency graph. CPU publication reads only scalar count/error metadata.
- Source fields may be full-width i64. Only selected index key/payload fields
  must fit u32; GPU validation rejects violations. Unselected invalid fields and
  unrelated wide/signed payload columns are preserved, never silently narrowed.
- Explicit CPU comparison/radix baselines read the same shared source and write
  their own outputs. A lazy GPU input may first need evaluation for CPU access;
  there is no implicit CPU fallback in the GPU builder.
- GPU sort uses four stable byte-radix passes. Merge preserves older-before-newer
  equal-key order. Identical pairs remain distinct occurrences. Keys and payloads
  are full-width u32; row counts are limited to `1 << 30` and device capacity.
- `rows()` and `directory()` borrow ready shared storage. Exact radix-12 lookup
  uses the 4,097-entry directory and returns a Copy, exact-size, fused cursor.
  Reading neither allocates, calls FFI, evaluates a graph nor retains an owner.
- Index clones pin immutable allocations through rebuilds, cache clears and
  context disposal. Merging requires the originating `Gpu` or its clones;
  independently created contexts are rejected even on the same device.
- MLX owns the inactive-buffer cache, encoder hazards and in-flight scratch.
  No additional workspace, pool, command queue or implicit wired budget exists.
- Kernel selection checks actual compiled pipeline resource requirements.
  Sort prefers eight rows per thread and merge four; smaller four/two-row tiles
  are selected when needed. This also accommodates Shader Validation's enlarged
  threadgroup storage without disabling validation or changing bag semantics.

The C ABI exposes `mccl_allocate_u32`/`mccl_allocate_i64` for exclusive source
ingestion and `mccl_relation_view_u32` for a lazy retained view. The synchronous
`mccl_build_relation_index_u32` publishes ready `[selected_count,2]` rows and a
4,097-entry directory, preserving both previous outputs and count on failure.
Private capacity tails are not published. The buffer-level sort/merge/directory
primitives remain lazy and useful for already-packed GPU arrays.
Low-level allocation returns **uninitialized storage**: callers must initialize
it exclusively before sharing/evaluating it. See `include/mccl.h`.

Shared-source verification on M3 Max/macOS 27.0.1 passed ten MCCL integration
tests and twelve CQ tests. All eight MCCL index regressions also passed with
Metal Shader Validation, including repeated columns, original row IDs after
mask compaction, selected signed-domain errors, lazy GPU-produced inputs and
source/index survival across cache clearing and context disposal. Actual hybrid
maintenance and repeated H03/H09 execution passed with validation enabled.
A direct C smoke verified error-atomic publication and retained lazy sources.
See the [shared-source cutover](https://github.com/StarGazerM/mini_linq/blob/codex/all-answers/doc/MAC-IMPLEMENTATION.md#shared-relation-input-cutover)
for query boundaries and measurements. This pair-index remains a sorted-run/
prefix-directory index; the separate grouped RadixRun path below builds a spline.

## GPU-built grouped RadixRun

`Gpu::build_radix_index(&relation, spec, selection)` returns a completed
`radix::RadixIndex` with distinct u32 keys, u64 group offsets, contiguous u32
payloads, u64 key/rank spline points and an adaptive radix-12 directory.
`Gpu::merge_radix_indexes(older, newer)` merges these grouped arrays directly,
preserving older-before-newer occurrence order. Final indexes do not retain a
second packed occurrence array.

Construction reuses shared-source projection/selection and stable GPU radix
sorting. GPU boundary flags and scans build groups; independent 256-distinct-key
chunks build error32 spline corridors with exact integer arithmetic. Chunk
endpoints are retained, then exactly collinear points are removed on GPU before
directory construction. There is no CPU grouping or spline finishing pass.
Publication reads scalar metadata only, after the complete graph succeeds.

The slice getters expose initialized immutable prefixes without copying or
read-time FFI. MLX retains capacity allocations behind these views: logical
length is not a claim of exactly sized physical allocation. Spline scratch
capacity is bounded by nine points per chunk, rather than one point per row.
The input-domain, selection, context-identity and immutable ownership contracts
are the same as the existing u32 index path; selected i64 values outside u32
are rejected, not truncated. Native 64-bit-key GPU sorting is not added.

MCCL deliberately does not duplicate the CPU spline reader. The CQ adapter
exposes `MacContext::build_radix`, `merge_radix` and `MacRadixRun`, using
`cq_storage::RadixRunView` for the same lookup/expansion implementation as native
`RadixRun<u32,u32>`. Dense membership uses the same lazy bounded bitmap policy.
Existing pair-index APIs remain independently selectable, not aliases.

The C ABI exposes `mccl_build_relation_radix_u32`, `mccl_merge_radix_u32` and
`mccl_radix_index`. All five initialized output handles and scalar fields remain
unchanged on failure. See `include/mccl.h` for layout and completed-input contracts,
and the [implementation notes](https://github.com/StarGazerM/mini_linq/blob/codex/all-answers/doc/MAC-IMPLEMENTATION.md)
for the matched construction/read benchmark.

## Index construction diagnostics

The opt-in native diagnostic delegates to the actual MCCL primitives and retains
their command buffers to read GPU execution and CPU-driver scheduling timestamps:

```sh
cmake --build build/native --target mccl_index_timing --parallel 8
./build/native/mccl_index_timing --sizes 1048576,16777216,67108864 --runs 5 \
  --cache pooled --out /tmp/index-steady.csv
./build/native/mccl_index_timing --sizes 67108864 --runs 5 \
  --cache cleared --out /tmp/index-cleared.csv
./build/native/mccl_index_timing --sizes 67108864 --runs 5 \
  --cache pooled --idle-ms 250 --out /tmp/index-idle.csv
```

Run timing diagnostics without Shader Validation and without concurrent GPU
benchmarks. This target is excluded from the normal build and changes no
production primitive or allocator policy. It relies on the pinned MLX C++
interfaces. Every output row and all directory entries are checked outside
timing. Previous cache limits are restored; no wired limit is changed.

Each size ingests one immutable single-column relation exactly once, recording
source allocation/population and faults separately. Every repetition reuses that
same handle; the GPU generates row IDs. Repeated CSV ingestion fields describe
the one-time event, not work repeated per build. Columns record graph creation,
evaluation/publication, native encoding, CPU-driver scheduling and actual GPU
intervals; there is no per-build CPU population or split-count synchronization.
Only scalar count/error metadata is host-read during construction; validation
reads finished rows and source outside timing. Missing scheduling timestamps
remain empty fields. `--cache cleared` affects unused output/scratch allocations,
not the still-owned relation; ownership does not guarantee physical residency.
Previous CSV files retain their original per-build-population boundaries.

The [large construction investigation](https://github.com/StarGazerM/mini_linq/blob/codex/all-answers/doc/MAC-IMPLEMENTATION.md#2026-10-26-controlled-construction-investigation)
contains the size/distribution/relation matrix, matched raw-Metal controls,
actual scheduling/idle measurements and remaining causal limits.

## Apple execution design

`native/merge.metal` uses 256 threads and 1,024 output keys per tile. Two threads
compute global merge-path boundaries. The group stages contiguous A/B ranges,
then each thread computes a local co-rank and merges up to four keys. A second
threadgroup array transposes the outputs into coalesced stores. There are three
threadgroup-memory barriers and no cross-threadgroup spinning or global atomics.
Declared threadgroup storage is 8,200 bytes, below the measured M5 Pro limit of
32,768 bytes. This is a starting policy, not a claim of optimal Apple tuning.

`native/mccl.cpp` uses MLX allocation, active command encoding, and resource
tracking. Output storage never aliases the inputs. The copy baseline dispatches
an actual read/write kernel, rather than an identity operation MLX can eliminate.

### Allocation ownership and first use

MCCL deliberately uses the pinned MLX allocator rather than a second MCCL pool.
MLX first tries its buffer cache, uses shared Metal storage and owns the lifetime
through array data references and GPU completion handlers. Its Metal resources
are untracked at the Metal flag level because MLX supplies encoder/fence/event
tracking; MCCL must not replace that machinery with independent raw buffers.

The pinned MLX backend has standing residency-set requests, but its wired budget
is **zero by default**. A cached allocation is not proof of GPU residency.
`mccl::memory` exposes official mlx-c controls:

- `stats()` reports active, cached and peak MLX allocation bytes, not process RSS,
  physical residency or an atomic snapshot of concurrent work.
- `clear_cache()` releases inactive cached buffers. Live arrays, pending graph
  dependencies and in-flight GPU resources remain owned by MLX.
- `set_cache_limit(bytes)` returns the previous process-wide cache budget.
  It does not immediately empty the existing cache.
- `set_wired_limit(bytes)` returns the previous process-wide wired budget.
  It is explicit, backend/platform-dependent and does not guarantee that every
  allocation is wired.

These controls affect **all MLX clients in the process**. Coordinate policy
changes with other users/streams and restore previous settings when appropriate.
No constructor silently changes the cache or wired limits.

### Completion-scope migration

The old Rust `upload` forced an extra GPU copy after the MLX host-data constructor
had already populated shared storage. The old `eval` also performed a whole-stream
barrier after MLX's synchronous array evaluation. Both extra operations have been
removed. Call `gpu.copy(&array)?.eval()?` when an actual GPU copy is wanted, or
`gpu.synchronize()?` when code requires whole-stream completion. Existing
`bench` output now labels its timing `cpu_wall_array_ready_call`; the older
M5 Pro CSV retains the previous timing boundary and is not rewritten.

### Lifecycle diagnostic

```sh
cargo run --release --manifest-path experiments/mac/mccl/Cargo.toml -- lifecycle --sizes 64K,1M,16M,64M --reps 9 --warmup 2 \
  --wired 0 --out experiments/mac/mccl/benchmarks/lifecycle-rerun.json
cargo run --release --manifest-path experiments/mac/mccl/Cargo.toml -- lifecycle --sizes 1M,16M --reps 9 --warmup 2 \
  --wired 1G --out experiments/mac/mccl/benchmarks/lifecycle-wired-rerun.json
cargo run --release --manifest-path experiments/mac/mccl/Cargo.toml -- lifecycle --sizes 1M --reps 5 --warmup 2 --sync \
  --out experiments/mac/mccl/benchmarks/lifecycle-sync-rerun.json
```

The six cases compare retained GPU-touched inputs with fresh outputs, and whole
rounds using either direct shared upload or explicit extra GPU-copy preparation;
each has cache-cleared and pooled variants. First invocations, warmups and raw
measured samples are separate. Input splits are unequal and all four existing
key distributions are checked.

Each sample records CPU-wall graph construction, array-ready evaluation/wait,
optional separately timed whole-stream synchronization, CPU first-read scan,
input preparation, releases, MLX allocator accounting and process minor/major
fault deltas. Full exact comparisons occur outside operation timers. Cache clears
occur at recorded synchronized boundaries and free only inactive allocations;
“cache-cleared” does not mean a cold device or guaranteed cold physical pages.
The checksum scan is not a pure bandwidth measurement. No GPU timestamp or DRAM
counter claim is made.

The standalone diagnostic bounds its cache budget to four largest output
buffers and uses only the explicitly requested wired budget. Previous limits
are restored on success and attempted on error paths, with failures reported.
Peak memory is MLX's process high-water mark and is not reset. The extra-copy
control reproduces the extra data movement, not every barrier in the old wrapper.


## Verification

On 2026-10-15, the release build and the GPU integration test passed. The actual
CLI passed 456 merge cases and their forced copies, graph-lifetime checks, and
cross-context rejection with Metal Shader Validation enabled, launched from `/tmp`.
A separate direct C ABI smoke covered strided input, wrong dtype/shape/CPU stream,
output preservation on error, and an unsorted-input bounds exercise. That smoke
is not part of the permanent Rust test suite.

```sh
MTL_SHADER_VALIDATION=1 MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1 \
MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1 ./target/release/mccl check
```

Do not enable validation during performance measurement.

## Initial bandwidth investigation

Raw results: `benchmarks/m5-pro-2026-10-15.csv`.
Machine: M5 Pro, 16 GPU cores, 24 GB unified memory; macOS 27.0.1, Xcode 27.0.
The desktop was active on a 7680x2160 display; this was not a headless/isolated
hardware-counter experiment. Dependency build and validation finished before timing.

Sizes are total output keys; K/M suffixes mean powers of 1024. Inputs are equal
length in this benchmark. Each case has 10 warmups and 50 measured repetitions,
alternating merge/copy order. These historical measurements used the previous
Rust wrapper: each repetition constructed a fresh output graph and waited for
evaluation plus stream completion. Upload, readback and output destruction were
outside timing. MLX's warmed allocator pool was already included/reused.

The CSV records median/p10/p90 CPU-wall completed-call latency. Effective bandwidth
is `8*N / elapsed`: N u32 keys read and N written. Merge also reads global partition
probes and uses threadgroup memory; these are not included in that logical-byte
count. These are NOT GPU timestamp durations or measured DRAM bandwidth.

At 33,554,432 keys:

| Distribution | Merge median (ms) | Copy median (ms) | Merge logical GB/s | Copy logical GB/s |
|---|---:|---:|---:|---:|
| Disjoint | 2.159 | 1.214 | 124.351 | 221.158 |
| Reverse disjoint | 1.835 | 1.170 | 146.255 | 229.510 |
| Interleaved | 1.973 | 1.189 | 136.024 | 225.774 |
| Duplicate-heavy | 1.955 | 1.167 | 137.310 | 230.001 |

Interpretation: large merges reach about 56–64% of the matched copy throughput.
The data do not establish DRAM-bandwidth saturation. They leave substantial
kernel-specific overhead (partition searches, dependent accesses, threadgroup
traffic/barriers, occupancy/resource limits) to distinguish through GPU profiling.
The ~0.15 ms small-input floor is dominated by completed-call overhead, not bulk
memory movement. Copy throughput is a measured comparison point, not a theoretical
hardware maximum. Earlier Instruments recording attempts had logging errors;
no clean hardware-counter capture is claimed here.

## 2026-10-25 lifecycle integration (M3 Max)

The first-touch/reuse investigation is now implemented in MCCL without replacing
MLX's allocation ownership. The release build passed both Rust integration tests,
the CLI's 456 merge/copy cases with Metal Shader Validation, retained borrowed-read
checks and cross-context rejection. A separate direct C ABI smoke verified
array-ready reads without an extra stream barrier, dropped-input graph retention,
cache clearing and output preservation on graph-construction errors.
Rust formatting and `cargo clippy --release --locked --all-targets -- -D warnings`
passed. The local native link emitted a macOS 27.0 versus MLX 27.0.1 deployment
target warning; these artifacts were exercised on macOS 27.0.1, not an older OS.

Measurements were collected on an Apple M3 Max, 40 GPU cores, 128 GiB unified
memory, macOS 27.0.1. They are not a remeasurement on the historical M5 Pro.
Raw files and scope are listed in [benchmarks/README.md](benchmarks/README.md).

For 67,108,864 interleaved output keys with unequal input lengths, zero explicit
wired budget, and nine warmed samples per operation:

| Output allocation policy | Merge array-ready eval (ms) | Copy array-ready eval (ms) | First CPU merge-result scan (ms) | CPU read minor faults |
|---|---:|---:|---:|---:|
| Clear inactive cache before each sample | 13.358 | 8.794 | 19.444 | 16,384 |
| Keep MLX allocator pool | 4.876 | 2.172 | 3.876 | 0 |

These are CPU-wall phase medians, not GPU timestamps. The output is 256 MiB,
and this machine reports 16 KiB CPU pages. GPU-produced data can still incur
first CPU mapping faults: these counters alone do not classify them as zero-fill
or GPU faults. The allocator accounting also showed the output moving from
cached to active bytes during evaluation and back to cached bytes after release;
live retained outputs stayed exact while new operations ran.

For the same large interleaved merge, pooled direct input preparation took
4.511 ms. The extra-copy control spent 4.510 ms on the same CPU preparation plus
2.596 ms in additional GPU copies/array evaluation. These are separate marginal
phase medians, not an asserted sum of end-to-end medians. Removing those copies
also avoids their temporary allocations and pool pressure.

The separate `--sync` diagnostic measured a 39.000 us median additional stream
barrier after an already-ready 1M-key pooled merge. It is available when needed,
but no longer forced by every array read. Explicit 1 GiB wired-budget runs passed
and restored their previous settings; they did not establish a consistent warmed
benefit, so no wired budget is enabled by default.

All first invocations, warmups, measured samples, allocator snapshots and fault
deltas are preserved. Case order, GPU performance-state history, normal OS
contention and the pooling size pattern can affect the measurements. This is
evidence for using the existing pool and avoiding redundant preparation—not a
universal crossover threshold or a claim that merge saturates DRAM bandwidth.
