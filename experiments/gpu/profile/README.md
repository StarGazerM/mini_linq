# Resident GPU Datalog profiling

## Reproduce

Use the pinned Rust/CUDA toolchain from `experiments/gpu`:

```sh
export CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9
cargo +nightly-2026-08-28 oxide build --arch sm_89 -- --release \
  -p gpu-datalog-oracle --bin gpu-datalog-oracle --features cq-gpu-runtime/nvtx
profile/profile.sh /data/artifacts/mini-linq-gpu-profiles/DATE/oracle -- \
  target/release/gpu-datalog-oracle --bench
```

`cq-gpu-runtime/nvtx` is **off by default**. Disabled ranges are inline empty
RAII guards: no environment lookups, allocation, C compilation, NVTX linking, or
NVTX calls. Enabled ranges use static C strings and the CUDA toolkit's NVTX3
header-only implementation. Stream names are `MiniLinq resident`. The macro emits
`program`, `fixed-point`, `scc/N`, `round/seed`, `round/step`, and
`hole/HEAD/variant-N` (Datalog rules are unnamed; HEAD plus scheduled variant
uniquely identifies a rule invocation), plus negative-key holes. Runtime ranges
cover union, dedup, difference, count check, full copy, exact/bitmap membership,
index/run builds, and each resumable attempt. Round ordinals are chronological
within each capture; they need no runtime formatting/allocation.

The fixed-point range begins **after EDB canonicalization and empty relation
creation**, ends after final synchronization, and includes frozen-key builds and
all SCCs. `program` additionally includes resident input setup. Uploads and
CPU correctness-oracle work are outside these ranges. To select the two bench
cases (the executable first runs seven smaller correctness cases):

```sh
python3 profile/analyze.py /path/oracle.sqlite --window-index 7 \
  --json /path/chain.analysis.json --markdown /path/chain.summary.md
python3 profile/analyze.py /path/oracle.sqlite --window-index 8 \
  --json /path/branching.analysis.json --markdown /path/branching.summary.md
python3 profile/ncu.py --nvtx-range fixed-point --range-instance 8 \
  /path/chain.analysis.json /path/ncu-chain -- /path/oracle-nvtx --bench
python3 profile/ncu.py --nvtx-range fixed-point --range-instance 9 \
  /path/branching.analysis.json /path/ncu-branching -- /path/oracle-nvtx --bench
```

The NCU runner chooses the median-duration invocation of each top-N kernel from
the nsys report, using its ordinal among same-name launches. Capture scopes and
binary/data must therefore match. `--launch-count` increases samples starting
there; default is one. `--resume` reuses captured reports, not rerunning successful
kernels. Raw CSV and `.ncu-rep` files are retained; `metrics.json` supports both
wide and long CSV layouts. Warp execution efficiency is active threads per
executed warp instruction divided by 32, from
`smsp__thread_inst_executed_per_inst_executed.ratio`. This is not branch efficiency.
NCU replay/clock/cache controls perturb execution: saturation samples must not be
used as uninstrumented timing or concurrency measurements.

### MiniLinq ddisasm cvc5

Build the consumer with the same NVTX feature, then profile the recorded cvc5
facts and constants (commands run from `experiments/gpu`):

```sh
cargo +nightly-2026-08-28 oxide build --arch sm_89 -- --release \
  -p gpu-datalog-ddisasm --bin gpu-datalog-ddisasm --features cq-gpu-runtime/nvtx
data=/home/stargazermiao/workspace/SRDatalog/benchmarks/data/ddisasm/cvc5
meta=/home/stargazermiao/workspace/SRDatalog/integration_tests/examples/ddisasm/cvc5_consts.json
out=/data/artifacts/mini-linq-gpu-profiles/2026-10-19-ddisasm
profile/profile.sh "$out/cvc5" -- \
  target/release/gpu-datalog-ddisasm "$data" --meta "$meta"
python3 profile/ncu.py --nvtx-range fixed-point --top 5 \
  "$out/cvc5.analysis.json" "$out/ncu-cvc5" -- \
  target/release/gpu-datalog-ddisasm "$data" --meta "$meta"
```

`profile.sh` runs `analyze.py` automatically. Omit `--check` during profiling:
the separate exactness run checks all eleven output relations against CPU tuples;
the captures keep output resident and do not repeat the CPU oracle. The consumer's
`fixedpoint_ms` timer includes EDB canonicalization; the narrower `fixed-point`
NVTX range excludes it. Both exclude input upload and output checking.

### SRDatalog (without modifying its source)

First produce `datalog-ddisasm/srdatalog-cvc5.json` with that directory's
`measure_srdatalog.py`. The wrapper below loads the **same recorded compiled
artifact and data**, runs init/load/prepare outside capture, and encloses only
run+synchronize with `cudaProfilerStart/Stop`. It does not overwrite baseline
measurement JSON or modify external trees.

