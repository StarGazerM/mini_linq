# Experimental GPU Sequence backend

The main workspace uses checked Sequence programs, resident compressed columns,
and explicit GPU grading. `cq-gpu-sequence` exposes `machine()` and the direct
`translate(&Program, &Analysis)` API alongside the pipeline macros
`machine_grade!` and `translate!`. The grading macro checks the authored grades
against `machine()`; it does not assign placements or choose a plan.

A GPU pipeline explicitly selects `sequence = ::mini_linq::passes::sequence`,
records its decomposition with `decompose = [...]`, then selects
`grade = ::cq_gpu_sequence::machine_grade` and
`print = ::cq_gpu_sequence::translate`. Sequence rewrites carry decisions:
`store_level` places storage, `tile` records tile size and items, `choose`
records guarded alternatives, `output` records publication protocol/capacity,
and `grade` records machine placements. Canonicalization remains a Sequence
`canon` term with its recorded implementation. Translator rejection is not a
license to substitute another query or plan.

The translator must decide nothing (T-Gpu, `doc/formal/backend_ir.tex`
§"Refinement and the emitter"): kernels, barriers and launch shapes come from
the grading and recorded analysis. `emit.rs` still decides grid size (1 or
the literal 2272), deferred lookup ranges, one-block offset scans and fold
finalization, and per-dispatch host synchronization; these are listed as open
violations in `doc/SEQUENCE-PLAN.md` [6]. Do not add further translator-side
decisions; record them in grading first.

Rust `if`/`let` expressions remain opaque, total Rust. Device execution requires
identity representation for every column read; codec-backed reads execute on the
host. Device Rust uses cuda-oxide-specialized callables with query-closed inputs
captured on the host, without arithmetic recognition or wrapping rewrites.
Only `where` predicates compare encoded columns: closed operands use the recorded
column codec once on the host. Provider traits supply scalar widths and bounds.
Computed bindings are lane-local values; sibling lanes may read parent-owned
values through their owner index, but maps never write shared parent slots.
Resident inputs retain their owning pool. A query whose inputs are entirely host
values acquires the shared device-0 thread pool for its recorded GPU publications.

## Build and run

The workspace pins the pushed `mini-linq/main` branches of the StarGazerM
cuda-oxide and RCCL forks by commit, without local dependency patches.
The declared source of truth is cuda-oxide `299eda41b` and RCCL `1d33079a`;
the local `/data/cuda-oxide` and `/data/rccl` checkouts match those pushed
revisions. Do not add path patches: update the git pins and lockfile together.
It uses cuda-oxide's Rust **1.99.1** SIMT toolchain, with `RUSTC_BOOTSTRAP=1`
configured for the compiler backend. Build the backend and install `cargo-oxide`
from that pinned fork's nested `cuda-oxide/` workspace; the backend path is
configured in `.cargo/cuda-oxide.toml`.
The monorepo keeps host crates at its root and device/codegen crates under
`cuda-oxide/crates`. Rebuild the configured release backend from the pinned
checkout with `RUSTC_BOOTSTRAP=1 cargo build --release --lib --target-dir target`
in `cuda-oxide/crates/rustc-codegen-cuda`.

The shared host bindings require CUDA 13.0 or newer. With the HPC SDK layout,
bindgen also needs the separate cuRAND headers under `math_libs`.
From this directory, with that backend:

```sh
export CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/cuda/13.0
export BINDGEN_EXTRA_CLANG_ARGS=-I/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/math_libs/13.0/targets/x86_64-linux/include
cargo oxide build --arch sm_89 -- --release --workspace
cargo oxide test --arch sm_89 -- --release --workspace
```

Release build dependencies also use optimization level 3: the build-time
compression catalog shares CUDA-containing crates with the runtime, and their
device address-space lowering requires optimized codegen.

The workspace's `.cargo/cuda-oxide.toml` selects `sm_89` and enables
`CUDA_OXIDE_MATERIALIZE_CUBIN` for the **cargo-oxide wrapper**. This is equivalent
to passing `--materialize-cubin --arch sm_89`: ordinary device code keeps the
LLVM optimization/PTX pipeline, and provenance-checked `ptxas` assembles its
native cubin during the build. Materialization does not force the different
libNVVM optimizer. Artifacts that require NVVM IR retain their libNVVM/nvJitLink
route. At the final host link, retained local/dependency inputs produce
`__cuda_oxide_linked` for cross-crate generic kernels (including RCCL), using
the same PTX merge semantics as the nonmaterialized loader. Compile-only inputs
live in unanchored, linker-excluded archive members; they are not embedded in
the final runtime image or parsed by module loading.
`cuda-host::load_bundle` selects each ordinary module's cubin; the generic loader
selects the linked cubin instead of merging PTX at runtime. Keep invoking
`cargo oxide`, not raw `cargo`; the wrapper fingerprints the compiler/toolkit
inputs and supplies the required
materializer handshake. Do not set the internal materialization variable around
raw Cargo. Cubins target the configured GPU architecture rather than providing
the runtime PTX bridge to other architectures; rebuild with the appropriate
`--arch` when changing GPUs.

### Packed Datalog runtime

`gpu-datalog-runtime` provides `Packing`/`Column` encodings and `DeviceRuns<W>`
on the resident-column `Pool`. The narrowest word is u64, u128, or `[u64; L]`
for larger schemas. Array lanes are most-significant first; native u128 keeps
its native in-memory lane order. Integer columns use their full declared width;
signed columns flip the sign bit, and the first tuple column occupies the most
significant packed bits. `prefix_range` returns inclusive bounds, including the
maximum word without overflow.

`DeviceRuns::admit` sorts, deduplicates, subtracts all accepted runs on-device,
and returns a cheap immutable `Run` delta while accepting it internally.
It reads back only the unique count and fresh count (empty batches and an empty
accepted state omit unnecessary readbacks). `DeviceRuns::new(pool, ratio)`
stores the supplied maintenance ratio (at least two); merges retain sorted,
unique, pairwise-disjoint runs. For another layout, permute the delta with
`Packing::permute_device`, sort with unsafe `DeviceRuns::sort_known_unique`
(the column permutation preserves uniqueness and cardinality), and use unsafe
`append_sorted` after proving disjointness. This path needs no unique-count
readback or compaction; arbitrary inputs still use `sort_unique`. `RunView` and device
`runs::range` support inclusive prefix searches without materialization.
All work and frees use the pool's legacy default stream.
Packed permutation, wide-word radix gather, admission membership and wide merge
size their grid-stride launches from the loaded kernel's CUDA occupancy,
capped by the work domain; they do not assume a particular device's grid cap.

```sh
cargo oxide test --arch sm_89 -- --release -p gpu-datalog-runtime
cargo oxide test --arch sm_89 -- --release -p gpu-datalog-runtime --test runs -- --ignored --nocapture
```

The ignored probe admits 10M random u64 candidates against 50M accepted rows.
Timing is opt-in: `enable_timing(true)` records CUDA events, and the explicit
`last_timings()` accessor synchronizes and reports sort/unique, anti-join, and
merge milliseconds. `to_host()` is an explicit diagnostic sorted download,
not part of the admission path.

### Run-bound hole inputs and candidate arenas

