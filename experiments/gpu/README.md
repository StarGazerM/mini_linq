# Experimental GPU Sequence backend

**Audit scope (2026-11-02):** implementation descriptions below were checked
against repository revision `7e187597c478cfcbc5ff9daf61c520c3ce984106`.
Commands are reproduction instructions, not evidence that this revision passed
them. Measurement tables are retained historical results at their cited
revisions/dates, not current benchmark baselines or current test inventories.
Where a retained claim lacks a checked-in measurement artifact, it is explicitly
marked unverified. Do not compare different query versions, timing boundaries,
or profiled and unprofiled runs as a paired speedup.

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
grading and recorded analysis. The former emitter decisions are now checked
records: `Partition::{One,Tiles,Spread}` and kernel occupancy for dispatch
geometry, `Publication::DeferredRange`, `OffsetScan::DeviceExclusive`,
`FoldCompletion::BlockPartials`, and recorded metadata synchronization.
The old literal grid cap and one-block offset/fold completion are historical,
not open violations. See [Sequence contract §6](../../doc/SEQUENCE-PLAN.md#6-experimentsgpusequence-cuda-translation-replaces-experimentsgpuprinter-at-cutover),
[`realize.rs`](sequence/core/src/realize.rs), and
[`emit.rs`](sequence/core/src/emit.rs). New decisions must likewise be recorded
and checked before emission.

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

The workspace pins the StarGazerM cuda-oxide and RCCL forks by commit.
The pool-workspace integration temporarily overrides RCCL with
`/data/rccl-wt/pool-workspaces` in [`Cargo.toml`](Cargo.toml); replace that patch
with the pushed RCCL revision and update [`Cargo.lock`](Cargo.lock) together.
cuda-oxide remains pinned to `57eacecc081fe1809a815ae4ca32643884dce15b`.
It uses cuda-oxide's Rust **1.99.1** SIMT toolchain, with `RUSTC_BOOTSTRAP=1`
configured for the compiler backend. Build the backend and install `cargo-oxide`
from that pinned fork's nested `cuda-oxide/` workspace; the backend path is
configured in `.cargo/cuda-oxide.toml`.
The monorepo keeps host crates at its root and device/codegen crates under
`cuda-oxide/crates`. Rebuild the configured release backend from the pinned
checkout with `RUSTC_BOOTSTRAP=1 cargo build --release --lib --target-dir target`
in `cuda-oxide/crates/rustc-codegen-cuda`.
The configured backend is built from that pinned commit ("Remove quadratic
passes from the device pipeline"). Measured against the previous pin
`299eda41b` on `cq-gpu-sequence-runtime` (530 device functions): device
pipeline about 52 s to 16 s, with byte-identical PTX for it and the four device
crates below it (evidence: `design-notes/oxide-profile/`).

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

Formatting needs no CUDA toolchain: run `cargo fmt --all --check` from this
directory. `--all` also checks local path dependencies outside this workspace
(the DOOP crates under `features/datalog/doop`), which belong to the root
portable workspace, including `doop/srd` and its CPU engines.
To exercise formatting without changing files or failing on existing style
differences, use `cargo fmt --all -- --emit stdout`; `--check` additionally
requires every included source to already match rustfmt.

### Lookup/Expand child counters

Build with `CQ_EXPANSION_COUNTERS=1` to instrument GPU Sequence and CPU pull
expansions. This is a **compile-time** opt-in; build scripts track its presence,
so rebuilding without it restores the uninstrumented generated code. Do not
use a counter build for timings. No Rust expression or Rust type is inspected.

```sh
CQ_EXPANSION_COUNTERS=1 cargo oxide build --arch sm_89 -- --release -p srd-doop-gpu
# Run the usual xalan command, saving stderr as counters.log.
python3 ../../tools/expansion_report.py < counters.log
```

GPU counters sum `u64` children at each fused range expansion **before**
downstream closing predicates/lookups. The block leader adds the collective
child count, never one atomic per lane/child. Deferred range publication counts
the same children without materializing them. Only the successful capacity
attempt is reported; overflow attempts are not observations. Static IDs are
the checked Sequence program/module, publication definition, and expansion
ordinal in its emitted chains. Datalog uses the source rule label when present
(otherwise head/hole), plus the checked delta variant and runtime round number;
no new scheduling advice is inferred.

`cq_expansion` stderr records contain `rule`, `variant`, `iteration`, `program`,
`definition`, `stage`, and `children`. `cq_expansion_output` contains the
definition's closing output and `admitted`. For Datalog, `admitted` is the
rule's distinct output absent from the **pre-round** accepted relation, measured
on-device without mutating admission state. Two rules can contribute the same
fresh tuple, so these per-rule counts must not be summed as relation growth.
This diagnostic adds sort/subtraction work; ordinary admission is unchanged.
Outside Datalog (including GPU TPC-H), reporting is automatic after each
successful dispatch and the denominator is that definition's closing output
before any separate canonicalization or later host aggregation.

The report tool sums each rule/variant/definition/stage across iterations and
prints JSON, including `largest_preclosing`, `admitted`, and
`largest_preclosing_per_output` (`null` for zero output). It does not add
duplicate admission copies from different definitions.

In counter builds only, `GPU_DATALOG_TRACE_KERNELS=1` also enables existing
arena event timing around each hole and emits `gpu_datalog_hole_timing` records
with rule/variant/iteration/hole, kernel, total GPU, and wall milliseconds.
The previous arena timing mode is restored after each hole. These fenced,
instrumented timings support attribution, not the uninstrumented paired
`GPU_DATALOG_PHASES=1` fixpoint comparisons.

For CPU pull and Tasks, also enable the consuming crate's `cq-storage/counters` feature.
Call `cq_storage::counters::reset()` before the query, fully exhaust/drop its
iterator, then print `cq_storage::counters::render_expansions()` or inspect
`expansions()`. Stage labels are `Sequence-definition/Physical-result`; the
`output` counter is the final query iterator. Counts occur on child production
before closing consumers, including exact/range/intersection expansions.
Without the storage feature the wrappers are identity functions; without
`CQ_EXPANSION_COUNTERS` the printer emits no wrappers at all.
Operator profiling can be enabled at the same time: child producers remain
ordinary expressions visible to closure ownership analysis, so nested pull
closures and Tasks workers retain their own profile/provider handles.

### Authored Datalog Lookup/Expand experiments

The SRD GPU engine uses order-preserving decomposition by default and offers
mutually exclusive `advice-aipt`, `advice-load`, `advice-fieldflow`, and
`advice-dispatch` features for individually authored experiments.
Compare order-preserving decomposition and decomposition plus one authored
advice separately. `cq_gpu_sequence::decompose` realizes the recorded CQ
construction without relational replanning.

`advice-aipt` requests an intersection of source heaps with compatible heaps
(and the symmetric baseheap site in the source-heap delta variant). `advice-load` moves VarType and
the baseheap intersection before array contents only in the Reachable-delta
variant. The controls add membership lookups before full expansion: FieldFlow
on its ReachableLoadInstanceField/VarPointsTo deltas, virtual dispatch on its
ReachableInstruction delta. True smallest controls are rejected by the current
packed-layout prefix contract: FieldFlow needs a field-first candidate prefix;
dispatch's canonical `(descriptor, simplename, heap)` prefix differs from its
packed `(simplename, descriptor, heap)` order. No layout or type is inferred.

**Smallest support is provider-specific.** The P3.2 experiment exposed a
historical silent fallback to one provider's range followed by membership.
That fallback was rejected, then P4-GPU slice B (`af735660`, 2026-11-01)
added real minimum-candidate selection over recorded two-level radix layouts
with a Candidates read on every side. The current rejection boundary is
documented in [`realize.rs`](sequence/core/src/realize.rs), not a blanket
target-wide prohibition. The historical packed `DeviceLayout` experiment
results here are not evidence for the slice-D radix relation cutover;
`advice-aipt` and `advice-load` are not evidence of a realized smallest plan.
See the executable `triangle-smallest`
case and its dated paired evidence below. CPU smallest support is unchanged.

Build each variant separately, e.g.
`cargo oxide build --arch sm_89 -- --release -p srd-doop-gpu --features advice-fieldflow`.
For timing, omit `CQ_EXPANSION_COUNTERS`, set `GPU_DATALOG_PHASES=1`, and use the
`phase<TAB>fixpoint` record rather than the harness's outer evaluation total.
Counter builds are separate; relation counts must match all 40 reference
relations on every run (the factored program also exposes three helper outputs).

#### P3.3 evidence ordering and grading (2026-10-31)

Historical acceptance snapshot; “treatment” below means the measured P3.3
revision, not the current checkout. The dated
[`p33-evidence.results.json.gz`](compare/p33-evidence.results.json.gz) is the
source for this table; later P6b DOOP measurements are summarized below.

The automatic TPC-H route records manifest counts and TPC-H primary keys in
the VO (O-EvidenceLe); a key-functional root replaces the written root only
when it dominates it (count ≥, filter rank ≥, no neighbour other than the
written root filtered as strongly). Grading retains a prefix of a sub-tile
root up to 65,536 rows and segments later fan-out (GR-RetainPrefix,
GR-SegmentFanout). Against `c1b75718`, generated GPU code changes only for
H04 automatic (SF1, SF10) and H02/H11 advised at SF10; d85 is identical.
Same protocol as below (5 AB/BA pairs, 1 warmup + 5 samples, bootstrap
20,000 resamples, seed 3303), all answers exact; H05 step R uses the
preserved step-R order as advice.

| Target | `c1b75718` median ms | P3.3 treatment median ms | Treatment/base [95% CI] |
|---|---:|---:|---:|
| H04 SF1 automatic | 0.990 | 0.647 | 0.661 [0.652, 0.676] |
| H04 SF10 automatic | 11.677 | 5.810 | 0.500 [0.497, 0.503] |
| d85 SF1 automatic (identical code) | 1.487 | 1.492 | 1.007 [0.996, 1.022] |
| H02 SF10 advised | 138.259 | 102.115 | 0.733 [0.720, 0.745] |
| H11 SF10 advised | 12.915 | 5.433 | 0.422 [0.417, 0.427] |
| H05 SF1 step-R order | 10.097 | 1.216 | 0.121 [0.119, 0.124] |
| H05 SF10 step-R order | 124.742 | 8.658 | 0.070 [0.069, 0.070] |

Before the neighbour clause, an unfiltered Lineitem root (reduced by
date-filtered Orders in the baseline) displaced the filtered PartSupp written
root in d85: 1.363 [1.358, 1.367]. Raw pairs and page faults:
[`compare/p33-evidence.results.json.gz`](compare/p33-evidence.results.json.gz).

#### P3.3-lite removal acceptance (2026-10-30)

At this historical removal checkpoint, automatic GPU routes decomposed the
planner's recorded order; Sequence lookup-first and optional-filter adaptive
replanning had been deleted. Five fresh-process interleaved AB/BA pairs compare
against baseline `0072f5f`. TPC-H uses one warmup and five samples per process;
DOOP uses the synchronized `GPU_DATALOG_PHASES=1` fixpoint phase. The GPU was
idle before every invocation. Intervals below are paired treatment/baseline
ratios, percentile bootstrap, 20,000 resamples, seed 3303 (lower is better).

| Target | `0072f5f` median ms | P3.3-lite treatment median ms | Treatment/baseline [95% CI] |
|---|---:|---:|---:|
| H05 SF1 automatic | 7.409 | 1.752 | 0.236 [0.235, 0.237] |
| H05 SF10 automatic | 68.649 | 13.084 | 0.191 [0.190, 0.191] |
| DOOP xalan fixpoint | 1590.302 | 1481.221 | 0.931 [0.930, 0.934] |
| DOOP batik fixpoint | 5654.268 | 4464.366 | 0.789 [0.788, 0.791] |
| H04 SF1 automatic | 0.798 | 0.968 | 1.213 [1.169, 1.225] |

All 26 automatic TPC-H queries at SF1 and H05 at SF10 matched the DuckDB
answers on every invocation. Every DOOP run matched 40/40 reference counts.
All 52 advised GPU executor bodies (26 queries × two scales) are identical
after normalizing only generated kernel-name hexadecimal salts. The concurrent
recorded canon-strategy migration changes those salts and CPU reference
interpreters, not advised GPU execution.

H04 is the only ratio interval wholly above 1. Its preserved CQ order retains
filtered Lineitem and probes it from Orders. The removed rewrite instead
retained date-filtered Orders and streamed Lineitem through semijoin/probe,
using one dispatch with four items per lane. Decomposition materializes the
recorded Lineitem provider and uses two dispatches with one item per lane.
Build time falls 0.263→0.002 ms, but kernel time rises 0.422→0.854 ms.
This is the loss of a downstream join-order optimization; no Sequence
reordering is reintroduced to hide it. Full per-query ratios, samples,
binary hashes, advised-code hashes and the two H04 plans are retained in
[`compare/p33-replanning.results.json.gz`](compare/p33-replanning.results.json.gz).

#### P3.2 DOOP measurements (2026-10-30)

This is the historical P3.2 checkpoint (`7a1c19df`, 2026-10-30), before
P3.3 order-preserving cutover and P6b root sharing/multistream execution.
The baseline and control timings below remain evidence of that experiment,
not current GPU timings. For later xalan/batik fixpoint results see the
SRDatalog section below; no old measurements have been overwritten.

Five interleaved process-isolated pairs per comparison and dataset, alternating
AB/BA; `nvidia-smi` confirmed no competing compute process before every run.
Every run matched all 40 reference counts in
`/data/doop-bench/srd-{xalan,batik}-fullclock/*/report.json`.
Times below are uninstrumented fixpoint milliseconds, median [95% percentile
bootstrap CI, 20,000 resamples, seed 3202]. Advice speedups are medians of paired
ratios against the **decomposition-only** baseline, not against automatic plans.

**The AIPT and array-load timing rows below are historical measurements of the
pre-rejection sequential fallback, not measurements of realized smallest
intersections.** The lookup-only controls and baseline configurations are
historical measurements too. The counter discrepancy below motivated rejecting
unsupported smallest plans rather than silently accepting them.

| Whole-program baseline comparison | xalan ms | batik ms |
|---|---:|---:|
| P3.2 automatic Sequence plan (historical) | 1587.5 [1585.1,1669.9] | 5401.7 [5396.7,5420.9] |
| Decomposition-only, no rule advice | 1489.6 [1482.2,1493.4] | 4380.6 [4368.2,4385.1] |
| Paired automatic/decomposition-only speedup | 1.070 [1.064,1.124] | 1.234 [1.231,1.238] |

Each row below is a separate paired experiment; its local decomposition-only
baseline is reported rather than hiding between-batch variation. Together with
the automatic baseline above these expose all three configurations.

| Advised rule(s) | Dataset | Decomposition-only baseline ms | Advised ms | Paired speedup [95% CI] |
|---|---|---:|---:|---:|
| AIPT_Store | xalan | 1485.5 [1483.9,1491.4] | 1525.1 [1523.9,1528.0] | 0.974 [0.971,0.978] |
| AIPT_Store | batik | 4377.4 [4364.9,4378.1] | 4457.6 [4453.2,4466.3] | 0.981 [0.980,0.982] |
| VPT_LoadArray, Reachable delta only | xalan | 1481.2 [1479.0,1484.1] | 1484.0 [1480.8,1486.6] | 0.998 [0.995,1.002] |
| VPT_LoadArray, Reachable delta only | batik | 4361.8 [4359.6,4371.6] | 4369.4 [4365.0,4374.4] | 0.998 [0.997,1.001] |
| FieldFlow lookup-only control | xalan | 1488.7 [1487.4,1492.5] | 1500.9 [1497.3,1503.6] | 0.992 [0.989,0.997] |
| FieldFlow lookup-only control | batik | 4383.4 [4373.4,4411.3] | 4419.7 [4405.5,4428.2] | 0.993 [0.990,0.996] |
| VirtualDispatch_MultiHead_0/1/2 lookup-only control | xalan | 1483.4 [1481.5,1488.4] | 1495.0 [1487.4,1496.0] | 0.992 [0.991,1.001] |
| VirtualDispatch_MultiHead_0/1/2 lookup-only control | batik | 4379.8 [4361.1,4381.8] | 4383.9 [4378.9,4400.8] | 0.997 [0.995,1.001] |

The three virtual-dispatch heads are changed together, so their fixpoint timing
is one shared treatment, not three independent timings. The supported controls
are neutral/slightly slower. The historical AIPT slowdown and array-load neutral
result cannot establish the performance of a real minimum-candidate
intersection. Do not automate the offline skew estimates on this evidence.

An explicitly exploratory earlier run kept automatic Sequence planning and
applied the array-load order to **every** delta variant: xalan 1587.8→1520.0 ms,
paired 1.051 [1.039,1.070]; batik 5433.1→3239.4 ms, paired
1.679 [1.662,1.935]. That result changes delta drivers and permits subsequent
replanning; it is not evidence for the isolated Reachable-delta intersection.

Counter totals below are identical in default, decomposition-only, and the
corresponding pre-rejection advised/control configuration. “Largest” sums each
variant's largest stage total over iterations; “fresh” sums per-variant distinct
output absent from the pre-round state, **not** additive relation growth.

| Rule | Dataset | Largest preclosing (all three configurations) | Fresh output | Largest/fresh |
|---|---|---:|---:|---:|
| AIPT_Store | xalan | 58,410 | 2,426 | 24.08 |
| AIPT_Store | batik | 4,716,422 | 6,461 | 729.98 |
| VPT_LoadArray | xalan | 451,014 | 44,459 | 10.14 |
| VPT_LoadArray | batik | 35,709,707 | 1,636,178 | 21.83 |
| FieldFlow_rule | xalan | 612,069 | 30,685 | 19.95 |
| FieldFlow_rule | batik | 3,762,782 | 256,357 | 14.68 |
| VirtualDispatch_MultiHead_0 | xalan | 948,814 | 64,013 | 14.82 |
| VirtualDispatch_MultiHead_0 | batik | 9,778,922 | 333,602 | 29.31 |
| VirtualDispatch_MultiHead_1 | xalan | 948,814 | 14,359 | 66.08 |
| VirtualDispatch_MultiHead_1 | batik | 9,778,922 | 71,981 | 135.85 |
| VirtualDispatch_MultiHead_2 | xalan | 948,814 | 2,277 | 416.69 |
| VirtualDispatch_MultiHead_2 | batik | 9,778,922 | 11,757 | 831.75 |

AIPT's actual stage children, summed over iterations, explain the failed
prediction. Delta positions 0/1/2 are ReachableSortedIndex/base VarPointsTo/source
VarPointsTo; global variant IDs are 101/102/103. The historical advised stages
equal the automatic stages; decomposition-only omits their final closing stage.

| Dataset | Delta | P3.2 automatic / historical advised stage children | Decomposition-only stage children | Fresh | Traced GPU total ms: automatic / decompose / historical advice |
|---|---:|---|---|---:|---|
| xalan | 0 | 5,558; 1,829; 506 | 5,558; 1,829 | 506 | 7.483 / 6.860 / 6.913 |
| xalan | 1 | 8,772; 3,789; 560 | 8,772; 3,789 | 555 | 11.000 / 10.244 / 35.269 |
| xalan | 2 | 28,195; 44,080; 2,277 | 28,195; 44,080 | 1,365 | 14.459 / 13.394 / 14.135 |
| batik | 0 | 23,560; 5,759; 831 | 23,560; 5,759 | 824 | 12.115 / 11.384 / 11.440 |
| batik | 1 | 29,129; 2,606,058; 26,664 | 29,129; 2,606,058 | 1,062 | 123.583 / 109.614 / 169.092 |
| batik | 2 | 739,438; 2,086,804; 42,361 | 739,438; 2,086,804 | 4,575 | 50.594 / 47.168 / 48.236 |

In instrumented decomposition-only runs, AIPT accounts for 30.50/2888.65 ms
(1.06%) of xalan's traced fixpoint and 168.17/6535.08 ms (2.57%) of batik's.
Historical fallback advice raises those to 56.32/2934.41 ms (1.92%) and
228.77/6632.42 ms (3.45%). Even eliminating AIPT's whole GPU hole cost would
therefore remove only about 2.6% of the **traced** batik fixpoint, holding the
rest fixed; this is not a bound on uninstrumented wall time. The sourceheap
visit count did not fall by the offline 4.8×/15.4× estimate.

The targeted VPT_LoadArray Reachable-delta variant (75) produces zero fresh
output on both datasets: baseline stages are `[1434,0,0,0]` / `[3766,0,0,0]`;
historical advice adds the VarType stage, yielding `[1434,1434,0,0,0]` /
`[3766,3766,0,0,0]`. The large batik work is in variants 76/77, not this site.

Complete per-rule/per-variant stage totals, each iteration's children/largest/
fresh output, traced GPU milliseconds and fixpoint shares, paired raw timings,
confidence intervals, and the explicitly labelled exploratory runs are in
[`compare/p32-doop-advice.json`](compare/p32-doop-advice.json).
[`compare/p32-doop-llvm-identity.json`](compare/p32-doop-llvm-identity.json)
records final no-advice LLVM identity (6,250,872 bytes before and after; exact
normalized equality without reordering instructions).

### Radix-backed Datalog runtime

`gpu-datalog-runtime` provides `Packing`/`Column` encodings and ordinary
`DeviceRadixRun<W, u64, ()>` full-key sets on the resident-column `Pool`.
Every Datalog layout publishes a typed
`RadixLayout<(all physical row columns), (), W, u64, ()>` view, including
EDB, canonical, secondary, delta and frozen negative layouts. The recorded
`radix_relation` provider uses `NativeAbi::RadixRelation`; the logical
key/payload split is a prefix boundary, not a split into radix item columns.
[`datalog/src/storage.rs`](datalog/src/storage.rs) records and independently
checks the contract; [`datalog-runtime/src/radix_run.rs`](datalog-runtime/src/radix_run.rs)
and [`datalog-runtime/src/layout.rs`](datalog-runtime/src/layout.rs) implement it.
The narrowest word is u64, u128, or `[u64; L]` for larger schemas.
Array lanes are most-significant first; native u128 keeps
its native in-memory lane order. Integer columns use their full declared width;
signed columns flip the sign bit, and the first tuple column occupies the most
significant packed bits. `prefix_range` returns inclusive bounds, including the
maximum word without overflow.

Canonical `fresh` sorts, deduplicates and subtracts all accepted complete keys
on-device without changing state, returning an immutable `SortedBatch`.
Separate `append_sorted` publication requires a sorted, unique batch disjoint
from accepted keys. All secondary layouts derive from that same admitted
batch: bijective column permutation preserves uniqueness and cardinality,
but sorting must restore the new order. Negative projections independently
deduplicate, since projection need not be injective. Recorded delta moves
are unchanged. Maintained views use `republish(&state)`; initial construction
uses `from_sorted(pool, packing, params, batch)`, and delta replacement uses
`with_sorted(batch, params)` to retain kernels and field descriptors.
Explicit output uses `to_rows`. Full-key views require the state/snapshot's
construction-only set witness, established by `new_set` and preserved by set
append/replacement. Occurrence publication into that state is rejected;
unit item columns alone are not a uniqueness certificate.

Each `DeviceState` records `offset: u64`, `offset_layout: implicit_unit`,
`item_sources: none`, `construction: full_key_set`, complete-codec `active_bits`,
`sort: native_keys_or_lanes`, certified `admitted_order`, `read: key_prefix`,
ratio, `radix_bits`, merge-tile rows, block threads and permutation threads.
`radix_bits` defaults to zero (no directory) but legal
configured values are accepted. `RadixRunParams` consumes explicit settings:
ratio is at least two, merge tiles contain at least one row, and compiled
run/permutation kernels require 256 threads. Unsupported geometry is rejected
rather than adjusted. The checker validates token/ABI, complete ordered
schemas/codecs, state and initializer agreement, legal settings, cardinalities,
canonical/layout references, scratch policy and exact source erasure; it never
replays realization or requires its default choices. Full keys are retained
regardless of directory resolution; unit item columns alone do not establish
set uniqueness. The former public `DeviceRuns`/`DeviceLayout` APIs have no
compatibility aliases.
Work is ordered on the owning pool's stream. The main pool uses the legacy
default stream; explicit wave lanes use streams sharing that pool's allocator,
with event fork/join before admission (see `lanes` below).
Packed permutation, wide-word radix gather, admission membership and wide merge
size their grid-stride launches from the loaded kernel's CUDA occupancy,
capped by the work domain; they do not assume a particular device's grid cap.

```sh
cargo oxide test --arch sm_89 -- --release -p gpu-datalog-runtime
cargo oxide test --arch sm_89 -- --release -p gpu-datalog-runtime --test runs -- --ignored --nocapture
```

The ignored probe is a runtime workload, not acceptance evidence. Slice D's
2026-11-02 acceptance is recorded in
[`compare/p4-gpu-d.results.json`](compare/p4-gpu-d.results.json): GPU runtime,
Datalog core/facade and Sequence suites pass; xalan and batik each match an
independent CPU reference on all **43 duplicate-free output tuple sets**,
and Cassandra matches all **26 complete tuple sets**. Root, quick (15 targets),
formal and API gates pass; all **46 CPU expansions are identical** to `b7e83c6f`.

Five interleaved process-isolated pairs per dataset held the GPU lock, then
the timing lock. Every sample checked complete answers. Times below are only
`GPU_DATALOG_PHASES=1` **fixpoint** means; exports are outside that interval.
The ratio is current/baseline, with a paired-bootstrap 95% confidence interval.

| Dataset | `b7e83c6f` seconds | Slice D seconds | Ratio [95% CI] |
| --- | ---: | ---: | ---: |
| xalan | 0.877541 | 0.886834 | 1.01059 [0.99901, 1.02241] |
| batik | 1.554275 | 1.525438 | 0.98145 [0.97875, 0.98556] |
| Cassandra | 1.112116 | 0.555870 | 0.49983 [0.48944, 0.51058] |

Xalan is neutral/slightly slower, not a demonstrated speedup. Batik is modestly
faster. Cassandra was repeated after rebuilding the baseline with the same
`oxide run` materialized-cubin workflow; the second series above retained the
improvement. Both series and executable hashes remain in the evidence.

Nsight Systems 2025.3.1 attribution covers **all process GPU work**, including
setup/output, not only fixpoint. These kernel totals cannot be subtracted from
paired wall time. Direct sort/lane-gather/unique-mark work and explicit layout
permutation/merge/metadata work were:

| Dataset | Sort/unique marks, before → after (ms) | Layout maintenance, before → after (ms) |
| --- | ---: | ---: |
| xalan | 227.111 → 227.019 | 18.504 → 25.971 |
| batik | 240.230 → 241.200 | 40.322 → 55.706 |
| Cassandra | 57.215 → 57.227 | 8.404 → 11.476 |

Shared scan/select kernels are reported separately rather than assigned
speculatively to uniqueness or admission. Sort/unique-mark instance counts
are unchanged. The new unit-offset metadata adds 4,269 / 7,085 / 3,132 launches
on xalan / batik / Cassandra. Cassandra's generated relational kernel time is
nearly unchanged, while observed module load/unload calls fall **158 → 6**;
its wall-time improvement is not a sort/unique optimization. Xalan/batik
relational kernel totals decrease, so this evidence does not motivate
nonzero-directory retuning merely to explain xalan's neutral total.

**Slice E:** target recorded active-bit/key-only sort specialization and
reuse or elimination of materialized unit offsets under an explicit checked
representation; then reduce repeated secondary-layout publication work.
Rank directory choices by matched build/read/maintenance cost, not the
provider's name. Per-command gate times and raw kernel attribution accompany
the results; historical measurements elsewhere remain historical.

### Radix relation hole inputs and candidate arenas

A checked `radix_relation` input binds a complete-key `RadixLayout` snapshot
(`GPU-Runs`). Scans visit every run; logical key-prefix lookups encode probe
operands once and search an inclusive key interval per run, with block-uniform
expansion. Membership ORs over runs, and only demanded fields are decoded
from complete keys; no fictitious item payload is read. A recorded lookup on
nonleading columns obtains a sorted, permuted provider view that retains each
original run's rows. Unused selection fallbacks do not construct providers.
Scalar widths come from the native `Scalar` provider trait; codec-declared
symbols use u32 codes, with no 128-bit tuple cap.
`cq_gpu_sequence::translate_arena(&mut arena)` (`GPU-Arena`) publishes head rows
packed most-significant-first in head order into a `CandidateArena` and returns
a `CandidateBatch` (u64 lanes, native low-first for u128 and most-significant
first for larger words; length; packing; retained owners). The hole must be
consumer-canonical: the batch is a candidate bag for canonical `fresh`.
Sizing keeps the dispatch's output protocol, so each attempt reads its
metadata back; candidate rows are never downloaded. Arena invocations omit
the redundant final stream synchronize and phase timing events by default.
`cq_gpu_runtime::enable_arena_timing(true)` opts the current thread into those
events, the final fence, and `take_last_timings()` publication. Required metadata
validation and device overflow/failure checks are unchanged; retained owners
and the pool stream order protect asynchronous work.
The explicitly recorded `output reserve_previous` protocol is supported only by
the arena terminal. Its initial capacity is
the arena's last validated row count for the selected Round hole, not a
relation-wide count or a proven bound (zero initially or after `clear`).
Keys-only attempts may use the full capacity of the leased slot.
Each attempt counts all rows and checks sticky failure flags; an undersized
attempt is discarded and replayed against unchanged inputs at the required
capacity before publication. Shrinking, growing, and empty rounds remain exact,
and retained batches cannot be overwritten by subsequent leases.
Other output protocols are unchanged.
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
    incremental = ::cq_datalog::passes::incremental(driver(delta_first););
    storage_rewrites = [::cq_gpu_datalog::device_storage];
    cq = pipeline {
        physical = [::cq_gpu_sequence::capture_constants];
        structures = [
            ::cq_gpu_datalog::bind_inputs,
            ::cq_gpu_sequence::bind_runs,
            ::cq_gpu_sequence::select_catalog,
        ];
        stage = ::cq_gpu_sequence::stage;
        sequence = ::mini_linq::passes::sequence;
        decompose = [
            ::cq_gpu_sequence::decompose,
            ::cq_sequence_rewrites::output(reserve_previous),
            ::cq_sequence_rewrites::roots(spread),
        ];
        grade = ::cq_gpu_sequence::machine_grade;
        print = ::cq_gpu_sequence::translate_arena;
    };
    terminals = [::cq_gpu_sequence::translate_arena => batch];
    round = [
        ::cq_gpu_datalog::bind_round(binding = 5),
        ::cq_gpu_datalog::lanes(streams = 4),
    ];
    print = ::cq_gpu_datalog::print_rounds;
}
```

This configuration follows [`srd-doop/src/lib.rs`](srd-doop/src/lib.rs);
[`doop/src/lib.rs`](doop/src/lib.rs) additionally declares its symbol codec.
`reserve_previous`, `roots(spread)`, and `lanes(streams = 4)` are explicit
benchmark choices, not implicit defaults. The lane rewrite records independent
holes of a wave on worker streams sharing the allocator; fork/join events order
their work before main-stream admission. It does not run dependent waves in parallel.

The GPU Structures stage validates target support without inheriting the CPU
batching policy. GPU batches are selected by Sequence decomposition; explicit
unsupported Structures witnesses, stages and join filters are rejected rather
than silently discarded.

`bind_round` records complete input-layout and arena argument packets before
printing. Its zero-based invocation ordinal names the binding rewrite after
plan, index demands, lower, capture constants, and select in this queue.
Update that ordinal when inserting preceding phases; aliases do not change
ownership. The printer rejects missing owners or modified packets.
`device_storage(ratio(8))` overrides the geometric maintenance ratio; omission
records four. The ratio is printed in Storage and passed to every run state.

Device Storage records every provider, initializer, layout state, candidate
container, arena and scratch policy; the printer composes only those records.
Inputs upload once and are admitted into the canonical layout; each round
issues the recorded waves in order and, after each member's recorded
publication wave, computes fresh = unique candidates minus the committed
canonical layout, appends it to every full layout, builds NEW from it and
moves NEW to the delta; outputs read the canonical layout once. All programs on one thread share
`gpu_datalog_runtime::thread_pool()` (device 0), so the runtime's thread-local
caches never pin a finished program's pool.
Candidate batches are released after admission. Multistream execution retains
component-scoped arenas per lane/head and hole-local previous-count hints;
completed strata release cached Sequence scratch and wholly unused pool chunks.
Layout states and views retain their published owners; trimming never
invalidates live device views.

Symbol columns are declared with
`device_storage(symbols(IStr => MyCodec))`, where
`MyCodec: gpu_datalog_runtime::SymbolCodec<IStr>` supplies a process-fixed,
injective `code(&IStr) -> u32` mapping and its `decode(u32) -> IStr` inverse.
Use the same type spelling as the relation declaration; the compiler does not
resolve Rust aliases. Upload/download performs the conversion on the host.
The declared codec map is a printable Storage field, independent of opaque
provider types; `DeviceRadixRun<W, u64, ()>` has no logical-schema metadata.
Closed guard, binding and selection expressions are host captures, evaluated
once per invocation on demand for a nonempty dispatch input. Invocation-local
`OnceCell` values retain captures across retries and dispatches; symbols use
the column's codec, and kernels receive only integer parameters.
Query-level answer gates are recorded separately and short-circuit before later
captures, inputs or GPU work; false gates publish empty rows or arena batches.
Host upload subplans reuse once-bound inputs and captures. Closed relational
constraints with no row operands capture one host Boolean result. Grouped output
with a query answer gate is currently rejected before printing.
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
Radix controls, merge partitions, selection counts and native runtime scratch
allocate through the same RCCL pool and are included in its counters.
Enable the `gpu-datalog-runtime/radix-counters` Cargo feature to emit separate
setup/fixpoint counters for canonical, layout and occurrence sorts, merges,
grouping readbacks, descriptors and offset materialization. With the feature
disabled, recording, resetting and tracing are no-op stubs, with no runtime
enable check. Radix `passes` counts planned 8-bit digit equivalents (small
tile kernels may fuse them); `bytes` models radix input/output traffic, not
measured DRAM bandwidth, and excludes gather/compaction/histogram/copies.
Run counters/profilers separately from timing.

Device Storage records each complete-codec active-bit expression, the
`native_keys_or_lanes` sort form, implicit-unit set offsets, and certified
admitted-row permutations. Canonical sorted/unique input is adopted; each
other admitted order is sorted once and shared by full/delta views.
Widths come from declared scalar/codec metadata, never observed row maxima.
The ordinary occurrence API keeps materialized group offsets; a full-key
`new_set(pool, params, active_bits)` publication carries an explicit set witness.

Slice E's 2026-11-02 matched five-pair measurements versus `1b1540bc`,
same-source linker-shuffle controls, radix-work counters and exact exports are
recorded in [`compare/p4-gpu-e.results.json`](compare/p4-gpu-e.results.json).
The combined change is retained at fixpoint ratios 0.96599/0.95177/0.96163
for xalan/batik/Cassandra; sort counts stay unchanged while active radix
passes shrink and unit-offset materialization disappears. All 104 TPC-H
advised/automatic SF1/SF10 cases are exact. Its separate advised H05 SF10
performance ceiling already fails on pristine baseline (951.291 ms versus
current 952.262 ms); those executable nonzero exits remain recorded.

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
The storage declaration explicitly assigns `DoopSymbols` codec id 0. The
pipeline's decomposition catalog supplies an ordered `CodecToken` for that id
at `[host]`, `[host, device]`, `[host, device, block]` and
`[host, device, block, lane]`; declaring a codec in storage alone does not
authorize its use at machine levels.
Closed symbolic answer expressions retain their answer-slot logical identity
after capture hoisting. The GPU carrier recorder uses that identity's codec
for arena metadata instead of requiring `IStr` itself to implement `Scalar`.

The 2026-11-02 codec-admission repair built `doop-gpu` and `srd-doop-gpu`
with materialized sm_89 cubins (Gate time **81.25 s**). Cassandra matched
all **26/26 complete DOOP output tuple sets** (Gate time **251.97 s**,
including shared-lock wait). The SRD xalan executable also completed
(Gate time **256.53 s**, including lock wait); that invocation reports counts,
not an external exact-tuple comparison. These are correctness/build gates,
not controlled performance measurements.

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

#### Relational heads (2026-10-31)

Historical `c1b75718` comparison. Later same-workload P6b acceptance
([planning record](../../doc/PLANNING-REDESIGN.md), 2026-10-31) records Cassandra
fixpoint **1.668 → 1.205 s**, four streams versus one, ratio
**0.723 [0.702, 0.765]** over five locked pairs, with all **26/26 outputs exact**.
This is a fixpoint comparison, not an update to the evaluation-time rows below.
The following Cassandra sections retain separate older toolchain, loading,
code-generation and scheduling experiments; they are not today's performance.

Since contract 7 (`0124315f`) every Rust expression is opaque, and Rust that
reads a codec-encoded column must run at host (BE-RustRepresentation). The DOOP
source wrote every head as an Ascent copy (`varpointsto(*heap, *to) :- ...`),
so each head column became an opaque binding `__cq_k_j: IStr = (*x)` over a
Symbol-encoded column. Placement put it at host, mid-pipeline where its
variable is bound: all 155 holes downloaded complete rows, ran the copy at
host, and re-encoded and uploaded the result (`upload_numeric_columns`) every
dispatch. Before contract 7, `capture_binding` recognized `*x`; at `4944b917`
no hole had a host stage.

The DOOP source now writes heads as variables (`varpointsto(heap, to)`), a
relational projection; Datalog fork projects batch key-hole columns as plain
variables; and each Round hole records its answer ownership
(DL-AnswerOwnership: a host-interface head variable defined by a positive atom
is borrowed, everything else owned), which CPU admission uses to build owned
keys and rows. All 155 Sequence programs have no host stage.

Cassandra, `--gpu-only`, five interleaved pairs against main (`aad2670d`
binary), RTX 6000 Ada, other CPU work running:

| | main | relational heads | new/main, mean (bootstrap 95% CI) |
|---|---:|---:|---:|
| fixpoint (median) | 6.849 s | 1.942 s | 0.284 (0.276–0.292) |
| GPU evaluation (median) | 7.133 s | 2.251 s | 0.315 (0.306–0.324) |
| minor page faults / max RSS | 663K–767K / 2.39 GB | 435K / 1.74 GB | |

One exact-comparison run matched `doop-mini` on all 26 outputs. Nsight Systems
fixpoint windows (profiled 8.16 s → 2.29 s): 307K → 34.5K kernel launches,
device-to-host 1,392 → 0.2 MB, host-to-device 1,719 → 8.5 MB, `encode` /
`numeric_bounds` launches 44,439 → 7 each. `4944b917` measured
2.12–2.14 s fixpoint in the same session. Per-hole attribution of the
regression (top holes, main vs `4944b917`), the paired timings and the profile
summaries are in
[`compare/doop_cassandra.relational_heads.results.json`](compare/doop_cassandra.relational_heads.results.json).

#### Historical delta-driven rule scheduling (2026-10-26, `2bfb7f23`)

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

#### Earlier measurements (2026-10-26, `9245cdb5`)

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

#### Historical ahead-of-time cubin loading (2026-10-26, `7901fb3f`)

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

#### Historical AOT code-generation repair (2026-10-26, `14ec44d2`)

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

The fork revisions used for that historical measurement were cuda-oxide
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
`select_catalog`, `reserve_previous`, `roots(spread)`, `translate_arena`,
`lanes(streams = 4)`, and `print_rounds`, like `doop-gpu`.
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
cargo test --release --manifest-path ../../Cargo.toml -p srd-doop-harness -p srd-doop-mini -p srd-doop-mini-par --features phase-timing
cargo build --release --manifest-path ../../Cargo.toml -p srd-doop-harness -p srd-doop-mini -p srd-doop-mini-par --features phase-timing
../../target/release/srd-doop-mini PREPARED --export OUTPUT
../../target/release/srd-doop-mini-par PREPARED --threads 15
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

**Later same-workload evidence (P6b, 2026-10-31):** the
[planning record](../../doc/PLANNING-REDESIGN.md) records locked four-stream
fixpoint results of **0.905 s xalan** and **1.580 s batik**, versus one-stream
**1.046 s / 1.947 s**, with paired ratios **0.865 [0.852, 0.892] /
0.811 [0.800, 0.816]** and **43/43 relations byte-identical**. These supersede
the P3.2/P3.3 snapshots as later recorded timings of this workload, not as
measurements of every subsequent revision. Raw P6b captures are referenced by
the planning record under `/data/p6b-work`; the summary is checked in, but this
README audit did not revalidate those external captures.

**Host preparation follow-up (2026-11-02):** against rebuilt main `20101707`,
five interleaved locked pairs measured xalan **0.912370 → 0.904628 s**
(paired ratio **0.99133 [0.98154, 1.00028]**) and batik
**1.609418 → 1.588257 s** (**0.98519 [0.97691, 0.98829]**).
The 26-output Cassandra workload measured **1.331751 → 1.311806 s**
(**0.98218 [0.95805, 1.02116]**). Only batik is statistically faster;
xalan and Cassandra are neutral. GPU-ProgramLaunch records and caches the
validated launch envelope per compiled CUDA entry/context, including opaque
callable specialization; emitted scalar-ABI wrappers retain the same grid
caps and count/failure readbacks. All timed samples had exact relation counts.
The dated P6b plan entry records the separate setup attribution and artifacts.
Complete sorted exports matched **43/43** relations for both xalan and batik;
both binaries matched the CPU's **26/26** complete Cassandra output sets.
The comparison checked duplicate-free exported counts as well as tuple hashes.

**Fresh paired run (2026-11-02/03, `44bf82bc` vs `c1b75718`):** see
[`compare/bench-2026-11-02.md`](compare/bench-2026-11-02.md). GPU TPC-H advised ran
on a RunPod RTX 6000 Ada ([`compare/pod_bench_2026_11_02.py`](compare/pod_bench_2026_11_02.py)),
because a foreign tenant occupied the local GPU. Every query was exact. SF1
geomean was **1.085 [1.033, 1.120]** (5 pairs) and SF10 **1.023 [1.000, 1.054]**
(4 pairs). Unadjusted increases: h09 2.52/2.34, h21_positive 2.07/2.09, h10
1.55/1.61, h03 1.26/1.22, h04 1.15/1.16. Improvements: h12 0.456/0.423, h11 SF10
0.416, h02 SF10 0.735. A second pod session timed `doop-gpu` xalan/batik
(26/26 exact against `doop-mini`). Fixpoint cur/ref was xalan 0.239 [0.150,
0.599] and batik 0.724 [0.482, 0.836]. Treat both as low confidence: the same
binary varied up to 10x between fresh processes (xalan reference 3.8–31.4 s),
cause not established. Live SRDatalog and `srd-doop-gpu` were not part of the pod
runs (no SRD toolchain or prepared corpus there). A later local comparison is
recorded below; do not mix its field-flow program with the historical SRD
default-query references.

These historical ratios use mismatched boundaries: current includes terminal
host downloads in build/total, while the reference excludes them. They are
biased against current and do not establish execution regressions. Current
reports separate final `download_ms` and expose executed stage accounting.
**Fresh local SRD comparison (2026-11-03):**
[`compare/srd-local-2026-11-03.md`](compare/srd-local-2026-11-03.md) measures
unmodified `dbe4c2cb` against live SRDatalog `fab849f` on the same RTX 6000 Ada
and the same 43-IDB field-flow corpus. Xalan fixpoint medians are **0.880 s**
(`srd-doop-gpu`) and **1.187 s** (SRD), ours/SRD **0.741 [0.598, 0.943]**,
five rotating paired rounds. First samples matched all 43 complete IDB sets;
later samples checked exact counts and seeded actual tuples. Batik completed
one exact observation (**1.929 / 1.844 s**) before the user stopped further
runs: no Batik ratio or CI is claimed. End-to-end figures are explicitly
boundary-confounded and shown phase-by-phase, without a speedup ratio.
The separate 26-output `doop-gpu` workload has no ratio against SRD.
These are the latest live observations; the Oct 26 **default-query** ranges
remain historical, not paired field-flow evidence. The report also records
the default planner's late root-bound eligibility test in two array-store
delta variants; no planner or production code changed in this measurement.

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

### Planner-driven 26-query TPC-H suite

`tpch` is one crate with one binary, `cq-gpu-tpch`. Its authoritative frontend
is the 26 authored `tpch/queries/*.stages` programs. CPU workload Rust remains
an independent reference and is not parsed by the GPU compiler. The checked,
printable stage IR explicitly declares producers, captures, dependencies,
owned publications and terminal projections; embedded Rust stays opaque.
Each CQ stage uses the checked CQ → VO → RA → Physical → Structures passes,
catalog selection, Sequence lowering, grading and translation.
The default build generates only `advised`
executors, using explicit pipeline choices followed by decomposition-only
completion. Automatic generation is opt-in: build with
`CQ_GPU_TPCH_AUTOMATIC=1` to also generate `automatic` executors using
`cq_sequence_plan::decompose` plus existing grading. The advised recipe
path remains unchanged; automatic Sequence relational replanning is removed.
The manifest loader records provider encoding facts separately from logical Rust
types. Relational-only columns may use manifest scalar or dictionary codes, with
closed `where` operands encoded on the host. Rust arithmetic and conditional binds
are never rewritten into dictionary operations; logical host adapters handle
expressions reading nonidentity columns. Neither loader nor translator chooses a plan.

The default loader reads each declared table projection. Dictionary-domain
columns retain published `u32` codes; explicit domain identities, not Rust
type spellings, select this representation. Shared CQ domain normalization
captures all closed operands once on the host, using exact/lower/upper runtime
operations for sorted dictionaries. Output-only dictionary domains are not
evaluated. Integer-only relations retain resident uploads and share columns
between aliases. Dates use i32 epoch days and decimals use scale-2 integers
when the recorded scalar representation supports relational-only reads.
Catalog provider tokens declare their native ABI, contiguous-range promise and
bounds source. Loader-authored input instances record their logical/storage
columns and codecs; no ABI or scalar support is inferred from Rust spelling.
Non-run membership probes compare every key in the full-width row carrier;
secondary keys are not narrowed to u32. Contiguous selection bounds intersect,
including equality constraints, so contradictory selections remain empty.
An unmapped live generated row read is a compilation error rather than a
zero-valued fallback. GPU launch checking independently validates recorded
offset and synchronization obligations and requires positive tile sizes.

Opaque text predicates are authored as dictionary input producers and receive
each distinct string once; their code sets feed relational operations.
Arbitrary host producers declare dependencies explicitly, retain their Rust
payloads, and convert domain-marked columns at publication. Terminal projection
slots and dictionary domains control final text publication. Neither a method
named `literal`/`publish` nor a function named `dictionary_where`/`date` is
recognized by the frontend. The synthetic String-guard regression checks the
declared dictionary producer and closed-operand boundary against Sequential
and explicit expected keys.

All generated TPC-H queries, including H05 in both modes, use the shared
runtime Offset/Rank domain loader. Integer-key membership is static schema
data; minima, significant bits and sparse Rank mappings are frozen at load.
GPU and Sequential outputs are decoded back to source keys before comparison.
H05 has no separate catalog/profile path. Its advised pipeline records Region → Nation → Customer → Orders →
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
`tools/gpu_gate.sh` runs that advised H05 SF10 acceptance command under the
same GPU lock as the crate suites and writes `target/gpu-gate-h05-sf10.json`.
Exactness-only tests are not a substitute for this timing gate. Column-backed
index sources retain a host publication grade: their recorded GPU source
provider builds consume resident column descriptors directly, without an
identity-copy device dispatch. Rebuilding a row-oriented native provider
continues to require descendant row work.

`--base-codec for|plain64|native|hostfor|delta` selects the resident layout
of base columns. Domain-normalized integer keys currently require `for` or
`hostfor`; incompatible modes are rejected explicitly. The default is `for`.
Device readers dispatch on each column's recorded codec. `--base-cache DIR`
(only with `hostfor`) persists and reuses host-packed loader columns. Each
file is bound to its source column's path, length, mtime and stored-byte
SHA-256, and key columns are rehashed before they certify a set. Each
result row reports the query's `base_columns` load accounting (outside query
timings). `experiments/storage-bench/gpu_base_codec.py` runs rotated paired
campaigns over these layouts.


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
`segment`, `output`, `grade`, and `evidence from authored("source") { ... }`. CPU workload
`advise plan(...)` hints are not imported. GPU advice stays in its existing
explicit catalog/override interface; CPU-specific reduction hints cannot
silently replace the measured GPU baseline.

Each accepted report includes `stage_source`, `terminal_stages` and
`terminal_direct`; each entry in `stages` includes `sequence_input`,
`sequence_trace`, `sequence_planned`, publication facts, symbolic cost and a
resolved `pipeline`. For a single-stage query, copy
`{ \"query_name\": report.stages[0].pipeline }` into the override file to replay
it. A query-level override applies to every stage, so one stage's resolved
pipeline is not a replay specification for an arbitrary multistage query.
`complete: true` means the Sequence trace already records the complete
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
translations it performs one warmup plus five measured iterations. The shared
`testkit/bench-check` helper fully compares the first measured sample of each
binary/configuration against `answer.retail_<query>.tsv` and its `.count`;
later samples check exact answer counts and up to 1,000 seeded actual tuples
for reference membership (all if smaller). References are indexed outside the
sample loop. Reports distinguish full and sampled checks, with seed, counts and
validation seconds; historical result files retain their original policy.
The binary reports median CUDA-event total/build/kernels/dedup milliseconds.
Input loading, answer download and validation are outside those event intervals.
Conformance tests remain fully exact. SF1 also
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
The historical cache-lifetime investigation (2026-10-24, `dc518657`) found that
a fresh worker thread let H09/H21's large downloaded answers fragment its malloc
arena, delaying subsequent host-publication allocations inside the build interval.
That four-query subset showed H09 totals around 45–66 ms, with an
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
(`dc518657`, 2026-10-24; not a current inventory or a remeasurement of the later
authored-stage suite) record all 25 compiled queries exact in one process, with zero free-memory
delta after every query (49,617,043,456 bytes free before and after each).
Before the fix, per-expansion thread-local `RangeWorkspace` and
`PackedColumnsAdapter` owners kept their pools' arenas alive across queries:
free memory fell to 814,219,264 bytes and D85 failed with an OOM. Explicit
release hooks remove those owners without changing plans or kernels. The
merged cuda-host module cache is bounded per native context and needs no
external API change. That historical full inventory exited 1 for H14/H19
translation rejections, not a runtime failure; its H05 timing gate passed at
10.275 ms. That run's full-suite totals versus
`performance_report.md` are H05 10.275 vs 10.314 ms, H09 44.588 vs 44.563 ms,
H21-positive 40.053 vs 40.158 ms, and D85 26.925 vs 26.935 ms.
Statistics-derived H07 advice and its comparison with H07-block are recorded in
[`compare/tpch_statistics.md`](compare/tpch_statistics.md).

H14 is absent from the current authored-stage inventory
([`tpch/src/source.rs`](tpch/src/source.rs)); no GPU support claim is made here.
Current H19 declares explicit dictionary domains and `StringCode` columns in
[`tpch/queries/h19.stages`](tpch/queries/h19.stages), with relational string
guards and a revenue aggregate. The loader's `dictionary_types` consumes
declared domains, not special recognition of an `intern(...)` function.

The following H19 measurements are the historical author-interned query
checkpoint (`4cbe5eea`, 2026-10-24), before the current staged query contract.
They are not timings or output counts for today's H19 aggregate:
The [author-interned full-suite run](compare/gpu_tpch_intern.results.json) passed
all 26 supported queries at SF1 and SF10 in one process (52 exact results).
H19 total/build/query medians are SF1 9.254/0.002/9.253 ms and SF10
24.375/0.002/24.373 ms; [the performance addendum](compare/performance_report.md#author-interned-h19-addendum)
compares these with the report's DuckDB t1/t24 baselines.

Historical initial inventory (retained README account at audit revision
`7e187597`, not an independently located raw acceptance log), before subsequent
translation fixes: all eleven queries were rejected at both scales, so **no GPU
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

The exporter reads authored declarations and SQL from
`features/workloads/src/queries/retail/tpch/*.rs`, including both
`retail_kernel!` and explicit `NAME`/`SQL`/`relation_loaders!` forms.
The current corpus includes grouped folds and negation as well as positive CQ
kernels; it is not uniformly the original TPC-H/TPC-DS SQL. The GPU frontend
still reads `.stages`, not CPU Rust. See [`export_tpch.py`](compare/export_tpch.py).

The column table below describes the historical initial positive-query export,
not the complete current union. Current exports derive their column inventories
from those declarations and record each query's `required_columns` in the manifest.

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

`manifest.json` schema version 3 (version 2 remains readable) contains `sf`, `byte_order: "little"`,
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

Version 3 adds per-relation `verified_keys`: each entry has `set: "verified"`,
`rows`, `types` keyed by column name, and `columns` mapping each key column to
the SHA-256 of its stored bytes. Preparation sorts the actual key tuples once;
declared TPC-H constraints are only candidate keys, never trusted evidence.
Both native and Source/Catalog loaders hash the bytes they decode and omit
projection deduplication only when all key columns are loaded with matching
hashes, widths and row count. A projection such as H03 Lineitem, which omits
`l_linenumber`, still deduplicates. Dictionary keys and host-built relations do
not use this shortcut. No query/compiler annotation is created.

To prepare manifests without modifying shared datasets (requires NumPy):

```sh
python3 experiments/gpu/compare/export_tpch.py --root /data/tpch-gpu --sf 1 10 \
  --manifest-only /data/design-notes/dataset-manifests
TPCH_GPU_MANIFEST=/data/design-notes/dataset-manifests/sf1.json <SF1-loader-command>
```

`Manifest::load_with_manifest(root, path)` and
`Source::load_with_manifest(root, path)` provide explicit overrides. Relative
column paths always resolve against the dataset root, not the override's
directory. The dataset owner may adopt the generated `sfN.json` as that scale's
`manifest.json` after review; ordinary export also publishes verified keys.

Every `queries.<query>` entry records source path, exact SQL,
`required_columns` grouped by table (the complete per-query input inventory),
answer filename, distinct answer count, and timing filename. Answers are
`answer.retail_<query>.tsv` plus `.count`, sorted by all output columns.
`duckdb_timing.retail_<query>.json` retains the original timing schema:
engine/version, SQL, seconds, one warmup and five measured complete SELECT
runs (including grouped folds), and thread-keyed warmup/samples/median at 1 and 24 threads. Raw CLI logs
are retained per query/thread count; timings include normal result emission,
not a COUNT wrapper or the separate sorted answer export. H05 additionally
retains `answer.tsv`, `answer.count`, and `duckdb_timing.json`.

All pre-existing `.bin` files are SHA-256 checked before and after export;
`binary_checksums.json` records both maps and requires equality, preserving
the H05 binary inputs byte-for-byte. The manifest is published only after all
columns, checksums, answers, and timings complete. Temporary projected TSVs
are removed after conversion.

Historical initial positive-query export: distinct counts and DuckDB median
wall times in seconds (five samples after one warmup). These tables and the
byte totals are retained from the README at audit revision `7e187597`; the
original local manifests/logs are not checked-in evidence here, so their
measurement date and exact raw provenance remain unverified. They do not
describe the expanded staged suite or establish current directory sizes.

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

That initial export was reported to have passed before/after SHA-256 equality
for all 13 pre-existing H05 `.bin` files per scale and to contain 11 query
inventories and 8 tables. This is a historical report, not the current manifest inventory.

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

### Historical H05 step-order experiment (P3.2, 2026-10-30, `7a1c19df`)

The tables below retain the pre-P3.3 experiment. The P3.3 evidence table above
records later step-R results with retained-prefix/fan-out grading; do not read
the old “Automatic/default” row as the current default's performance.

The opt-in workload configurations `step_r`, `step_r_lookups`, and
`step_r_retained` leave existing configurations unchanged. `step_r` requests
`no_reduce, order(Region, Nation, Customer, Orders, Lineitem, Supplier)`.
The final Supplier read introduces no variable: it checks the full
`(nation_key, supplier_key)` cycle key. There is no k-way intersection in R.
`step_r_lookups` first checks Customer and Supplier on `nation_key`, then
expands Customer and completes Supplier after Lineitem.

GPU overrides are [`tpch/h05_step_r.json`](tpch/h05_step_r.json) and
[`tpch/h05_step_r_lookups.json`](tpch/h05_step_r_lookups.json), selected with
`TPCH_GPU_ADVICE` while compiling only H05 (`CQ_GPU_TPCH_QUERIES=h05`).
Both use default Sequence decomposition, not an authored Sequence or a second
`plan(machine)` reorder, so the CQ-advised R traversal is preserved.
The lookup variant additionally needs `build_host_reads()`; without it,
catalog selection rejects `r6: missing retained rows type`.

On RTX 6000 Ada, five interleaved forward/reverse blocks gave the following
median milliseconds [95% percentile-bootstrap CI of block medians].
GPU blocks contain five measured runs after one warmup; CPU blocks contain
one measured run after one warmup with the default worker and one Rayon
thread. Every measured answer was checked exactly (7,242 SF1 / 72,983 SF10).
Input loading is excluded; query index/provider builds are included.

| Variant | CPU SF1 | GPU SF1 | GPU SF10 |
|---|---:|---:|---:|
| Automatic/default | 39.918 [39.325, 41.375] | 7.413 [7.403, 7.430] | 68.714 [68.644, 68.821] |
| Existing advised R recipe | not a CPU configuration | 1.190 [1.186, 1.216] | 8.617 [8.515, 8.917] |
| Existing CPU `advised` (Lineitem-first, **not R**) | 37.851 [35.791, 38.471] | — | — |
| Step-order R | 387.729 [381.005, 392.901] | 10.005 [9.998, 10.015] | 124.022 [123.751, 124.270] |
| Step-order R + early cycle lookups | 395.083 [386.620, 402.455] | 14.081 [14.076, 14.094] | 176.169 [175.951, 176.228] |
| Step-order R + `retain_lookups` | 395.679 [387.584, 397.264] | CPU-only | CPU-only |
| Step-order R + Lineitem `join_filter` | rejected: no covering exact probe/semijoin | CPU-only | CPU-only |

The filter rejection is intentional evidence, not a fallback: R materializes
Lineitem as a keyed right-hand provider, rather than a scan covered by a
downstream scalar probe. Changing its order to make a filter admissible would
measure a different variant. Likewise, the existing CPU `advised` configuration
must not be mislabeled as the GPU R recipe.

The timing result rules out CQ order alone as the explanation for the
automatic-versus-R GPU gap. Preserving R without its scheduling recipe is
slower than automatic: its Sequence tiles the Region root (five rows, one
ASIA survivor), with all later fan-outs nested underneath. The existing
R recipe instead stores the Customer prefix, then segments Orders and
Lineitem expansion into independent work. This placement of parallel work,
plus provider/materialization choices and full-key Supplier membership,
distinguishes the fast R recipe; the experiment does not individually
attribute the remaining cost to each choice. Early cycle lookups prune no
selected nation here. On CPU, disabling reductions and building the R-side
indexes is substantially worse than the default reduced plan. No automatic
planning policy is changed on the basis of this result.

A diagnostic run with `recipe.automatic=true` is retained separately in the
evidence: it invokes `plan(machine)` after CQ advice and rewrites R into a
Lineitem/date-root schedule. Its SF10 medians were 59.269 ms (R advice) and
61.732 ms (R+lookups), **not** timings of the preserved R traversal.

Separate CPU expansion-count runs report the same largest preclosing stage,
184,082 children / 7,242 distinct outputs = **25.419**, for all five measured
CPU configurations. R's expansion stages are Nation 5, Customer 30,183,
Orders 46,008, and Lineitem 184,082; the cycle closes at 7,243 rows before
distinct. Its early Customer/Supplier nation lookups both retain all five
nations. This is not a CPU fan-out reduction: the default already uses the
ASIA/date reductions. Legacy operator counters instead show peak retained
bytes increasing from 5,172,200 (default/advised) to 132,807,464 (R and
R+retention), or 133,104,440 (R+lookups). That difference corroborates the
index-build/materialization explanation for the CPU regression.

The GPU's largest individual preclosing stage and final closing **bag**
output per invocation are below. Counter ratios use bag output, unlike the
CPU final-iterator counter above; distinct GPU answers remain 7,242/72,983.
All values come from separate `CQ_EXPANSION_COUNTERS=1` builds, not timings.

| Variant | GPU SF1 children / closing output (ratio) | GPU SF10 children / closing output (ratio) |
|---|---:|---:|
| Automatic | 36,718 / 7,243 (5.069) | 366,425 / 72,985 (5.021) |
| Existing advised R recipe | 184,082 / 7,243 (25.415) | 1,825,856 / 72,985 (25.017) |
| Preserved step-order R | 184,082 / 7,243 (25.415) | 1,825,856 / 72,985 (25.017) |
| Preserved R + early lookups | 184,082 / 7,243 (25.415) | 1,825,856 / 72,985 (25.017) |

For preserved R, the stages are Nation 5/5, Customer 30,183/300,270,
Orders 46,008/456,771, and Lineitem 184,082/1,825,856 (SF1/SF10).
The lookup variant adds a final 7,243/72,985 stage but does not reduce any
earlier stage. Automatic's forward-provider rewrites produce smaller
individual expansion stages yet run much slower than the R recipe:
minimizing the largest fan-out alone is not a valid GPU cost model here.

[Raw samples, bootstrap intervals, exactness, all per-stage counts, plans,
and reproduction commands](compare/h05_step_advice.results.json) retain
both the preserved-order experiment and the automatic-rescheduling diagnostic.
GPU counter reports aggregate six invocations and include normalized
per-invocation values. Intermediate definition outputs count publication
units, which need not equal flattened tuples (the R Customer-prefix store
reports five units although its child counter is 30,183 at SF1); only the
final closing output is used in the ratios above.

## Authored advice and rejection boundaries

The resident H05 A/B/R and identity-chain callers author their Sequence programs
directly, then apply checked store/output/grade rewrites. They do not infer
grades or provider choices from generated temporary names.

| Former decision | Recorded Sequence decision |
|---|---|
| Retained prefix/build | Explicit `store` at its authored level |
| Sorted/prefix lookup representation | `RangeProvider` with the same recorded key levels |
| Expansion batch and `tile(256,4)` | Explicit `tile 1024 items 4` and flatten boundaries |
| Device deduplication | `canon` with the constrained `radix_set` full-row `RadixLayout` token |
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
`triangle-compiled EDGES --check` uploads native i64 IDs, executes the default
`probe first` plan over Range providers, and compares actual GPU tuples with
its CPU reference (`triangle-smallest` is the authored smallest configuration).
It reports one discarded warmup and five measured resident calls, including
per-call and median GPU build/kernel/dedup/total timings and query wall time.
`--check` separately times the CPU tuple reference and checks every GPU call.
Input loading/upload, answer download, and reference checks are excluded from
the query samples.
The recorded directory requires a dense domain fitting its u32 addressing;
signed keys are normalized by the provider's recorded `min(0, bounds.min)` base.
Domain and sentinel sizes are checked, and payload values remain signed.
Canonical publication uses ordinary full-row/unit-item `RadixLayout` sets.
The constrained `radix_set` token promises Distinct, unlike the generic
occurrence-preserving family. Shared stable radix ordering compares every
lane, including headings beyond 128 bits. `SetBuild` records checked construction
parameters, including block width; the consumed FOR codec's scalar or resident
domains determine the lane extent at setup. No schema narrowing, type-syntax
recognition, ID remapping or alternate query plan is substituted. Counts stay
resident; sorted unique owned prefixes can be adopted without a sort/copy.
Per-thread/pool sort, permutation, compaction and count scratch is reused on the
pool stream; immutable publications own separate keys and metadata. Smaller
calls sort only their live prefix. `QueryScope` releases cached scratch without
a host wait; direct runtime callers can call `radix_run::release_set_scratch`.
Radix controls use grow-only reuse per thread, pool and key/pair policy; their
allocator and stream remain owned until scratch release. Sequence and reference
runtime temporaries, generated group flags, and lexicographic-sort scratch also
use the execution pool rather than CUDA's separate async allocator.

Allocation acceptance on xalan (`doop-gpu ... --gpu-only`, GDB breakpoints at
`cuMemAlloc_v2` and `cuMemAllocAsync`, each recording size and `bt 8`):
base `2f3588aa` made 1,977 synchronous calls (1,968 radix-control allocations
and nine pool-growth calls) and 5,504 async calls (5,482 Datalog native scratch,
22 Sequence scratch). The pooled implementation made nine synchronous calls,
all RCCL pool growth at 1, 2, 4, 8, 16, 32, 64, 128 and 256 MiB, and zero async
calls. Full-mode cassandra and xalan comparisons matched all 26 full-tuple
output sets. The GPU had a foreign tenant; these are allocation and exactness
results, not performance measurements.

## Historical device-store and i64 acceptance

The 2026-10-22 checkpoint (`ddef2065`) recorded an NVIDIA RTX 6000 Ada Generation
(driver 580.178.04), an `sm_89` release build/test pass, 22 tests and doc tests,
and a formal inventory of 228 rules. Those counts are not current gate results;
the linked timing JSON below substantiates H05 samples, not those separate
build/test/formal claims. Positive source coverage includes provider construction
after publication, i64 triangle execution, signed payload extrema and stored
column descriptors. No checks were rerun for this documentation audit.

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

`triangle-compiled` (default) runs `probe first [b]` over Range providers
(`r [b]`, `s [a][c]`, full-row set output). `triangle-smallest` is an authored
configuration that runs `probe smallest [b]` over two radix layouts
(`r [b][c]`, `s [a][c]`, GPU-Smallest), with the same set output. Historically, with `--check`
on `/data/dataset/graph/com-dblp.ungraph.txt` (1,049,866 distinct native-i64
edges) both produce exactly 2,224,385 tuples, equal to the independent CPU
reference on every call; `triangle-smallest` is also exact on the same graph
with IDs relabeled across the full signed i64 domain
(`x * 0x9E3779B97F4A7C15` wrapping), which the dense Range directory cannot
represent.

Later paired triangle timing (`af735660`, 2026-11-01;
[`compare/p4_gpu_triangle_smallest.paired.json`](compare/p4_gpu_triangle_smallest.paired.json),
RTX 6000 Ada, timing and GPU locks held, five interleaved process pairs,
each one warmup plus five resident calls; medians of per-process medians), DBLP:

| Triangle binary (DBLP) | Build ms | Kernel ms | Dedup ms | GPU total ms | Minor faults |
|---|---:|---:|---:|---:|---:|
| `triangle-compiled` (probe first, Range) | 0.485 | 1.347 | 0.303 | 2.129 | 24,712 |
| `triangle-smallest` (probe smallest, radix) | 0.975 | 1.939 | 0.366 | 3.278 | 22,765 |

In that paired run, smallest was 1.54× slower in GPU total (bootstrap 95% CI
1.52–1.55×); it remains opt-in. The following profiling figures are retained
from the `af735660` README account, not from the paired JSON (which contains
timing samples, medians and ratios): their raw nsys/ncu captures are not cited
as checked-in evidence and remain unverified by this audit.
The first smallest kernel reportedly took 3.795 ms (2.42× total): nsys
attributed 77% of GPU time to the dispatch kernel, whose children recomputed
every side's counts and choice per candidate. The parent now counts and
chooses once and passes the choice in the child position tag
(ChoiceCarrier::PositionTag), and a one-run chosen side passes its first key
slot so children skip `distinct_at`. ncu on the remaining kernel (2.36 ms
under profiling, 64 registers, 61% achieved occupancy, 20% memory and 39%
SM throughput) indicates a latency-bound chain of dependent u128 binary
searches (prefix interval, membership, the authored `lookup s [a,c]`)
against the Range plan's one dense-directory read per edge; build also
doubles (radix grouping of two u128-keyed layouts).

The H05 samples are retained in
`compare/gpu_sequence_device_stores.results.json` and
`compare/gpu_sequence_device_stores.reference.sf{1,10}.json`.

## Historical cutover validation (2026-10-22, `358012d9`)

The checked-in [`cutover summary`](compare/gpu_sequence_cutover.summary.json)
records both workspace build/test commands passing on NVIDIA RTX 6000 Ada,
`sm_89`, with 16 main-workspace tests, one exercising three schema UI cases,
and a formal inventory of 226 rules. This is acceptance of that revision, not
today's gate status or rule count. The GPU interpretation marker lives in `sequence/core`.
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

## Historical negation GPU acceptance

The original negation checkpoint expanded the then CPU-importing suite to
15 workload sources, including H04, H04-not-exists, H16 and H22. Current
executors instead consume explicit authored stage programs; see
[`tpch/src/source.rs`](tpch/src/source.rs) for the current inventory.
The historical test exercised negative membership for the latter three and
positive membership for H04, with wildcard H22 keys as existential witnesses.
This is not a claim that the current frontend imports CPU source initializers.

The checkpoint's advised order was `Lineitem, Orders` for H04, `Orders` for
H04-not-exists, `Part, Partsupp` for H16, and `Customer` for H22, with `no_reduce`.
At that checkpoint the importer executed the original `LateLineitem` and
`BadSupplier` setup initializers once while loading. Derived-input construction,
manifest loading and initial uploads were outside its query interval; provider
construction and uploads from retained host rows, including string predicates,
were inside `build`. Current producers are explicit `.stages` declarations.
DuckDB's existing t24 measurements execute the complete independent SQL;
these are not identical end-to-end timing boundaries.

Historical release sm_89 report: one warmup and five measured iterations,
all eight answers reported exact against TSVs, with SF1 Sequential agreement.
The only cited raw artifact is the external file below; its measurement date
and results are unverified by this repository-only audit (README at `7e187597`).

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
pointers. [`prune_kernel_parameters`](sequence/core/src/emit.rs) emits a
const assertion that accumulates each retained parameter's `size_of` and
`align_of`: a parameter area of 4096 bytes or more fails compilation with
`BE-Resource: kernel parameter area must be below 4096 bytes`.
This is a generated compile-time check, not a translator diagnostic reporting
the byte demand and live parameter count. It does not move parameters into
device memory or change the recorded plan.

Catalog `levels` describe provider reads, not construction. Resident
`RangeProvider` and `radix_set` tokens are device/block/lane tokens;
host-only row providers remain host-only even when their inputs are built there.

The TPC-H build script parses its generated module as a `syn::File` and writes
`prettyplease::unparse` output. Generated-code warnings are scoped out at the
module boundary; ordinary handwritten code retains its diagnostics. The retained
README account reports an interrupted automatic-suite library compile exceeding
54 minutes, then 398.31 seconds after formatting (482.87 seconds for the whole
release build). No raw timing artifact or measurement revision is cited for
those numbers; they remain **unverified historical compilation observations**
at audit revision `7e187597`, not current compile or query performance.