```sh
NSYS_CAPTURE=cudaProfilerApi profile/profile.sh /path/srdatalog-cvc5 -- \
  uv run --project /home/stargazermiao/workspace/srdatalog-python \
  python profile/srdatalog.py datalog-ddisasm/srdatalog-cvc5.json
python3 profile/ncu.py --profile-from-start off \
  /path/srdatalog-cvc5.analysis.json /path/ncu-srdatalog -- \
  uv run --project /home/stargazermiao/workspace/srdatalog-python \
  python profile/srdatalog.py datalog-ddisasm/srdatalog-cvc5.json
```

`profile.sh`, `ncu.py`, and `overhead.py` wait for an empty
`nvidia-smi --query-compute-apps=pid --format=csv,noheader` before each GPU run.
This is a shared-device guard, not a cross-user lock; coordinate independent
launchers to prevent simultaneous starts.

### Analyzer definitions

* GPU busy: union of kernel + memcpy + memset intervals divided by selected
  wall-time windows; kernel busy excludes transfers. No double counting overlap.
* Without matching NVTX, scope is first-to-last captured GPU activity. Generic
  totals, concurrency, top kernels, and launch gaps do not require NVTX. Round
  metrics are explicitly unavailable without round ranges, not estimated.
* Concurrency: maximum and time-weighted mean distinct `(device, context, stream)`
  with kernels in flight; overlap is time with at least two kernels in flight,
  divided by the full selected window. JSON also gives mean active streams
  conditional on kernel-busy time.
* Per-round kernel count follows CUDA launch correlation and calling thread,
  including asynchronously completed kernels. Idle gaps are clipped to **host**
  round boundaries and include boundary gaps. Synchronization/readback calls
  include stream/event/device/context synchronizations, synchronous memcpy, and
  DtoH asynchronous copies (direction resolved through CUPTI correlation for
  generic `cudaMemcpyAsync`). CUDA API version suffixes are accepted. Counts are
  API calls, **not proof that each call blocked**; nested runtime/driver wrappers
  may both be counted. API durations use interval union.
* Launch-gap histogram is previous launch API end → next launch API start, not
  kernel start-to-start. GPU-idle metrics separately union device activity.
* GPU idle overlapping CUDA APIs and idle outside all CUDA APIs partition idle
  time; neither alone proves causality. Host API time overlaps GPU execution and
  cannot be added to GPU busy time. Phase NVTX durations similarly nest.
* `python3 -m unittest discover -s profile -p test_analyze.py` covers interval
  unions/clipping, concurrency, disjoint windows, versioned APIs, and DtoH filtering.

## Measured baseline: 2026-10-18, with cvc5 update 2026-10-19

RTX 6000 Ada, sm_89. Evidence root:
`/data/artifacts/mini-linq-gpu-profiles/2026-10-18-baseline/`.
`oracle.nsys-rep/.sqlite` contains the complete `gpu-datalog-oracle --bench` run;
`chain.analysis.json` and `branching.analysis.json` select windows 7 and 8.
`srdatalog-cvc5.nsys-rep/.sqlite` contains the isolated SRDatalog run. The JSONs
contain every round and the full kernel ranking; adjacent `*.summary.md` files
contain complete per-round tables. These are profiler-perturbed measurements:
oracle workloads differ from cvc5, and the cvc5 cross-engine timing boundaries
are distinguished below. The successful MiniLinq cvc5 update uses evidence root
`/data/artifacts/mini-linq-gpu-profiles/2026-10-19-ddisasm/`, containing
`cvc5.nsys-rep/.sqlite`, `cvc5.analysis.json`, `cvc5.summary.md`, and
`ncu-cvc5/metrics.json` plus all five raw NCU reports.

| Capture | Window ms | GPU busy % | Kernels | ≥2-kernel overlap % | Max streams | Avg streams/window | GPU idle ms | Idle p50/p95/max µs |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Oracle chain-192 | 382.413 | 24.680 | 58,289 | 0 | 1 | 0.233 | 288.033 | 3.392 / 10.784 / 3588.560 |
| Oracle branching/cycles/duplicates-192 | 147.031 | 35.552 | 18,657 | 0 | 1 | 0.345 | 94.759 | 3.456 / 11.392 / 351.106 |
| SRDatalog ddisasm cvc5 | 803.948 | 46.234 | 24,901 | 0.1532 | 7 | 0.421 | 432.250 | 6.016 / 15.328 / 19952.221 |
| MiniLinq ddisasm cvc5 | 12896.983 | 80.136 | 629,196 | 0 | 1 | 0.797 | 2561.799 | 2.080 / 8.704 / 14627.134 |

| Capture | Round ranges | Sync/readback calls in rounds | Mean calls/round | Total capture sync/readback calls | CUDA API union ms | Sync/readback API union ms |
|---|---:|---:|---:|---:|---:|---:|
| Chain | 603 | 12,854 | 21.317 | 12,867 | 246.790 | 43.089 |
| Branching | 176 | 4,174 | 23.716 | 4,187 | 101.824 | 36.199 |
| SRDatalog cvc5 | unavailable (no round NVTX) | unavailable | unavailable | 36,437 | 659.546 | 444.240 |
| MiniLinq cvc5 | 878 | 75,426 | 85.907 | 75,443 | 11690.570 | 9383.917 |