A CQ input whose declared host type is `::gpu_datalog_runtime::DeviceLayout<K, V>`
(`DeviceLayout::from_runs(pool, packing, runs)`) is bound as packed device runs
(`GPU-Runs`): scans visit every run, key-prefix lookups binary-search one range
per run with block-uniform expansion, and membership ORs over runs. A recorded
lookup on nonleading columns obtains a sorted, permuted provider view; it retains
each original run's rows and cross-run multiplicity. Unused selection fallbacks do
not construct providers. Scalar widths come from the native `Scalar` provider
trait; codec-declared symbols use u32 codes, with no 128-bit tuple cap.
`cq_gpu_sequence::translate_arena(&mut arena)` (`GPU-Arena`) publishes head rows
packed most-significant-first in head order into a `CandidateArena` and returns
a `CandidateBatch` (u64 lanes, native low-first for u128 and most-significant
first for larger words; length; packing; retained owners). The hole must be
consumer-canonical: the batch is a candidate bag for `DeviceRuns::admit`.
Sizing keeps the dispatch's output protocol, so each attempt reads its
metadata back; candidate rows are never downloaded. Arena invocations omit
the redundant final stream synchronize and phase timing events by default.
`cq_gpu_runtime::enable_arena_timing(true)` opts the current thread into those
events, the final fence, and `take_last_timings()` publication. Required metadata
validation and device overflow/failure checks are unchanged; retained owners
and the pool stream order protect asynchronous work.
The explicitly recorded `output reserve_previous` protocol is supported only by
the arena terminal. Its initial capacity is the arena's last validated row count
(zero on first use or after `clear`), not a proven bound. Each attempt counts all
rows and checks sticky failure flags; an undersized attempt is discarded and
replayed against the unchanged inputs at the required capacity before publication.
Thus shrinking, growing, and empty rounds remain exact, and retained batches
cannot be overwritten by subsequent leases. Other output protocols are unchanged.
Declared run layouts also realize native keyed Host fallbacks: scan and keyed
reader records remain distinct, and lookups reuse the supplied runs without
building another index.

```sh
cargo oxide test --arch sm_89 -- --release -p cq-gpu-sequence --test run_holes
```

### Whole-program GPU Datalog

`cq-gpu-datalog` runs a Datalog program on the device through the ordinary
Datalog pipeline (`CK-DL-DeviceStorage`, `P-DL-Gpu`):

```rust
pipeline {
    storage_rewrites = [::cq_gpu_datalog::device_storage];
    cq = pipeline {
        physical = [::cq_gpu_sequence::capture_constants];
        structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs];
        sequence = ::mini_linq::passes::sequence;
        decompose = [::cq_gpu_sequence::plan];
        grade = ::cq_gpu_sequence::machine_grade;
        print = ::cq_gpu_sequence::translate_arena;
    };
    terminals = [::cq_gpu_sequence::translate_arena => batch];
    round = [::cq_gpu_datalog::bind_round(binding = 5)];
    print = ::cq_gpu_datalog::print_rounds;
}
```

`bind_round` records complete input-layout and arena argument packets before
printing. Its zero-based invocation ordinal names the binding rewrite after
plan, index demands, lower, capture constants, and select in this queue.
Update that ordinal when inserting preceding phases; aliases do not change
ownership. The printer rejects missing owners or modified packets.
`device_storage(ratio(8))` overrides the geometric maintenance ratio; omission
records four. The ratio is printed in Storage and passed to every run state.

Inputs upload once; each round issues the recorded waves in order, admits each
head's candidate batches into its accepted runs, and publishes the delta into
the recorded layouts; outputs download once. All programs on one thread share
`gpu_datalog_runtime::thread_pool()` (device 0), so the runtime's thread-local
caches never pin a finished program's pool.
Candidate batches are released after admission, arenas are scoped to the
producing stratum, and completed strata release cached Sequence scratch and
wholly unused pool chunks. Accepted runs and layouts retain their published
owners; trimming never invalidates live device views.

Symbol columns are declared with
`device_storage(symbols(IStr => MyCodec))`, where
`MyCodec: gpu_datalog_runtime::SymbolCodec<IStr>` supplies a process-fixed,
injective `code(&IStr) -> u32` mapping and its `decode(u32) -> IStr` inverse.
Use the same type spelling as the relation declaration; the compiler does not
resolve Rust aliases. Upload/download performs the conversion on the host.
The declared codec map is a printable Storage field, independent of opaque
provider types; `DeviceRuns<W>` contains no logical-schema generic metadata.
Closed guard, binding and selection expressions are host captures, evaluated
once per invocation on demand for a nonempty dispatch input. Invocation-local
`OnceCell` values retain captures across retries and dispatches; symbols use
the column's codec, and kernels receive only integer parameters.
`capture_constants` is a checked Physical rewrite before provider selection:
literal atom arguments become fresh columns, equality guards, then a projection
that removes those columns. `bind_runs` never changes the Physical program.

Set `GPU_DATALOG_TRACE_ROUNDS=1` to print metadata/count readbacks for each
stratum seed and fixpoint round. This includes candidate dispatch attempts,
admission counts and layout canonicalization counts, not final row downloads.
`GPU_DATALOG_TRACE_MEMORY=1` additionally records pool current/peak allocation,
reserved/cached bytes, per-relation accepted/layout/arena ownership footprints,
and each hole's candidate rows and retry capacities. Layout footprints include
shared backing allocations and must not be summed as physical pool usage.
Native CUDA library scratch outside the pool is not included in pool counters.

For kernel attribution, build with `CQ_GPU_SEQUENCE_DUMP=/existing/directory`.
The Sequence printer writes each checked program under its exact kernel module
name (`*.sequence`), including its providers, scan, expansions, grades and output
protocol. Run with `GPU_DATALOG_TRACE_KERNELS=1` to record layout descriptor
identities and run lengths, input rows/run counts, every dispatch attempt's
capacity and candidate count, and per-relation admission counts. Combine this
with `GPU_DATALOG_TRACE_MEMORY=1` to associate invocations with Round hole IDs.
These diagnostics neither download candidate rows nor change plans; diagnostic
printing is excluded from performance measurements. Admission is shared by all
holes for a head, so admitted rows are attributed to that head/round, not
arbitrarily to an individual hole.

```sh
cargo oxide test --arch sm_89 -- --release -p cq-gpu-datalog --test oracle
GPU_DATALOG_PROBE_NODES=2048 GPU_DATALOG_PROBE_CASES=tc_left,tc_right,same_generation \
  cargo oxide test --arch sm_89 -- --release -p cq-gpu-datalog --test oracle timing -- --ignored --nocapture
```

### DOOP points-to benchmark

`doop-gpu` instantiates `doop_shared::doop_engine!` with the GPU pipeline above.
It uses the shared fact loader and compares all 26 complete output tuple sets
against `doop-mini`; it does not substitute a separate points-to program.
The loader owns the global string dictionary and precomputes method descriptors.
`DoopSymbols` implements `SymbolCodec<IStr>` using that dictionary's stable u32
codes; no string operations run on the GPU.

The benchmark selects the target-independent incremental advice
`incremental = ::cq_datalog::passes::incremental(driver(delta_first););`.
For each recursive variant whose changing occurrence lies in the first
uninterrupted positive prefix, the planner leads with that delta and then
prefers connected positive reads, breaking ties by source position. It records
the complete source-occurrence permutation before selecting storage layouts.
The independent Incremental checker checks bijection, prefix barriers, delta
substitution and exact inverse-permutation erasure; Storage checks keys in
recorded execution order. Rust and negative clauses are never crossed. Seeds
and variants with a delta beyond such a barrier retain written order. This is
explicit program advice, not a GPU-specific Datalog compiler or printer rule;
the default incremental policy remains written order.

DOOP also explicitly selects the checked Sequence
`::cq_sequence_rewrites::output(reserve_previous)` advice after GPU decomposition.
This changes only the recorded terminal capacity protocol; it does not alter
the rule order, dispatch geometry, candidate bag, or admitted tuple sets.

```sh
GPU_DATALOG_TRACE_ROUNDS=1 cargo oxide run --arch sm_89 --bin doop-gpu -- /tmp/doop-data/cassandra
```

Fact loading and CUDA context creation are outside the reported evaluation
time; input upload, device layout construction, fixpoint execution, output
download and symbol decoding are included. Full-set comparison is outside the
timer. Pass `--gpu-only` to skip the CPU reference and exact comparison when
measuring GPU execution alone. Set `GPU_DATALOG_PHASES=1` to report synchronized
setup, fixpoint, and output phase times on stderr; disabled phase marks add no
stream synchronization.

#### Delta-driven rule scheduling

On Cassandra with Rust 1.99.1, CUDA 13.0 and `sm_89` cubins, paired untraced
runs measured **9.602122 s → 3.672471 s** GPU evaluation (2.61×), matching all
26 output sets exactly. Nsight Systems measured summed GPU kernel time
**7.720946 s → 1.895041 s** (4.07×). Profiling is reported separately:
the corresponding instrumented evaluation times were 10.038853 s and 4.484224 s.

