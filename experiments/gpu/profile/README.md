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
[`gpu-cvc5.json`](../datalog-ddisasm/gpu-cvc5.json) records full tuple equality for
all eleven outputs, 11,444.806 ms fixed-point time, 13.844 ms upload, and
6,504,636,136 bytes (6.058 GiB) allocator peak. Its fixed-point timer is
**18.79×** the prepared, synchronized SRDatalog run's 609.062 ms from
[`srdatalog-cvc5.json`](../datalog-ddisasm/srdatalog-cvc5.json). Both exclude
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
[`srdatalog-cvc5.log`](../datalog-ddisasm/srdatalog-cvc5.log).
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
`../datalog-oracle/results.json`: chain 199.903 ms, branching 86.100 ms. Current
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