Chain has 1,434 stream-sync calls, 1,433 DtoH calls, and **10,000 event-sync
calls**; branching has 453, 452, and **3,282**, respectively. Every event sync is
inside `admission/count-check`; their summed durations are only 3.231 and
1.070 ms. `sets.rs` calls `runtime.collect_timings()` after the checked readback;
`collect_timings()` calls `elapsed_ms` on retained timing events. Thus the
runtime's diagnostic `host_wait_count` is not a complete CUDA synchronization API
count, and these mostly short event calls must not be mistaken for 10,000
additional long blocking waits.

SRDatalog has 21,475 stream-sync, 5,757 device-sync, 4,586 async DtoH, and 4,619
synchronous memcpy calls in its GPU window. Its streams do overlap, but measured
overlap occupies only 0.1532% of that window; seven possible active streams is not
sustained utilization.

### Where time goes

These columns are an exclusive wall-time partition, except the parenthesized
sync component which is a subset of idle-in-CUDA:

| Capture | GPU active ms | GPU idle inside CUDA APIs ms (sync subset) | GPU idle outside CUDA APIs ms |
|---|---:|---:|---:|
| Chain | 94.380 | 169.705 (14.508) | 118.328 |
| Branching | 52.272 | 55.973 (4.672) | 38.786 |
| SRDatalog cvc5 | 371.699 | 295.747 (106.261) | 136.503 |
| MiniLinq cvc5 | 10335.184 | 1704.881 (184.900) | 856.918 |

Our GPU spends **75.32% / 64.45%** of the chain/branching fixed-point windows
idle. Most idle-in-CUDA time is not synchronization: it is other driver/API
work between short kernels. The remaining 118.328 / 38.786 ms is GPU idle while
the host is outside CUDA APIs (including normal host scheduling, allocation,
dictionary/handle work, profiler overhead, and OS scheduling; the trace does not
separate those causes). This is direct evidence that kernel execution alone is
not the majority cost; optimizing only join arithmetic cannot remove these
launch/host gaps. NVTX phase durations nest and must not be added together.
For example, branching spends 25.346 ms in index/run builds, 14.685 ms in dedup,
16.752 ms in difference, 13.845 ms in count checks, and 9.067 ms in full-copy host
ranges; those figures include enqueue/host work, not just corresponding kernels.

SRDatalog is also idle for 53.77% of its captured GPU window, with substantial
sync-associated idle time, despite using multiple streams. MiniLinq cvc5 is idle
for only 19.86%, but takes much longer: busy percentage measures activity, not
useful throughput or proximity to peak compute/memory bandwidth.

### Our ddisasm cvc5 profile: measured after GPU unblock

The consumer now executes all 23 rules. The separate unprofiled
[`gpu-cvc5.json`](../legacy-datalog/datalog-ddisasm/gpu-cvc5.json) records full tuple equality for
all eleven outputs, 11,444.806 ms fixed-point time, 13.844 ms upload, and
6,504,636,136 bytes (6.058 GiB) allocator peak. Its fixed-point timer is
**18.79×** the prepared, synchronized SRDatalog run's 609.062 ms from
[`srdatalog-cvc5.json`](../legacy-datalog/datalog-ddisasm/srdatalog-cvc5.json). Both exclude
CSV parsing, H2D upload, and output checking; MiniLinq includes EDB
canonicalization, first-use query-module loading, index construction, admission,
and resumable compaction, while SRDatalog's recorded run follows prepare.
These are single observed runs, not repeated timing distributions.

The NVTX fixed-point capture is 12,896.983 ms versus SRDatalog's 803.948 ms
first-to-last captured GPU interval (16.04×); do not substitute this
profiler-perturbed ratio for the unprofiled comparison. MiniLinq launches
25.27× as many kernels. It has one active stream and no kernel overlap.
Its 75,443 synchronization/readback API calls comprise 7,089 async DtoH,
7,090 stream synchronizations, and 61,264 event synchronizations; these counts
are not equivalent to the runtime's 7,118 diagnostic host waits.

The exclusive wall partition above leaves 1,704.881 ms GPU-idle inside CUDA APIs
and 856.918 ms outside them; only 184.900 ms of idle overlaps synchronization.
Nested host NVTX ranges include index/run builds 1,114.705 ms, resumable
compaction 539.077 ms, and admission/count-check 4,796.008 ms. They are not
additive GPU costs: count checks can wait for preceding asynchronous work.
The top five kernels account for 85.38% of kernel-active time, led by
`set_insert` at 3,946.044 ms (30.60% of the entire fixed-point interval).

#### Stratum comparison

MiniLinq SCC membership comes from nested `hole/HEAD/variant-N` ranges in
`cvc5.sqlite`; times include seed and step rounds. SRDatalog times below are
the integer-millisecond step timers in the baseline `srdatalog-cvc5.log`, with
seed and recursive steps grouped by matching output relations. They are coarse
host timers, **not matching NVTX windows or per-round measurements**.