The five formerly hottest step kernels all scanned full `reachable` before
their selected delta occurrence. Kernel `_124` (hole 123, cast propagation,
delta `varpointsto`) enumerated compatible types and heaps before its delta
test. Its representative full scan had 2,686 rows: only eleven useful 256-row
outer tiles on 142 SMs, although 2,272 blocks were launched. Descendant work
stayed within those blocks. Nsight Compute measured 15.95 ms, 0.01% DRAM
throughput and 90.71 million executed warp instructions—not a bandwidth-bound
scan. The delta-driven plan replaces reverse heap enumeration with a full-key
heap-allocation check; the representative launch fell to 199.01 µs and
2.94 million instructions, without relaxing exactness or changing tiling.

| Kernel suffix / Round hole | Selected delta / rule | Before kernel sum | After kernel sum |
|---|---|---:|---:|
| `_124` / 123 | `varpointsto` / cast | 4.080474 s | 0.065991 s |
| `_137` / 136 | `varpointsto` / virtual receiver | 0.554450 s | 0.117876 s |
| `_144` / 143 | `varpointsto` / virtual reachability | 0.502598 s | 0.102994 s |
| `_146` / 145 | `varpointsto` / virtual callgraph | 0.463852 s | 0.102617 s |
| `_133` / 132 | `instancefieldpointsto` / field load | 0.416583 s | 0.347589 s |

All 8,370 hole/round candidate counts and head-admission records agreed before
and after; the main component still took 179 step rounds. Full CQs, exact
before/after module names and checked Sequence plans, all per-launch input
run sizes for these five holes, admission counts and full-set evidence are in
[`compare/doop_cassandra.delta_driver.results.json`](compare/doop_cassandra.delta_driver.results.json).
The remaining largest kernel is the instance-field delta variant; this change
does not claim to eliminate its signature-key expansion skew.

#### Earlier measurements

Cassandra validation matched all 26 outputs exactly. An untraced release run
measured GPU evaluation **11.809409 s**, sequential mini **4.269240 s**, versus
the prior 15-thread mini-par reference **1.28 s** (GPU/reference **9.226×**).
These are end-to-end evaluation times, not kernel-only timings.

With the maintained upstream-based fork pins, Rust 1.99.1, and CUDA 13.0,
the Cassandra release validation again matched all 26 full-tuple outputs.
That run measured GPU evaluation **25.274430 s** and sequential mini
**4.176412 s** (GPU/prior mini-par reference **19.745648×**).
This is slower than the preceding local-patch measurement; no speedup is
claimed for the toolchain migration.

The instrumented run recorded a **232,744,960-byte pool allocation peak** and
**528,482,304-byte maximum pool reservation**, excluding native CUDA library
scratch. The main recursive stratum used 179 step rounds, with 53–109 metadata
readbacks per step; all strata totaled 16,553 metadata readbacks. Exact per-round
counts, full output cardinalities and memory snapshots are recorded in
[`compare/doop_cassandra.results.json`](compare/doop_cassandra.results.json).

The initial 7.44-GB allocation failure was a correctness bug, not retained
multi-gigabyte relation state: a dynamic-u128-mask device miscompilation corrupted
permuted keys and inflated the `returnvar` join to 929,428,430 candidates.
Constant lane shifts fix the permutation; the corrected hole emits 33,667
candidates using 269,336 bytes. Runtime differential tests cover that regression
and the explicit wide-row merge path required beyond RCCL's 16-byte key limit.

#### Ahead-of-time cubin loading

The `sm_89` AOT run preserves exact full-tuple results for all 26 Cassandra
outputs. The 13-program Datalog oracle and all fixtures passed, as did the three
Datalog packages (54 tests, two opt-in probes ignored), all 14 `run_holes` tests,
and both nonempty/empty `printer-smoke` cases.

| Measurement | PTX before | AOT cubin after |
| --- | ---: | ---: |
| Cold GPU evaluation | approximately 27 s (prior observation) | 10.463490 s |
| Warm GPU evaluation, unprofiled | 11.857853 s | 10.422113 s |
| Nsight `cuModuleLoadData`, 158 calls | 5.588267898 s | 0.163992571 s |
| Median module load | 35.210215 ms | 0.765312 ms |
| Nsight kernel time, 52,489 launches | 7.731254122 s | 8.793091834 s |
| Executable size | 22,623,200 bytes | 21,204,400 bytes |

The cold cubin run immediately followed `rm -rf ~/.nv/ComputeCache`; warm was
the next execution. The cache remained an empty 4-KiB directory after the
plain, instrumented and profiled runs. Warm end-to-end evaluation fell **12.1%**,
and profiled module-loading time fell **97.1%**. Kernel time increased, so this
is a loading improvement, not a claim of faster kernels. Nsight and preload
instrumentation are separate runs with observer overhead.

Driver interception recorded every image and its loader stack:

| Load IDs | Calls | PTX bytes/image before | Cubin bytes/image after | Loader |
| --- | ---: | ---: | ---: | --- |
| 1 | 1 | 11,210,914 | 4,816,608 | Generic runtime kernel set: merged PTX before, build-time linked cubin after |
| 2–3 | 2 | 2,632,192 | 1,353,888 | Anchored Sequence runtime crate, `load_bundle` |
| 4–158 | 155 | 8,421,855 | 3,360,992 | Per-hole loaders, anchored DOOP crate, `load_bundle` |

The 155 hole loads are byte-identical **crate-wide** images, not separate
per-hole PTX and not the full merged image. Load count was intentionally left
unchanged. Every after-image was an ELF cubin passed to `cuModuleLoadData`;
there were **zero PTX loads**, no other `cuModuleLoad*` variants, and no modules
requiring runtime compilation. Ordinary loads take `load_bundle`'s Cubin branch;
the generic loader selects `__cuda_oxide_linked`. PTX sizes exclude the trailing
NUL; cubin sizes are the file-backed ELF extent.

The maintained cuda-oxide pin adds build-time cross-crate nvJitLink support,
weak-ODR device definitions, fail-closed cubin/IR pairing and regression tests.
Compile-only IR is excluded from final host images rather than copied during
runtime bundle discovery. The CLI remains `--materialize-cubin --arch sm_89`;
the workspace enables that mode by default. No per-crate source attributes or
local dependency patches are required.

Full per-load sizes, hashes, timings, loader paths, fork revisions and
verification results are in
[`compare/doop_cassandra.aot_cubin.results.json`](compare/doop_cassandra.aot_cubin.results.json).
The temporary profiles and intercepted images from that historical measurement
have been removed; the JSON retains their measurements and image identities.

#### Preserving code generation during AOT assembly

The initial AOT mode changed more than module loading: it forced
`CUDA_OXIDE_EMIT_NVVM_IR=1`, replacing LLVM 23's `internalize,default<O2>`/`llc`
PTX pipeline with the legacy libNVVM optimizer and nvJitLink LTO. The resulting
integer/control-flow code was larger. It was not an optimization-level,
debug-information, math-mode, or register-cap flag mismatch.

The exact former PTX was recovered from the retained executable (SHA-256
`e047023e2b4acc3730fc1135dc41d824c7a2fc9dd8c6e06bd24e2146317ac06b`).
CUDA 13 `ptxas -arch=sm_89 -O3` produces instruction-identical SASS to the
driver's cached `cuModuleLoadData` cubin for all five hot kernels:

| Kernel suffix | Registers, PTX / initial AOT | Stack bytes, PTX / initial AOT | SASS instructions, PTX / initial AOT | Dynamic shared bytes | Resident blocks/SM, both |
| --- | ---: | ---: | ---: | ---: | ---: |
| `_124` | 64 / 64 | 16 / 0 | 1,920 / 2,592 | 34,896 | 2 |
| `_137` | 128 / 123 | 24 / 0 | 3,384 / 4,800 | 67,696 | 1 |
| `_144` | 128 / 120 | 8 / 0 | 2,992 / 4,264 | 39,024 | 2 |
| `_146` | 128 / 120 | 8 / 0 | 3,040 / 4,328 | 55,408 | 1 |
| `_133` | 62 / 48 | 0 / 8 | 1,544 / 2,152 | 24,656 | 3 |