| Relation group | MiniLinq SCC | MiniLinq NVTX ms | MiniLinq round ranges | SRDatalog steps | SRDatalog profiled ms | SRDatalog unprofiled ms |
|---|---|---:|---:|---|---:|---:|
| `JumpTableTarget` | 0 | 11.893 | 2 | 0 | 18 | 11 |
| Register def/use and live-variable recursion | 1 | 3894.060 | 378 | 2 + 5 | 348 | 229 |
| `StackDefUseLiveVarAtBlockEnd` | 2 | 6103.814 | 438 | 12 + 14 | 261 | 200 |
| `StackDefUseLiveVarAtPriorUsed` | 3 | 114.639 | 2 | 17 | 19 | 19 |
| `StackDefUseDefUsed` | 4 | 2575.037 | 43 | 19 + 21 | 136 | 131 |
| `DefUsedForAddress` | 5 | 182.189 | 13 | 23 + 25 | 14 | 11 |
| `RegRegArithOpDefs` | 6 | 15.343 | 2 | 27 | 0 (rounded) | 0 (rounded) |

Unprofiled SRDatalog step evidence is
[`srdatalog-cvc5.log`](../legacy-datalog/datalog-ddisasm/srdatalog-cvc5.log).
The register and stack-live-at-block-end SCCs occupy 77.52% of MiniLinq's
profiled window. SRDatalog also concentrates time in those groups, but uses
a different schedule (including `_temp_StackLiveVarBlockEnd1`), so round counts
cannot be compared directly; its capture has no per-round NVTX.

### Top-five kernels: Nsight Systems time and Nsight Compute saturation

NCU columns are **one median-duration invocation**, not weighted averages over
all invocations. Long-tailed SRDatalog launches make that distinction important.
Grid/block entries below are X dimensions (Y=Z=1); `mean → sample` distinguishes
the Nsight Systems average configuration from the selected NCU launch.
SM/DRAM are percentages of peak sustained throughput, occupancy is achieved
active-warps percentage, and WEE is active threads/32. All exact mangled/template
names and raw metric values are in `ncu-{chain,branching,srdatalog}/metrics.json`.

#### Chain-192

| Kernel | Count | nsys total ms | nsys avg µs | Grid / block (mean → sample) | SM % | DRAM % | Occupancy % | WEE % |
|---|---:|---:|---:|---|---:|---:|---:|---:|
| `initialize_view` | 13,513 | 12.028 | 0.890 | 1 / 256 | 0.01 | 0.14 | 7.87 | 67.03 |
| `cq_gpu_stream_23_r3` | 1 | 9.199 | 9198.826 | 18 / 256 | 1.17 | 0.00 | 16.46 | 81.03 |
| `cq_gpu_stream_22_r4` | 1 | 8.538 | 8538.375 | 18 / 256 | 1.18 | 0.00 | 16.56 | 64.44 |
| `fold_status` | 8,561 | 8.216 | 0.960 | 1 / 256 | 0.01 | 0.12 | 6.87 | 77.81 |
| `initialize_pointer` | 6,300 | 5.325 | 0.845 | 1 / 256 | 0.01 | 0.12 | 8.84 | 85.78 |

CUDA launch correlation and enclosing NVTX identify `cq_gpu_stream_23_r3` as
`hole/Copied/variant-17`, and `cq_gpu_stream_22_r4` as
`hole/Composite/variant-16`. These two long kernels also have low measured
SM/DRAM utilization; the one-block metadata kernels are launch-heavy rather
than GPU-saturating work. This is evidence for targeting both metadata launch
count and these specific rule kernels, not a claim that all joins are bandwidth
bound.

#### Branching/cycles/duplicates-192

| Kernel | Count | nsys total ms | nsys avg µs | Grid / block (mean → sample) | SM % | DRAM % | Occupancy % | WEE % |
|---|---:|---:|---:|---|---:|---:|---:|---:|
| `cq_gpu_stream_23_r3` | 1 | 12.840 | 12840.347 | 21 / 256 | 1.19 | 0.00 | 16.41 | 83.69 |
| `cq_gpu_stream_22_r4` | 1 | 11.894 | 11893.718 | 21 / 256 | 1.19 | 0.00 | 16.58 | 68.47 |
| `initialize_view` | 4,167 | 3.482 | 0.836 | 1 / 256 | 0.01 | 0.13 | 7.85 | 67.03 |
| `fold_status` | 2,804 | 2.530 | 0.902 | 1 / 256 | 0.01 | 0.12 | 6.72 | 77.81 |
| `set_insert` | 582 | 1.982 | 3.406 | 22.15 → 1 / 256 | 0.04 | 0.19 | 6.49 | 84.22 |

#### SRDatalog ddisasm cvc5

CUB names are shortened to their operation plus four-column specialization;
exact template names remain in the reports.

| Kernel | Count | nsys total ms | nsys avg µs | Grid / block (mean → sample) | SM % | DRAM % | Occupancy % | WEE % |
|---|---:|---:|---:|---|---:|---:|---:|---:|
| `JitRunner_StackDefUsed4_D1::kernel_materialize` | 41 | 39.018 | 951.670 | 1136 / 256 | 3.36 | 0.41 | 28.86 | 98.25 |
| `JitRunner_StackDefUsed4_D1::kernel_count` | 42 | 37.066 | 882.530 | 1136 / 256 | 3.34 | 0.45 | 29.16 | 97.81 |
| CUB `DeviceMergeSortMergeKernel` (4 columns) | 844 | 36.113 | 42.788 | 1406.73 → 40 / 256 | 2.65 | 6.68 | 16.27 | 97.84 |
| `small_dedup_kernel<unsigned int,4>` | 1,718 | 27.348 | 15.919 | 1 / 1 | 0.02 | 0.03 | 2.08 | 3.13 |
| CUB `DeviceMergeSortBlockSortKernel` (4 columns) | 1,906 | 18.583 | 9.750 | 48.85 → 1 / 256 | 0.09 | 0.12 | 16.64 | 93.66 |

The sampled materialize/count launches take 34.080/38.656 µs under NCU, versus
951.670/882.530 µs **mean** in nsys: the launch distributions are strongly skewed,
and these saturation samples must not be generalized to their longest launches.

#### MiniLinq ddisasm cvc5

Evidence: `2026-10-19-ddisasm/ncu-cvc5/metrics.json` under the artifact parent
above. All five samples use the same binary, inputs, and `fixed-point` filter.

| Kernel | Count | nsys total ms | nsys avg µs | Grid / block (mean → sample) | SM % | DRAM % | Occupancy % | WEE % |
|---|---:|---:|---:|---|---:|---:|---:|---:|
| `set_insert` | 10,014 | 3946.044 | 394.053 | 177.75 → 2 / 256 | 0.08 | 0.17 | 10.14 | 60.94 |
| `cq_gpu_stream_28_r11` | 265 | 1948.617 | 7353.273 | 568 / 256 | 3.19 | 0.88 | 44.98 | 76.38 |
| `order_bins` | 7,383 | 1261.573 | 170.875 | 292.67 → 256 / 256 | 12.21 | 6.24 | 23.89 | 43.69 |
| `pack` | 10,014 | 881.499 | 88.027 | 177.75 → 245 / 256 | 8.37 | 18.06 | 26.85 | 99.97 |
| `append` | 9,879 | 741.449 | 75.053 | 122.06 → 118 / 256 | 3.49 | 0.24 | 16.47 | 100.00 |

`cq_gpu_stream_28_r11` is the recursive stack-definition-use join observed
inside `hole/StackDefUseDefUsed/variant-22`. The sampled NCU durations are
16.960, 19,112.256, 7.648, 6.176, and 3.296 µs respectively, not the nsys means.
The sampler reuses each nsys median launch's same-name ordinal; resumable
compaction counts vary between runs (773 in the unprofiled check, 771 under
nsys), so this is not a guarantee of identical logical launch identity.
These sparse saturation samples do not show peak SM or DRAM utilization and
cannot establish average utilization across all 629,196 kernels. In particular,
80.14% device-busy time must not be interpreted as 80.14% SM throughput.

### Annotation overhead, without a profiler

Both feature variants were built from the same source and saved as
`oracle-disabled` / `oracle-nvtx` under the evidence root. `overhead.py` ran one
warmup pair (discarded), then seven pairs, alternating execution order and
checking GPU idleness before every run. All runs passed the executable's exact
tuple assertions. Evidence: `overhead/runs.json` and each numbered stdout log.
These `execution_ms` timers include resident setup and therefore differ from
the narrower fixed-point NVTX window.

```sh
python3 profile/overhead.py /path/oracle-disabled /path/oracle-nvtx \
  /path/overhead --pairs 7
```

| Case | Disabled median ms [min,max] | Enabled median ms [min,max] | Paired mean enabled−disabled ms | Paired 95% t interval ms |
|---|---|---|---:|---|
| Chain-192 | 198.858 [195.398,205.413] | 197.132 [195.075,199.508] | -2.376 | [-5.284,0.532] |
| Branching | 87.339 [86.615,87.648] | 87.052 [86.670,87.475] | -0.139 | [-0.668,0.390] |

The intervals use the seven paired differences, sample standard deviation, and
Student-t(6) critical value 2.4469. **No measurable overhead from enabled
annotations without a profiler was detected in these runs** (both intervals
include zero); negative differences are noise, not a speedup claim. Disabled
annotations compile to empty inline guards and do not execute any NVTX path.

The only pre-instrumentation phase-2a reference is the historical single run in
`../legacy-datalog/datalog-oracle/results.json`: chain 199.903 ms, branching 86.100 ms. Current
disabled medians differ by -0.52% and +1.44%, respectively. That reference was
measured on a shared GPU and no old binary was preserved: it is **approximate,
not a controlled proof of zero pre/post regression**. The controlled evidence
here is the disabled-vs-enabled comparison; no stronger historical claim is
made.

### Checks exercised

* Pinned `cargo oxide build --arch sm_89 -- --release` succeeded with and without
  `cq-gpu-runtime/nvtx` for `gpu-datalog-oracle` (existing dependency warnings).