Counts include terminal padding NOPs. Static shared memory is zero and block
size is 256 throughout. PTX `_124` and `_137` have 16-byte spill stores/loads;
the initial AOT versions eliminate those spills but are nevertheless slower.
The initial AOT `_133` has 8-byte spill stores and 16-byte spill loads.
The disassembly's `CALL.REL.NOINC` instructions are intra-kernel control flow,
not evidence of newly outlined device helpers. Neither image requests `-G`,
line information, or a register cap.

Nsight Compute on the same `_124` launch (2,272 blocks, fixed 915-MHz SM clock)
measured **19.02 ms** with the initial AOT code versus **15.94 ms** with the
PTX-assembled code. Both use 64 registers and have 33.33% theoretical occupancy,
limited by shared memory. The slower code actually has higher achieved
occupancy, 23.34% versus 18.63%; occupancy loss is not the cause.

Materialization now preserves ordinary PTX code generation and assembles it
ahead of time with provenance-checked `ptxas`. Generic cross-crate PTX uses the
same shared merger as the nonmaterialized loader, but runs during the build.
NVVM/LTOIR inputs retain their required compiler route. Runtime images still
contain only cubins, while compile-only inputs remain linker-excluded.
The rebuilt fixed cubins have instruction-identical SASS and identical resource
usage to the original driver-JIT kernels for every row in the table above.

The rebuilt Cassandra profile again matched all 26 complete output tuple sets:

| Measurement | Original PTX JIT | Initial NVVM AOT | Fixed PTX AOT |
| --- | ---: | ---: | ---: |
| Kernel time, 52,489 launches | 7.731254122 s | 8.793091834 s | **7.722568032 s** |
| Module loading, 158 calls | 5.588267898 s | 0.163992571 s | **0.119942920 s** |
| Warm GPU evaluation, unprofiled | 11.857853 s | 10.422113 s | **9.314324 s** |

The fixed kernel time meets the original 7.73-second target and is 12.17% below
the regressed AOT measurement. `_124` accounts for 4.079973207 s over 306
launches. Profiled end-to-end GPU evaluation is 10.334188 s; the separate
unprofiled warm run is 9.314324 s and also matches all outputs exactly.
The three required Datalog packages pass **54 tests** (two opt-in probes
ignored), and `cq-gpu-sequence --test run_holes` passes **14 tests**.

The published fork revisions are cuda-oxide
`d89093b8149b93c63324c5a0e687a097061f117c` and RCCL
`67f06e4ad723d6fefb7222bbead01ea94a20f29b`. Artifact identities, per-kernel
resource/SASS comparisons, compiler flags, Nsight Compute counters, and
verification results are recorded in
[`compare/doop_cassandra.aot_codegen.results.json`](compare/doop_cassandra.aot_codegen.results.json).

### SRDatalog field-flow DOOP on prepared integer inputs

`srd-doop-gpu` is separate from the 26-output `doop-gpu` benchmark above.
It instantiates `srd_doop_shared::srd_doop_engine!` from
`features/datalog/doop/srd`: the field-flow query has 37 inputs, 85 source
rules, and 43 output relations. Prepared integer IDs and `meta.json` constants
are passed unchanged; there is no symbol codec or alternate GPU query body.
The pipeline uses `device_storage`, `delta_first`, `capture_constants`,
`reserve_previous`, `translate_arena`, and `print_rounds`, like `doop-gpu`.
Each rule keeps its SRDatalog name as a label. Join orders come from the
planner's connected default; SRDatalog's `with_plan` orders are not emitted
unless `translate.py --plans` is used, which turns every `with_plan` into
`plan(Label, [delta(R@k),] order(...))` incremental advice (see the
`cq-datalog` facade docs; a `var_order` becomes a connected atom order, delta
first). Translated as atom-at-a-time orders they measured slower on the delta
versions (batik 9.1 s -> 21.3 s fixpoint). `block_group` and the other kernel
pragmas are not lowered; `generated.schema.json` records `source_plans` and,
with `--plans`, `unlowered_plan_flags`. Build with
`--features default-query` for SRDatalog's default (non-field-flow) program,
82 source rules and 40 outputs, which is what its default benchmark runs.

The translation comes directly from `build_doopdb_program(meta, field_flow=True)`
in SRDatalog `fab849f672d43af2a6a1e8a1727ec065201e3720`; multihead splitting
is the only rule transformation. `generated.schema.json` records the source
SHA-256, relation schemas, and 91 expanded rules. Regenerate from the repository
root with `python3 features/datalog/doop/srd/translate.py --srd-repo PATH --field-flow`.
The CPU engines use that same generated macro and loader:

```sh
cargo test --release --manifest-path ../../features/datalog/doop/srd-engines/Cargo.toml --workspace --features phase-timing
cargo build --release --manifest-path ../../features/datalog/doop/srd-engines/Cargo.toml --workspace --features phase-timing
../../features/target/release/srd-doop-mini PREPARED --export OUTPUT
../../features/target/release/srd-doop-mini-par PREPARED --threads 15
```

The CPU fixture checks all 43 IDBs, including recursive field flow, split
multihead rules, negative method modifiers, duplicate inputs, and unreachable
stores. The optional CPU phase instrumentation reports per-round evaluation,
admission, and publication times; their sum is not the whole evaluation wall time.

From this workspace, with the build environment above:

```sh
cargo oxide build --arch sm_89 -- --release -p srd-doop-gpu
./target/release/srd-doop-gpu \
  /data/doop-bench/doop-vs-srdatalog-20261026/fresh-corpus/prepared/xalan \
  --gpu-only --export /tmp/srd-doop-gpu-xalan
cargo oxide test --arch sm_89 -- --release -p srd-doop-gpu --test exactness -- --test-threads=1
```

Use `prepared/batik` in place of `prepared/xalan` for Batik. The shared harness
accepts `--gpu-only` explicitly; this standalone executable never runs a CPU
reference, even when that flag is omitted. `--export DIRECTORY` writes complete
output relations for external exact tuple comparison; counts alone are not an
exactness check. Evaluation includes upload, GPU execution, and output
materialization, excluding input loading, CUDA context initialization, and
export. `GPU_DATALOG_PHASES=1` enables the GPU setup/fixpoint/output split.
The small shared field-flow fixture test asserts every one of the 43 output
relations against explicit expected tuples and requires CUDA. Run GPU jobs
serially. These commands describe how to build and verify, not recorded results.

`compare/srd_query_benchmark.py` retains provenance, per-process logs, uptime,
and in-run `nvidia-smi` SM-clock samples in the results JSON. From this directory:

```sh
python3 compare/srd_query_benchmark.py build --datasets xalan batik
python3 compare/srd_query_benchmark.py export --datasets xalan batik
python3 compare/srd_query_benchmark.py run
```

Compilation is separate from execution. Timing uses one warmup and three
fresh-process measurements with a hard 10-second limit, baseline SRD plans,
`SRDATALOG_RMM_RESOURCE=cuda_async`, and 15 CPU threads. Correctness exports
have a separate 300-second deadline and never enter timing medians. Export
comparison checks all IDB tuples, rejects duplicates, and validates counts
against each engine's JSON. `--engines` and `--datasets` select subsets, which
accumulate in `compare/srd_query_xalan_batik.results.json`; never run concurrent
writers or GPU engines. The original-query `doop-gpu` timing is explicitly
separate and is not an equality comparison against the SRD query.

### Planner-driven 11-query TPC-H suite