* The complete oracle benchmark passed exact tuple assertions in the nsys run,
  all NCU runs, and all 16 unprofiled overhead runs.
* Three selected baseline analyses and all 15 top-five NCU captures completed.
* Five analyzer unit tests passed; `bash -n profile/profile.sh` passed.
* The successful cvc5 update completed one nsys capture/analysis and all five
  top-kernel NCU captures, after a separate full-tuple exactness check recorded
  in `gpu-cvc5.json`. Profiling omitted CPU checking and ran serially on an idle
  GPU. No builds, tests, or formatting were run by the profiling worker.
* SRDatalog source trees were not modified.

## Exact admission and fused-emission investigation (2026-10-19)

Dedup scheduling is now selected only by ordinary pipeline advice:
`retype(dedup => DeviceSet<_, Plain, Policy>)`, where the consuming pass records
the third `DeviceSet<Row, Codec, Policy>` parameter in checked Structures.
Compacting is the default; the ddisasm consumer's `hole(rule 19, delta 1)`
override selects Fused and printer `output(retry)`. A default pipeline can
select Fused globally for reproducing an all-Fused experiment. The former
macro-prefix and Cargo-feature policy channels are removed; archived measurements
below retain their original policy scope and are not measurements of this cutover.

Fused compares every emitted head word, publishes its output row with
acquire/release synchronization, and writes only the first occurrence.
Busy slots return to the reconvergent caller rather than spinning inside an
insertion. Output overflow leaves the pending candidate in its checkpoint;
the runtime grows output storage and rebuilds the exact row-ID table before
resuming. No hash fingerprint is treated as equality. Ordered/lossy head
canonicalization remains a separate boundary after projection.

Evidence is under
`/data/artifacts/mini-linq-gpu-profiles/2026-10-19-fused-admission/`;
`variant22-{Compacting,Fused}.structures.txt` and `.generated.rs` are actual
checked dumps, including the default `cq_staging` pass. The earlier
cyclic-plan dumps stopped before staging: the actual final decision is
`r0 stage = batch 128`, not an unstaged serial stream. The diagnostic dump
uses module number zero; the baseline consumer's corresponding kernel is
`cq_gpu_stream_28_r11`. The historical emitted LLVM and executable are
preserved beside the baseline trace as `gpu-datalog-ddisasm-baseline.ll`
and `gpu-datalog-ddisasm-baseline`.

### Variant-22 baseline diagnosis

The outer relation is complete `StackDefUseLiveVarAtPriorUsed`; the first
expansion probes delta `StackDefUseDefUsed` on `(eaUsed,varp,varr)`, and
the second probes complete `StackDefUseLiveVarUsedEDB` on
`(nxtBlk,varp,varr)`. Both are recorded HashedRuns, with full-key residual
comparisons after bucket lookup. The 256-thread kernel processes 128-row
staged units, retaining two expansion descriptors and 29 continuation
registers per lane. Dynamic shared memory is 24,672 bytes. No `expand`
decision is recorded, so expansion frontier publication is disabled.

The baseline SQLite trace contains 42 invocations of
`hole/StackDefUseDefUsed/variant-22`, 265 resumable attempts and 223 prefix
compactions. Kernel durations in milliseconds are min 1.073, median 7.849,
p95 10.270, p99 10.710, max 18.767, sum 1,948.617. Raw records and SQL-derived
statistics are in `../2026-10-19-ddisasm/variant22-baseline.json`.

Full NCU capture of same-name launch ordinal 187 (zero-based), selected by
the longest baseline nsys duration, is
`../2026-10-19-ddisasm/variant22-baseline-long.ncu-rep`; its raw CSV and
selected metrics are retained alongside it. It reports:

* 568 blocks × 256 threads, 48 registers/thread, 44.04% active-cycle occupancy.
* SM active/elapsed cycles only 4.58%, SM throughput 1.34%, DRAM throughput
  0.34%, and 30.76 active threads per executed warp instruction.
* 11.48 barrier and 6.95 long-scoreboard stalled warps per issued warp;
  atomic-pipeline activity rounds to 0.00%.

This supports a block-tail/load-distribution and synchronization/latency
bottleneck, not saturated bandwidth or primarily within-warp divergence.
NCU replay measured 39.204 ms for that launch; it is not a replacement for
the 18.767 ms nsys duration.

Exact CPU fixed-point exports plus the immutable outer/EDB inputs give
769,206,847 raw head witnesses over the recursive delta partition. The
separately recorded CPU per-round canonical outputs sum to 3,093,081
(248.69 raw witnesses per per-round distinct head), with 1,377,031 newly
admitted tuples. This is relational multiplicity evidence, not a GPU atomic
counter or a count of rejected hash-bucket matches. The SQL and provenance
are in `../2026-10-19-ddisasm/variant22-multiplicity.json`.

### Fused contention diagnosis and exact ticket reservation

`cq-gpu-runtime/dedup-diagnostics` is an opt-in diagnostic build. It adds
striped device counters for inserts, duplicate hits, busy returns, overflow
returns, total probes and maximum probe count, plus a host report of initial
capacity, final capacity, slots, distinct rows and growth attempts. All of
these counters/readbacks are compiled out normally. Never use diagnostic
execution durations as performance measurements.

The initial Fused implementation was exact but regressed: 24.136 s resident
computation versus 8.195 s for the integrated Compacting configuration.
Hoisted Fused was also exact and took 25.015 s. Those initial Fused binaries
are preserved for diagnosis, not recommended as the final implementation.
The old Fused nsys trace attributes 13.019 s to `cq_gpu_stream_21_r9`,
2.784 s to `cq_gpu_stream_24_r11`, and 1.779 s to variant-22.

Diagnostic variant-22 counters exactly reproduce the independent relational
witness count: 769,206,847 completed candidates, 3,093,081 distinct rows,
766,113,766 duplicate hits. There were **3,796,786,034 busy retries**, despite
maximum probe chain 8, zero growth/rehash and at most 1,404,862 occupied slots
out of 16,777,216 (8.37%). Even a nearly distinct 79,400-row projection had
95,164,903 busy returns. Table saturation is not the cause.

The lock owner was holding its hash slot while retrying a globally contended
CAS output-counter reservation. NCU of the old kernel21's longest nsys
invocation (ordinal 46) records **2,793,221,446 L2 CAS requests** in that one
launch (`fused-regression-k21.ncu-rep`). A warp reconvergence barrier alone
did not eliminate the bottleneck.

The final insertion takes a single atomic output ticket. Tickets outside the
capacity publish neither a key nor a row and leave their candidate pending.
Once the launch suspends, a stream-ordered finalizer clamps the count to the
completed dense in-capacity prefix before growth and exact rehash. Thus no
ticket is lost, no failed key is retained, and no candidate is replayed after
successful publication. The active-mask warp barrier prevents retrying lanes
from racing ahead of their publishing peers.

With tickets, diagnostic variant-22 busy returns fell to 79,097 with identical
candidate/distinct counts; the nearly distinct projection fell to 2,675.
The same regression oracle checks repeated growth, exact collisions, heavy
duplicates, frontier continuations, wide keys, and lossy/repeated head canon.

A separate scheduling ablation found no persuasive variant-22 improvement
from dynamic root claiming: static/dynamic kernel sums were 1,933.638 /
1,924.545 ms, with 267 / 263 attempts. Whole nsys windows were 7,180.803 /
7,031.861 ms. The small difference is confounded by suspension ordering;
the dynamic-root change was removed. The `.generated.rs` diagnostic dumps
preserve that experimental scheduler, while the final implementation retains
the original static root schedule.

### Final measured results and remaining limits

All timings are preliminary, single-run observations on the shared workstation;
CPU baseline work ran concurrently. Exactness checks compare every tuple of all
11 outputs, not fingerprints. The resident timer includes EDB canonicalization;
the narrower nsys fixed-point range does not.

| Configuration | Resident ms | nsys fixed-point ms | Exact CPU tuples |
|---|---:|---:|---|
| Historical Compacting baseline | 11,444.806 | 12,896.983 | all 11 |
| Persistent admission, Compacting, static roots | 5,919.146 | 7,180.803 | all 11 |
| Final Fused, static roots | 5,961.520 | 4,355.017 | all 11 |
| Final Fused + checked invariant hoisting | 5,897.183 | 4,331.103 | all 11 |
| Staged | unavailable | unavailable | blocked by device allocation |

The final Fused nsys totals versus the historical trace are:

| Kernel | Before ms / calls | Final Fused ms / calls |
|---|---:|---:|
| `set_insert` | 3,946.044 / 10,014 | 17.125 / 2,385 |
| `pack` | 881.499 / 10,014 | 4.842 / 2,385 |
| `append` | 741.449 / 9,879 | 0 / 0 |
| `order_bins` | 1,261.573 / 7,383 | 1,263.042 / 7,383 |
| variant-22 | 1,948.617 / 265 | 353.134 / 42 |

Variant-22 now has zero compactions; its launch-duration min/median/p95/max
is 1.138 / 1.152 / 25.593 / 126.714 ms. Fewer, longer complete launches
replace suspension-sized attempts, so a single-launch before/after duration
is not an equal-work comparison. Final GPU busy time is 54.04% of the window.
The old Fused regression kernel21 fell from 13,019.167 ms to 27.960 ms.

Final long-launch NCU (`final-variant22-long.ncu-rep`, ordinal 1) reports
SM throughput 17.54%, DRAM throughput 0.58%, active-cycle occupancy 41.37%,
28.72 active threads/warp instruction, and SM active/elapsed cycles 78.52%.
Barrier and long-scoreboard stalled warps per issued warp are 13.22 and 7.16.
The replay duration is 314.337 ms and must not substitute for nsys timing.
Its 58 registers/thread allocate 64 registers/thread. The final emitted LLVM
is preserved as `ddisasm-fused-final.ll`.