`tpch` is one crate with one binary, `cq-gpu-tpch`. It imports the CPU
`retail_kernel!` invocations directly with `include_str!`; no second query body
is duplicated. Build-time compilation uses the same CQ macro substitution,
checked CQ → Physical → Structures passes, generic catalog selection, Sequence
lowering, grading and translation. The default build generates only `advised`
executors, using explicit pipeline choices followed by decomposition-only
completion. Automatic is opt-in pending box 11b: build with
`CQ_GPU_TPCH_AUTOMATIC=1` to also generate `automatic` executors using
`cq_sequence_plan::plan`. No automatic lookup rewrite is required to compile or
measure the advised baseline.
The manifest loader records provider encoding facts separately from logical Rust
types. Relational-only columns may use manifest scalar or dictionary codes, with
closed `where` operands encoded on the host. Rust arithmetic and conditional binds
are never rewritten into dictionary operations; logical host adapters handle
expressions reading nonidentity columns. Neither loader nor translator chooses a plan.

The default loader reads only each imported query's declared columns. `String` columns
are decoded from `.bin` dictionary codes and `.dict.txt` into owned host strings;
integer/code declarations retain codes, and only their text literals are
resolved to dictionary IDs. Integer-only relations keep resident device uploads
and share columns between aliases. Dates use i32 epoch days and decimals use
scale-2 integers. Substring predicates receive real strings, not dictionary IDs.
This affects H03 (`c_mktsegment`), H05 (`r_name`), H07/H07-block (`n_name`),
H08 (`p_type`, `r_name`), H09 (`p_name`), H21-positive (`o_orderstatus`, `n_name`),
D19/D85 (`n_name`), and D27 (`p_brand`, `n_name`); D72-inner has no string columns.
Catalog provider tokens declare their accepted element types: resident range
providers accept scalar codes, while host providers retain unsupported rows.
The explicit `tpch_dictionary` profile also stores String columns as codes when
their uses are literal equality predicates, including relational `Or` arms and
`In` sets. Each literal uses its own column's dictionary; H07's country-pair
select therefore executes on integer codes on the device. Opaque string
predicates and ordered String comparisons retain host strings. Once a host
projection drops the strings, its numeric rows are uploaded for subsequent GPU
work; host evaluation and upload are included in the build timing interval.
The synthetic String-guard regression checks this boundary against Sequential
and explicit expected keys.

H05 uses the explicit `tpch_domains` loader profile in **both** modes: the shared
`gpu_compress_host::tpch::schema`/`gpu_compress::Catalog` stores join keys as dense u32 domain IDs
and the date/Region dictionary code as i32, exactly as the authored benchmark.
The imported query's types and dictionary literals are adapted to that schema;
GPU and Sequential outputs are decoded back to source keys before comparison.
The loader profile is recorded in the pipeline artifact, not selected by the
planner. H05's advised pipeline records Region → Nation → Customer → Orders →
Lineitem → Supplier, retains the Customer prefix, and uses the R schedule:
one 256-row input partition and 1024-row/items-4 segmented Orders and Lineitem
expansions. Its unchanged fully-bound Supplier probe receives the checked
one-level `membership(...)` Structures certificate, so Sequence lowers it to a
filtering lookup without expanding witnesses. It does not call the authored
H05 implementation. H09 advises Part → Lineitem → PartSupp → Orders → Supplier →
Nation and retains the filtered Part leaf using its checked numeric carrier;
the substring predicate remains at its authored source position.

The binary accepts repeatable `--query <name>`, `--scale sf1|sf10`, and
`--mode automatic|advised` filters. Omitted filters select every query, both
scales and all built modes (advised only by default). Every run writes its selected inventory to the optional
positional results path, including filtered runs. `--baseline FILE` retains
prior per-query best measurements and reports ratios; without it, an existing
output file is captured before replacement. `--authored FILE` reads the
`gpu-h05-compiled` R measurements and reports the same-scale comparison.
Advised H05 performance acceptance requires SF10 total ≤ 10.35 ms and, when
provided, no slower than the measured authored R. A failed timing gate keeps
the exactness/timing evidence and exits unsuccessfully.

`TPCH_GPU_ADVICE=/path/advice.json` selects build-time per-query overrides.
Each entry is a pipeline object, for example:

```json
{
  "h03": {
    "plan": "no_reduce, order(Customer, Orders, Lineitem)",
    "physical": [],
    "structures": [],
    "sequence": []
  }
}
```

`plan` uses the CPU `order(...)`, `no_reduce`, `reduce(T with S)` and
`skip(T with S)` conventions. Physical advice accepts `retain_streams(...)`,
`hoist(...)`, `build_host_reads(...)`, `canon_prefixes(...)` and
`smallest_intersections(...)`; Structures advice accepts `sorted_indexes(...)`,
`membership(...)`, and `retype(...)`. Sequence entries parse the planner's
public `Advice` syntax, including `store`, `store_level`, `tile`, `partition`,
`segment`, `output`, `grade`, and `evidence { ... }`. CPU workload
`advise plan(...)` hints are imported into the report's `cpu_plan_advice`
field for explicit selection. They do not automatically replace the measured
GPU baseline: CPU-specific reduction hints can produce unsupported GPU builds.

Each accepted report includes `sequence_input`, `sequence_trace`,
`sequence_planned`, symbolic cost, and a resolved `pipeline` object. Copy
`{ \"query_name\": report.pipeline }` into the override file to replay it;
`complete: true` means its Sequence trace already records the complete
schedule and is not replanned. Manifest row counts are recorded as evidence,
never fabricated selectivities. Replay coverage checks exact program equality.

```sh
export CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/cuda/13.0
export BINDGEN_EXTRA_CLANG_ARGS=-I/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/math_libs/13.0/targets/x86_64-linux/include
cargo oxide build --arch sm_89 -- --release -p cq-gpu-tpch
cargo oxide test --arch sm_89 -- --release -p cq-gpu-tpch -- --nocapture
./target/release/cq-gpu-tpch
```

The binary visits both SF1 and SF10 and writes
[`compare/gpu_tpch.results.json`](compare/gpu_tpch.results.json). For accepted
translations it performs one warmup plus five measured iterations, checks every
iteration exactly against `answer.retail_<query>.tsv` and its `.count`, and
reports median CUDA-event total/build/kernels/dedup milliseconds. Input loading,
answer download, and comparison are outside those event intervals. SF1 also
compares the GPU answer to independent Sequential translation of the identical
Physical using `cq_testkit::conformance::assert_answers_equal`, the external
target API used by H05 (`assert_conformant` itself exposes only CPU targets).
The suite conformance tests cover built modes and reject unexpected planning
failures; H07 now requires a positive exact run rather than an expected catalog
rejection. `TPCH_GPU_DATA_ROOT` changes the fixture root at build time.
Each query runs all its iterations inside a same-thread `QueryScope`. Generated
thread-local caches register release hooks; scope teardown drops their owning
references, runtime scratch, and per-program modules before the next query
loads. Caches remain reusable across warmup and measured iterations. The suite
uses one process and one calling thread; teardown is outside query timings.
Do not substitute a fresh worker thread: H09/H21's large downloaded answers
fragment its malloc arena, and subsequent host-publication allocations spend
hundreds of milliseconds in `malloc_consolidate` inside the build interval.
The four-query subset can still show H09 totals around 45–66 ms, with an
occasional ~820 ms sample: glibc consolidates the freed answer allocations on
a later provider allocation. This is host work inside the build timing, not
slower device kernels. Reusing the numeric-bound readback vector moved that
cost to another allocation rather than removing it, so that experiment was
discarded. Full-suite timings and the subset variance are both recorded below.
The original unmodified executable on the same four-query subset also has the
outlier: H09 samples 52.276, 843.368, 44.806, 52.410, 44.807 ms (median 52.276).
Thus subset allocator sensitivity predates this fix; the same-thread cleanup
subset measured 66.436 ms, rather than the worker-thread version's 807.611 ms.
The ignored `sequential_sf10_queries_release_device_memory` GPU regression
runs H09 and D85 sequentially and checks `cuMemGetInfo` after each query against
the pre-query free-memory baseline (64 MiB allowance for driver bookkeeping).
Run it alone on an idle device after building all advised executors:
`cargo oxide test --arch sm_89 -- --release -p cq-gpu-tpch tests::sequential_sf10_queries_release_device_memory -- --ignored --exact --test-threads=1 --nocapture`.
The [SF10 lifetime regression measurements](compare/gpu_tpch_vram.results.json)
record all 25 compiled queries exact in one process, with zero free-memory
delta after every query (49,617,043,456 bytes free before and after each).
Before the fix, per-expansion thread-local `RangeWorkspace` and
`PackedColumnsAdapter` owners kept their pools' arenas alive across queries:
free memory fell to 814,219,264 bytes and D85 failed with an OOM. Explicit
release hooks remove those owners without changing plans or kernels. The
merged cuda-host module cache is bounded per native context and needs no
external API change. That historical full inventory exited 1 for H14/H19
translation rejections, not a runtime failure; its H05 timing gate passed at
10.275 ms. Final full-suite totals versus
`performance_report.md` are H05 10.275 vs 10.314 ms, H09 44.588 vs 44.563 ms,
H21-positive 40.053 vs 40.158 ms, and D85 26.925 vs 26.935 ms.
Statistics-derived H07 advice and its comparison with H07-block are recorded in
[`compare/tpch_statistics.md`](compare/tpch_statistics.md).

**h14 removed for now: prefix predicate inside the aggregate; needs order-preserving interning.**
The CPU h14 workload is unchanged. H19 explicitly declares its shipping,
brand and container columns as `u32`: the author prepares sorted-distinct,
zero-based dictionaries and binds `intern(table, column, literal)` constants.
The GPU importer resolves those constants against the column's manifest
dictionary and preserves the authored `u32` storage type. Other queries retain
the existing automatic literal-equality `dictionary_types` path.
The exporter already assigns the same sorted-distinct, zero-based codes to
all four columns; SF1/SF10 data need no re-export. Raw String loader declarations
remain the export contract, independently of the authored interned CQ types.
The [author-interned full-suite run](compare/gpu_tpch_intern.results.json) passed
all 26 supported queries at SF1 and SF10 in one process (52 exact results).
H19 total/build/query medians are SF1 9.254/0.002/9.253 ms and SF10
24.375/0.002/24.373 ms; [the performance addendum](compare/performance_report.md#author-interned-h19-addendum)
compares these with the report's DuckDB t1/t24 baselines.

Historical initial inventory, before the subsequent translation fixes: all eleven queries were rejected at both scales, so **no GPU
exactness, conformance agreement, or timing is claimed**. The release build and
13 tests passed (two source/encoding tests plus eleven explicit rejection
reports). No compiler gap was patched around. The table abbreviates only the
large `syn::Type` debug payload in catalog errors; the JSON preserves each full
error, failed phase, complete term, and imported CPU query. DuckDB values are
the existing per-query t24 medians, converted from seconds to milliseconds.

| query | SF1 exact | SF10 exact | GPU total ms SF10 (build/kernels/dedup) | DuckDB t24 SF10 ms | status |
|---|---|---|---|---|---|
| h03 | not run | not run | — | 167 | translate rejected: Sequence lookup lacks checked Physical row provenance |
| h05 | not run | not run | — | 172 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r13`, `[nation_key, supplier_key]`, Range, `DeviceColumns` |
| h07 | not run | not run | — | 137 | translate rejected: T-Gpu: provider predicate requires a recorded contiguous range selection |
| h07_block | not run | not run | — | 139 | planner rejected: BE-Boundary: a dispatch must have one unambiguous outer stored space (definition `r3`) |
| h08 | not run | not run | — | 116 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r11`, `[supplier_key]`, Range, `DeviceColumns` |
| h09 | not run | not run | — | 4472 | planner rejected: BE-Partial: opaque Physical Rust must be evaluated at host before dispatch |
| h21_positive | not run | not run | — | 3942 | translate rejected: T-Gpu: provider bound must be a constant or host parameter |
| d19 | not run | not run | — | 204 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r8`, `[part_key]`, Range, `DeviceColumns` |
| d27 | not run | not run | — | 79 | translate rejected: Sequence lookup lacks checked Physical row provenance |
| d72_inner | not run | not run | — | 292 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r9`, `[supplier_key]`, Range, `DeviceColumns` |
| d85 | not run | not run | — | 191 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r14`, `[part_key]`, Range, `DeviceColumns` |

### TPC-H data

TPC-H datasets are `/data/tpch-gpu/sf1` and `/data/tpch-gpu/sf10` (ignored,
never committed). Regenerate all authored CQ projections and reference answers
from each directory's existing `dbgen.duckdb`, from the repository root:

```sh
python3 experiments/gpu/compare/export_tpch.py --root /data/tpch-gpu --sf 1 10
```

The exporter reads every `features/workloads/src/queries/retail/tpch/*.rs`
relation declaration and its exact `sql r#"..."#` reference, including guards.
These are positive CQ kernels, not the original aggregate TPC-H/TPC-DS queries.
The required union of columns is:

| Table | Exported columns |
|---|---|
| customer | c_custkey, c_nationkey, c_mktsegment |
| orders | o_orderkey, o_custkey, o_orderdate, o_orderstatus |
| lineitem | l_orderkey, l_partkey, l_suppkey, l_shipdate, l_quantity, l_receiptdate, l_commitdate |
| supplier | s_suppkey, s_nationkey |
| nation | n_nationkey, n_regionkey, n_name |
| region | r_regionkey, r_name |
| part | p_partkey, p_brand, p_type, p_name |
| partsupp | ps_partkey, ps_suppkey, ps_availqty |

`manifest.json` schema version 2 contains `sf`, `byte_order: "little"`,
`binary_bytes`, `constants` (the original H05 date bounds and ASIA ID),
`relations`, and `queries`. Each relation has `rows` and a `columns` array;
each column has `name`, physical `type` (`i32`/`i64`), `logical_type`,
`encoding`, and `file`. Binary files are headerless aligned columns in original
database row order. Encodings:

* `plain`: signed integer keys/quantities, with the declared Rust width.
* `days_since_epoch`: signed i32 dates, `epoch: "1970-01-01"`, `unit: "day"`.
* `dictionary`: i32 indices into the named UTF-8 `dictionary` (`*.dict.txt`),
  one lexicographically sorted value per line, `dictionary_index_base: 0`;
  `dictionary_entries` records cardinality. Dictionaries are per column.
* `scaled_integer`: i64 decimal values with `scale: 2`; divide by 100 to decode.
  Thus the D72 `l_quantity > 20.00` guard compares encoded values to 2000.

Every `queries.<query>` entry records source path, exact SQL,
`required_columns` grouped by table (the complete per-query input inventory),
answer filename, distinct answer count, and timing filename. Answers are
`answer.retail_<query>.tsv` plus `.count`, sorted by all output columns.
`duckdb_timing.retail_<query>.json` retains the original timing schema:
engine/version, SQL, seconds, one warmup and five measured full SELECT DISTINCT
runs, and thread-keyed warmup/samples/median at 1 and 24 threads. Raw CLI logs
are retained per query/thread count; timings include normal result emission,
not a COUNT wrapper or the separate sorted answer export. H05 additionally
retains `answer.tsv`, `answer.count`, and `duckdb_timing.json`.

All pre-existing `.bin` files are SHA-256 checked before and after export;
`binary_checksums.json` records both maps and requires equality, preserving
the H05 binary inputs byte-for-byte. The manifest is published only after all
columns, checksums, answers, and timings complete. Temporary projected TSVs
are removed after conversion.

The completed SF1/SF10 export has the following distinct counts and DuckDB
median wall times in seconds (five samples after one warmup):

| Query (`retail_` prefix) | SF1 count | SF1 1 thread | SF1 24 threads | SF10 count | SF10 1 thread | SF10 24 threads |
|---|---:|---:|---:|---:|---:|---:|
| d19 | 8,962 | 0.086 | 0.096 | 90,015 | 0.600 | 0.204 |
| d27 | 357 | 0.078 | 0.023 | 3,575 | 0.806 | 0.079 |
| d72_inner | 5,344 | 0.232 | 0.119 | 53,966 | 1.648 | 0.292 |
| d85 | 8,962 | 0.058 | 0.035 | 90,015 | 0.655 | 0.191 |
| h03 | 11,620 | 0.054 | 0.034 | 114,003 | 0.735 | 0.167 |
| h05 | 7,242 | 0.062 | 0.024 | 72,983 | 0.681 | 0.172 |
| h07 | 5,501 | 0.064 | 0.030 | 54,142 | 0.724 | 0.137 |
| h07_block | 5,501 | 0.062 | 0.029 | 54,142 | 0.724 | 0.139 |
| h08 | 2,603 | 0.069 | 0.018 | 24,254 | 0.918 | 0.116 |
| h09 | 319,402 | 0.694 | 0.466 | 3,261,612 | 7.327 | 4.472 |
| h21_positive | 302,284 | 0.514 | 0.418 | 2,930,274 | 5.438 | 3.942 |

| Bytes (logical file sizes) | SF1 | SF10 |
|---|---:|---:|
| Binary columns | 325,773,800 | 3,256,586,628 |
| String dictionaries | 6,753,882 | 67,493,549 |
| Named query answer TSVs | 12,386,099 | 143,352,887 |
| Entire directory, including database, logs, and H05 copies | 757,972,236 | 7,876,436,632 |

All 13 pre-existing H05 `.bin` files at each scale passed before/after SHA-256
equality. The manifest includes all 11 query inventories and all 8 tables.

`h05-reference` remains the independent H05 reference implementation.
The Sequence H05 example in `sequence/examples/h05.rs` is also available with
`--features cq-gpu-sequence/cuda-oracle`.

The executable H05 conformance test uses the checked-in
`sequence/tests/support/h05_program.rs` fixture for CUDA and translates that
fixture's **same Physical program** through default CPU structure selection,
Sequence lowering, and the Sequential translator for its reference:

```sh
cargo oxide test --arch sm_89 -- --release -p cq-gpu-sequence --test h05_conformance -- --nocapture
```

It loads `/data/tpch-gpu/sf1` once with the same catalog and resident bindings as
the H05 example, then downloads those exact encoded input rows for the CPU.
The shared `cq-testkit` answer comparator checks canonical answer sets in
`(order, supplier, nation)` order. This is conformance to the Physical reference,
not a claim that the CPU runs the GPU's provider types or identical Sequence
terms. It does not use `answer.tsv` or a handwritten join as its reference.
If the dataset directory is absent, it prints an explicit `SKIP` before CUDA
initialization; loading or CUDA failures with the directory present fail the
test. It also runs in the default `cargo oxide test --arch sm_89 -- --release -p cq-gpu-sequence`
suite without feature flags; only the standalone example requires `cuda-oracle`.

## Authored advice and rejection boundaries

The resident H05 A/B/R and identity-chain callers author their Sequence programs
directly, then apply checked store/output/grade rewrites. They do not infer
grades or provider choices from generated temporary names.

| Former decision | Recorded Sequence decision |
|---|---|
| Retained prefix/build | Explicit `store` at its authored level |
| Sorted/prefix lookup representation | `RangeProvider` with the same recorded key levels |
| Expansion batch and `tile(256,4)` | Explicit `tile 1024 items 4` and flatten boundaries |
| Device deduplication | `canon` with `PackedCanon` |
| Output allocation | Explicit `output(count)` for exact count publication; bounded/reserved protocols must be authored separately |
| Execution placement | `grade(gpu, ...)`, checked by `machine_grade` |

For the resident identity chain, the former `capacity(Input)` bound is
interpreted explicitly as `output count;`: the identity result length is
known from its input. No unsupported dynamic-capacity term is introduced.

`frontier(8)`, `directory(rank)`, and the legacy resume/retry policies have no
one-to-one Sequence advice mapping. They are not silently passed to the new
terminal or asserted to be equivalent to count publication. No size-guarded
alternative was authored for these callers, so none is invented with `choose`.

H05 A/B explicitly publish the reduced Nation semijoin `r2` with an authored
`R2 = store ...` graded at device (EXT-SeqStore), and a recorded
`RangeProvider [nation]` Structures choice. The store is present before source
contract checking; a later rewrite cannot establish scope for an earlier
`lookup r2`. The provider is built only after that store is published; the
translator neither chooses the representation nor substitutes R.

Recorded range providers can consume published device row stores or canonical
packed-key stores, not just base relations. Owning source descriptors project
row-major fields with a stride and packed fields with their recorded
base/shift/width; they retain the publication without an intermediate
row-to-column copy or full unpack. Selection, key sorting, and payload gathering
read those same views. Store publication must precede provider construction;
no provider layout or new Sequence term is inferred.

Checked `compact` membership reads retain their Physical row provenance through
later maps, projections, and payload lookups, even when one provider serves
both existence and payload reads. Row-valued bounds in a recorded device
`compact` are evaluated in each lane against that lane's row; they are not
converted into host provider-build intervals. Host provider selections still
require the recorded contiguous range supported by their adapter.

The compiled triangle keeps its original `i64` relation and now translates
normally, with positive translation and executable tuple-comparison tests in
place of the rejection fixture. Range payloads support `u32`, `i32`, and full
signed `i64`, including values outside the 32-bit domain. Their fused gather
and FOR packing preserves 64-bit extrema and decodes without narrowing.
`triangle-compiled EDGES --check` uploads native i64 IDs, executes the authored
Smallest/First plan, and compares actual GPU tuples with its CPU reference.
It reports one discarded warmup and five measured resident calls, including
per-call and median GPU build/kernel/dedup/total timings and query wall time.
`--check` separately times the CPU tuple reference and checks every GPU call.
Input loading/upload, answer download, and reference checks are excluded from
the query samples.
The recorded directory still requires dense nonnegative u32-range keys.
`PackedCanon` advertises a 128-bit packed-key capability. At setup, the sum
of the recorded codec-bound widths selects one u64 lane (up to 64 bits) or
two lanes (up to 128 bits); larger headings return a setup error before
packing or dispatch. Narrow keys retain radix sorting, while wide keys use
exact row sorting and adjacent deduplication. Cross-lane fields remain
zero-copy projections for later providers. This is provider-internal storage
selection, not a translator or planner choice; no schema narrowing, ID
remapping, or alternate query plan is substituted.

## Device-store and i64 acceptance

On NVIDIA RTX 6000 Ada Generation (driver 580.178.04), the `sm_89` release
workspace build and test commands pass, including 22 tests and all doc tests.
The positive tests cover provider construction after device-store publication,
i64 triangle translation/execution, signed 64-bit payload extrema, and strided
and packed stored-column descriptors. `python3 doc/formal/check.py` reports
228 synchronized rules; no new translation decision or rule was introduced.

These H05 measurements use one discarded warmup and five measured resident
calls. Every A/B/R call and the reference compare complete tuples against
`answer.tsv`; all are exact. GPU time includes base provider construction,
dispatches, device-store provider construction, and final canonicalization.
The latter provider builds occur inside the reported kernel phase, after the
base-build timing boundary. Upload and answer download are excluded.

| Workload | Exact rows | Sequence GPU ms | Reference GPU ms | Sequence wall ms | Reference wall ms |
|---|---:|---:|---:|---:|---:|
| H05 A SF1 | 7,242 | 2.526368 | 1.922624 | 2.587867 | 1.931820 |
| H05 B SF1 | 7,242 | 1.796928 | 1.922624 | 1.855975 | 1.931820 |
| H05 R SF1 | 7,242 | 2.129568 | 1.922624 | 2.192349 | 1.931820 |
| H05 A SF10 | 72,983 | 18.664608 | 9.899072 | 18.731918 | 9.909329 |
| H05 B SF10 | 72,983 | 10.202752 | 9.899072 | 10.264289 | 9.909329 |
| H05 R SF10 | 72,983 | 10.323200 | 9.899072 | 10.388337 | 9.909329 |

`triangle-compiled /data/dataset/graph/com-dblp.ungraph.txt --check` retains
1,049,866 distinct native-i64 edges and produces exactly 2,224,385 tuples,
equal to its independent CPU reference on every call. One warmup is excluded
from five GPU samples; the CPU reference is measured once.

| Triangle workload | Build ms | Kernel ms | Dedup ms | GPU total ms | Query wall ms | CPU reference ms |
|---|---:|---:|---:|---:|---:|---:|
| DBLP, native i64 | 2.062880 | 3.302496 | 0.627648 | 6.020128 | 6.082209 | 244.443683 |

The H05 samples are retained in
`compare/gpu_sequence_device_stores.results.json` and
`compare/gpu_sequence_device_stores.reference.sf{1,10}.json`.

## Cutover validation

On NVIDIA RTX 6000 Ada, `sm_89`, both workspace build and test commands above
passed. The main workspace has 16 passing tests, including one test exercising
three schema UI cases. `python3 doc/formal/check.py` passed with a 226-rule
inventory; the GPU interpretation marker now lives in `sequence/core`.
The generated CUDA exports have expansion-specific names so multiple query
expansions in one binary do not collide.

H05 medians below use one discarded warmup plus five measured calls. Both
implementations compare full answer tuples against the independent
`answer.tsv`; the identity-chain answer is checked after explicit download.
GPU times include resident index build, query, and deduplication, not input
loading/upload or answer download. Chain time is separate, not added to R.

| Workload | Exact rows | Sequence GPU ms | Reference GPU ms | Sequence wall ms | Reference wall ms | Chain GPU ms |
|---|---:|---:|---:|---:|---:|---:|
| H05 R SF1 | 7,242 | 2.086272 | 1.895552 | 2.148636 | 1.906872 | 0.181888 |
| H05 R SF10 | 72,983 | 10.139392 | 9.759232 | 10.206828 | 9.769801 | 0.228640 |

`printer-smoke` passed its nonempty and empty cases. `printer-oracle` passed
resident projection for empty/sorted/unsorted/duplicate inputs and repeated
head projection. These do not claim the old runtime certificate, multiword
key, or resumable-provider coverage.

Historical cutover validation compiled the then-unsupported H05 A/B and
signed-i64 triangle rejection fixtures independently. Those fixtures have now
been removed in favor of positive device-store provider and i64 translation
tests; the historical rejection results in the archived cutover summary are
not current support boundaries.

The **legacy** workspace also built independently. Its Datalog oracle passed
eight scenarios, and its handwritten triangle oracle passed seven cases
(including 3,999,859 answers in the million-degree case), against independent
CPU count/materialization checks. The cvc5 ddisasm smoke run checked all eleven
outputs exactly against the CPU implementation: fixed point 8,482.319 ms,
upload 21.342 ms, CPU reference 28,605.944 ms, 878 checked rounds. That is one
smoke run, not a median or a Sequence performance claim.

Machine-readable evidence:
[`summary`](compare/gpu_sequence_cutover.summary.json),
[`R and chain samples`](compare/gpu_sequence_cutover.results.json), and
[`SF10 reference samples`](compare/gpu_sequence_cutover.reference.sf10.json).
SF1 reference medians/samples were captured from reference stdout in the
summary; its single-object output file was overwritten by the SF10 invocation.

## Grouped GPU folds

`group` consumes an explicitly published canonical valuation store: `canon`
deduplicates every named body column, not the aggregate arguments. Its host
boundary orchestrates resident radix key ordering, adjacent-key owner offsets,
and collaborative segment-local count/sum/min/max kernels. Integer sum widths
follow the declared codec; overflow publishes failure rather than a partial
answer. Empty keyed input has no owners; empty global input has one identity.

Grouped results expose `checked_len()` and `to_host()` through `GroupedRows`.
They retain separate value/validity columns, preserving every i64 value while
decoding empty extrema as `Option::None`, and retain ancestor output statuses.
The translator does not add publication or canonicalization decisions.

The `group` integration test covers checked translation and duplicate-valued
distinct valuations. Runtime tests cover extrema, empty groups and arithmetic
failure. `grouped_h01` computes the CPU corpus's DISTINCT-valuation SQL oracle
against `/data/tpch-gpu/sf{1,10}/dbgen.duckdb`, compares exact aggregate tuples,
and reports GPU and DuckDB t24 wall times. A saved `duckdb_timing.json` is reused
only when its SQL matches H01; the normal H05 timing is not substituted.

## Negation GPU acceptance

The suite now imports 15 CPU workload sources, including H04, H04-not-exists,
H16 and H22. The latter three retain Physical `anti eager` provenance and
lower to `compact result lookup provider [keys] input`; H04 retains positive
membership. No lowering change was needed. E-Neg's GPU instance uses the
same provider membership probe in either polarity and inverts only the keep
bit for anti. Wildcard order keys in H22 remain existential witnesses.

Advised order is `Lineitem, Orders` for H04, `Orders` for H04-not-exists,
`Part, Partsupp` for H16, and `Customer` for H22, with `no_reduce`.
The importer executes the original `LateLineitem` and `BadSupplier` setup
initializers once while loading. These derived-input construction costs,
manifest loading, and initial base-column uploads are outside the reported
query interval. Provider construction and uploads from retained host rows,
including string predicate evaluation, are inside `build`.
DuckDB's existing t24 measurements execute the complete independent SQL;
these are not identical end-to-end timing boundaries.

Release sm_89 run, one warmup and five measured iterations, all eight answers
exact against exported TSVs; SF1 also agrees with Sequential:

| query | scale | rows | GPU total ms (build / kernels / dedup) | DuckDB t24 ms |
|---|---|---:|---:|---:|
| h04 | SF1 | 52,523 | 1.158 (0.511 / 0.519 / 0.126) | 33 |
| h04_not_exists | SF1 | 4,695 | 37.901 (37.653 / 0.135 / 0.110) | 15 |
| h16 | SF1 | 118,274 | 1.549 (1.304 / 0.090 / 0.150) | 111 |
| h22 | SF1 | 14,086 | 1.072 (0.927 / 0.043 / 0.102) | 21 |
| h04 | SF10 | 526,040 | 10.363 (5.039 / 5.033 / 0.291) | 311 |
| h04_not_exists | SF10 | 47,631 | 380.482 (379.280 / 1.053 / 0.148) | 92 |
| h16 | SF10 | 1,186,602 | 10.217 (9.752 / 0.155 / 0.307) | 1,047 |
| h22 | SF10 | 140,489 | 7.490 (7.298 / 0.060 / 0.131) | 132 |

Evidence: `/data/tpch-gpu/box13b-negation.json`, generated by
`./target/release/cq-gpu-tpch /data/tpch-gpu/box13b-negation.json --mode advised --query h04 --query h04_not_exists --query h16 --query h22`.
H04-not-exists is slower than DuckDB at both scales; its measured build cost
dominates, rather than the inverted membership kernel.

## Static shared-memory and generated-source contracts

`machine()` records a 101,376-byte block shared-memory capacity. BE-Resource
derives target-independent dispatch demands, including every choice arm and
the checked live-column type/codec facts. `core::layout` owns the byte/alignment
formula used both by grading and by the emitter. An advised plan exceeding the
capacity is rejected before translation, with its publication, required bytes
and capacity. Automatic planning considers explicit publication boundaries
between expanding stages and choices; the emitter never chooses a fallback.

Kernel signatures and their launch arguments retain only parameters referenced
by each generated body, preserving declaration order. This includes column and
provider views, prior stores, packing fields, domains, enables, and output
pointers. The translator checks the aligned NVPTX64 parameter ABI after pruning:
a live signature of 4096 bytes or more is rejected as `BE-Resource`, naming the
kernel, byte demand, and live parameter count. It does not silently move
parameters into device memory or change the recorded plan.

Catalog `levels` describe provider reads, not construction. Resident
`RangeProvider` and `PackedCanon` tokens are device/block/lane tokens;
host-only row providers remain host-only even when their inputs are built there.

The TPC-H build script parses its generated module as a `syn::File` and writes
`prettyplease::unparse` output. Generated-code warnings are scoped out at the
module boundary; ordinary handwritten code retains its diagnostics. The measured
automatic-suite library compile fell from an interrupted run exceeding 54 minutes
to 398.31 seconds after this source-format change (482.87 seconds for that whole
release build). These are compilation measurements, not query execution timings.