Persistent admission ranges in the final Fused trace are 2,390 `insert-new`,
2,390 `append-tail`, 28 `full-grow`, 13 `rehash`, and 878 `count-check`
calls. Their inclusive host-range totals are respectively 139.579, 6.765,
0.739, 1.019 and 180.526 ms, not GPU-only execution times.
`persistent_admit`, `persistent_rehash` and `persistent_copy` consume
35.944, 18.188 and 6.442 ms of GPU time. The remaining 1.263 s of
`order_bins` is unchanged; persistent admission is not maintained join indexes.

Verification completed: printer/Datalog tests **57 passed**; printer oracle
(including Fused growth/collision/frontier/head regressions), persistent
Datalog-runtime oracle, string-dictionary oracle, and Compacting/Fused
`gpu-datalog-oracle --bench` passed. Final static-root Compacting, Fused, and
hoisted Fused each passed cvc5 full-tuple comparisons. Persistent-admission code
needed no fixes. The late Compacting resident run was faster than its earlier
8,195 ms dynamic-root run; together with the scheduling ablation this reinforces
that single-run wall times cannot establish an isolated scheduling improvement.

Staged cvc5 builds but fails while growing a six-column candidate output to
274,508,544 rows with `CQ_GPU_VRAM_EXCEEDED` / CUDA out-of-memory
(`staged-exact.log`). Its full-input exactness and timing criterion therefore
remains unmet; no Compacting fallback was substituted. Final Fused allocator
peak is 12,687,827,976 bytes, versus 6,504,636,136 bytes historically: speed
improved at a higher persistent-storage memory cost.

The 0.609 s prepared SRDatalog target is **not reached**. Re-time isolated
after integration before claiming a stable end-to-end factor. All raw traces,
exact-check logs, feature configuration, diagnostic records and kernel summaries
remain in the evidence directory above.

## LE variant-25 regression isolation (2026-10-19)

Evidence: `/tmp/variant25-baseline-evidence.json`,
`/tmp/variant25-cpu-workload.json`, `/tmp/variant25-old-evidence/`, and
`/tmp/variant25-comparison.json`. Each comparison has an exact result
`/tmp/variant25-{a,b,c}-exact.json` and nsys capture/analysis under
`/tmp/variant25-{a,b,c}-profile.*`. All three checked all eleven cvc5 outputs.

The old default planner and LE both drive LVABE and build filtered UsedInBlock.
The old scan/select spine received a batch-128 stage and cooperative bucket
flattening. LE inserts an eager semi before the probe, losing that automatic
stage; HashedRun cannot certify an exact expansion range. The baseline therefore
performs two bucket reads and lane-local output reservation.

NCU measured 95.005% long-scoreboard stalls, 0.6135% SM throughput and 48.04%
active lanes/warp. SourceCounters attributed 80.72% of samples to the resumable
reservation CAS and its dependent branch: 84,378,877,080 lane CAS executions for
4,289,196 emitted rows in that instrumented pass. This is reservation contention,
not huge hash buckets: 15,152 filtered rows occupy 12,099 of 32,768 buckets,
maximum bucket length five. Of 14,986 distinct keys, 14,820 have degree one and
166 degree two. The probe examines 6,272,739 bucket candidates and emits
4,289,196 rows from 21,236,205 driver rows.

| Configuration | Exact-run fixedpoint ms | Nsys fixed-point window ms | Variant-25 output kernel attempts ms | Allocator peak bytes | Traced CUDA peak bytes |
|---|---:|---:|---|---:|---:|
| a: PrefixRun, resumable Compacting | 6283.576 | 3712.732 | 5.513 | 14390000048 | 17178820608 |
| b: PrefixRun, direct `output(retry)` | 7685.973 | 3600.475 | 2.704 + 2.771 | 6699175392 | 8588886016 |
| c: HashedRun, direct `output(retry)` | 6734.895 | 3421.746 | 5.160 + 5.199 | 6699175392 | 8588886016 |

These are single runs, not interchangeable timing boundaries or a stable
end-to-end speedup claim. Baseline variant-25 took 15,887.454 ms in nsys.
The PrefixRun direct successful attempt is about twice as fast as HashedRun,
but generic PrefixRun construction increases sorting work elsewhere. Direct
retry removes the resumable state machine; its discarded first attempt largely
cancels the per-attempt gain over staged resumable execution for this hole.
The consumer intentionally selects configuration b; this is not the fastest
whole-program measurement in the table.

Both b and c make 4,751 bounded attempts for 4,632 query-kernel calls (119
relaunches). Nsys discarded-kernel time is 23.426 ms and 30.928 ms respectively.
The unchanged Fused override separately makes 48 attempts for 42 calls.
The complete per-SCC, top-kernel and Fused-retry tables are in the comparison JSON.

`expand_lookups(N, depth D)` records at most D nested expansions (default two).
The printer honors omitted deeper boundaries with collective-safe lane-local
cursors, rather than implicitly adding another descriptor. Module-34 payload
frames are 12,304, 36,880 and 45,072 bytes; retaining only the first two reduces
its full shared-memory requirement, including frontier claim space, from
139,400 to 86,136 bytes. Variant-25 keeps its one 12,368-byte staged kernel.
