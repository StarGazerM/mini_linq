# CQ TPC-H-shaped performance comparison

## Scope and methodology

These are the **27 authored CQ-shaped catalog queries**, not full standard TPC-H coverage: 26 GPU-suite queries plus **h14, CPU-only**, at SF1 and SF10. GPU name `hXX`/`dXX` maps to CPU catalog name `retail_hXX`/`retail_dXX`. Numerical results require exact answers; no timeout is interpreted as an execution time. All times are milliseconds, lower is better.

Hardware: **AMD Ryzen Threadripper PRO 5945WX, 12 cores / 24 hardware threads**, approximately **125.77 GiB RAM**, **NVIDIA RTX 6000 Ada Generation (49,140 MiB)**, driver **580.178.04**. GPU compilation targeted `sm_89` with CUDA 12.9 (NVIDIA HPC SDK 25.5). See [hardware](performance_hardware.json). Retained CPU-default/LE/advised workers are sequential; CPU-par explicitly uses Rayon t1/t24. Run timed CPU windows serially, with no concurrent compilation.

### GPU: statistics-derived advised plans

GPU uses the checked `--mode advised` plans, one warmup and five measured executions. All 52 included GPU results passed exact comparison. SF1 additionally checks sequential-program conformance. The retained results are [the original suite](performance_gpu.json), [fresh-process SF10 recovery for d27/d72_inner/d85](performance_gpu_sf10_remaining.json), and **h19 from [the exact interning suite](gpu_tpch_intern.results.json)**. Initial device allocation failures are not timings. h14's prefix predicate requires order-preserving interning and has no GPU result.

- **Build:** CUDA-event elapsed construction, selection/index preparation and scheduled work. Staged plans also charge host-graded stages and nullable intermediate host publication to build; it is not exclusively device index construction.
- **Query:** median of each sample's kernels + dedup. Dedup includes grouping/reduction, canonicalization and device publication, not merely final DISTINCT.
- **Total:** median total. These independent medians need not sum exactly.
- Input file loading, upload/compression, module initialization, final answer download/decoding and comparison are excluded. Query-required intermediate publication is included. These are resident-input execution measurements, not end-to-end latency.
- h02 stage 1 and h15 stage 2 execute on the host after checked device rejection; these hybrid measurements are not wholly device execution.

H19 uses author-controlled `u32` dictionaries for shipping mode/instruction, brand and container. CPU and GPU use matching sorted-distinct dictionaries; missing literals use `u32::MAX`. Input dictionary preparation is excluded from timing. See [statistics derivation](tpch_statistics.md) and [`benchmark.rs`](../tpch/src/benchmark.rs).

### CPU: LE, advised and default

Each of the 27 queries has **default, le, le_runs and advised** measured independently at each scale. There is **no family pruning**. CPU-default is exactly `default`; CPU-advised is exactly `advised`; CPU-LE is the fastest successfully measured `le`/`le_runs` by its uninstrumented whole-query median. If a candidate times out, the best completed candidate is reported and `cpu_le_search_complete` records the incomplete search. No different planner substitutes for an unavailable LE result.

The original policy was one warm execution, **five plain runs**, one independent counter execution, and **five profile runs**, reducing to **three** timed runs when a warm or timed execution exceeded **60 seconds**, with a **600-second combined wall budget per configuration**. All SF1 and the first part of SF10 used that policy. **At the user's priority override, the remaining SF10 work switched to a 100-second cap, three runs above 10 seconds, and profiles only for each column's selected default / best-LE / advised configuration.** Plain runs still cover every configuration; already completed results are retained, including earlier profiles for nonwinning LE candidates. Per-row policy metadata distinguishes the two collection policies. Completed modes are retained across interruption; incomplete work resumes under the new cap.

Caps include loading, warmup, measurements and exact comparisons across modes. `>100 s timeout` (or the earlier `>600 s timeout`) is a **configuration-budget status, not a numeric query timing or per-execution lower bound**. A completed exact plain measurement remains usable if a later profile phase times out; missing phases stay unavailable.

**Plain total** surrounds `query::run`, including query-local preprocessing and structure/index construction. Parquet loading, load-time zone preparation, canonicalization, sorting, serialization, exact comparison and final answer destruction are excluded. Every warm and timed answer is compared as a normalized exact set against the same authored DuckDB SQL at the same scale. Only DECIMAL trailing-zero formatting is normalized; there is no floating tolerance.

**Profile build** is `BUILD_EXCLUSIVE_TOTAL`: the sum of exclusive elapsed construction time (elapsed minus separately measured source production, saturating) for operator kinds ending `_BUILD`. **Profile query** is the paired profile total minus that build; **profile total** is the median-total profile execution. The build tree rounds to 0.001 ms. All three refer to the same execution, not independent phase medians. Profiling materializes build inputs and adds clocks/bookkeeping, so its timings can differ substantially from plain execution. **Never subtract profile build from plain total.** Zero build means no time in these explicit scopes, not no allocation or hashing.

The separate counter run supplies checked operator rows; profiles check exact answers, graph labels, rows and the top-level clock. Independent primitive insertion-count agreement is recorded as `primitive_build_counts_verified`, separately from exactness. Per-configuration plain/profile samples, run counts, statuses and profiles are retained in the raw SF-specific directories and consolidated JSON/TSV. SF1 and SF10 data/cache directories are isolated; compiled artifacts may be reused only when source fingerprints agree.

Boundaries: [`worker.rs`](../../../features/workloads/xtask/src/tpch/worker.rs), [collector](performance_cpu_collect.py). Source hashes and machine metadata: [SF1](performance_cpu_environment_sf1.json), [SF10](performance_cpu_environment_sf10.json).

### CPU: fixed parallel configuration

CPU-par always selects the authored **`par`** configuration, never the best of multiple plans. Each scale and thread count has isolated measurement artifacts: `RAYON_NUM_THREADS=1` or `24`, raw data under `performance_cpu_raw/par/t{threads}/sf{sf}`, consolidated `performance_cpu_par_t{threads}_sf{sf}.json` and `.tsv`. Scale/thread-independent timing, counting and profile binaries and Cargo output are shared under `/tmp/mini-linq-performance-cpu/par`; exported data remains scale-specific. Cache stamps hash compiler/runtime Rust sources and existing Cargo/toolchain inputs, including Tasks, pull, IR, planner, printer, storage and facade. Existing raw results must match their source stamps before reuse; stale evidence is rejected rather than silently overwritten. Generated consumers depend directly on Rayon because emitted task code uses `::rayon`.

SF0.1 runs one untimed exact comparison for every query at each thread count. A separate `--default-spotcheck` checks h01, h02, h06, h13 and h19 by default, with isolated `performance_cpu_default_spotcheck_t{threads}_sf0.1` artifacts; it never changes the existing CPU measurements. SF1/SF10 run one exact warmup and five plain timed runs, an independent untimed counting run, then one exact profile warmup and five profile timed runs. Any warmup or timed query above **10 seconds** reduces that timed mode to the first **three** samples (including when the fourth/fifth sample triggers reduction). Every result is checked against the same authored SQL with exact normalized-set comparison. The **100-second combined configuration wall cap** includes all worker loading, warmup, measurement and comparisons across plain/counts/profile. Completed exact plain metrics remain valid if counting or profiling subsequently times out; unavailable phases stay null, never estimated. Query-local task scheduling and construction are included in totals; input loading and answer comparison are excluded. Parallel environment, source hashes and command logs are retained separately from the original CPU evidence.

Parallel **setup+build** uses `TASKS_BUILD_WALL_TOTAL`, the measured host-wall **union** of generated setup/publication intervals across all Tasks programs during that same query execution. Nested/concurrent build intervals count once, never once per task. Setup+build includes upstream scans, partition routing and joins needed to publish stores, not just exclusive index insertion. **Final evaluation** is the same median profile total minus that measured union, including boundary/teardown overhead. Per-program `TASKS_QUERY_WALL` final evaluation/collection spans remain raw diagnostics, not an additive wall split. Task-local/lazy construction during final evaluation remains there unless it enters an explicit build interval. Ordinary operator spans can overlap across tasks and exceed root wall time; they are **never summed** for the parallel wall split. **Parallel setup+build is not identical to default/LE/advised exclusive build** (`BUILD_EXCLUSIVE_TOTAL`). Internal JSON fields retain `profile_build` / `profile_query_residual` naming, with these parallel-specific definitions. Phase values never split an uninstrumented total, and no missing phase is inferred from plain timing. **Uninstrumented whole-query totals remain the primary comparison.**



### DuckDB

DuckDB **v1.4.4 (Andium), `6ddac802ff`**, explicit t1/t24. CLI `.timer` measures complete authored SQL in **`.mode trash`**, with one warmup and five samples, avoiding answer CSV formatting/output and comparison. Intrinsic result materialization/consumption remains timed; resolution is 1 ms. Database generation/export, process opening and thread configuration are excluded. Scanning, reads/decompression, hash construction, aggregation and DISTINCT are included. There is no separate external index-build phase. The report uses [SF1](performance_duck_trash_sf1.json) and [SF10](performance_duck_trash_sf10.json), not CSV-output timers.

## Reproduction

From the repository root (run CPU scales serially):

```sh
python3 experiments/gpu/compare/performance_cpu_collect.py all --sf 1 --runs 5 --cap-seconds 600
# Initial SF10 policy; completed results are retained:
python3 experiments/gpu/compare/performance_cpu_collect.py all --sf 10 --runs 5 --cap-seconds 600
# Resume remaining SF10 work using the user-authorized final policy:
python3 experiments/gpu/compare/performance_cpu_collect.py build --sf 10 --cap-seconds 100 --slow-seconds 10 --profile-winners-only
python3 experiments/gpu/compare/performance_cpu_collect.py measure --sf 10 --cap-seconds 100 --slow-seconds 10 --profile-winners-only
python3 experiments/gpu/compare/performance_cpu_collect.py consolidate --sf 10 --cap-seconds 100 --slow-seconds 10 --profile-winners-only
python3 experiments/gpu/compare/performance_cpu_collect.py cleanup --sf 10
# Parallel protocol: complete SF0.1 exactness first, then time SF1/SF10.
# Run serially; do not overlap builds or measurements:
for threads in 24 1; do
  python3 experiments/gpu/compare/performance_cpu_collect.py all --par --threads "$threads" --sf 0.1 --cap-seconds 100     --database features/workloads/target/bench/data/tpch-0.1/dbgen.duckdb
done
python3 experiments/gpu/compare/performance_cpu_collect.py all --default-spotcheck --sf 0.1 --cap-seconds 100   --database features/workloads/target/bench/data/tpch-0.1/dbgen.duckdb
for sf in 1 10; do
  for threads in 24 1; do
    python3 experiments/gpu/compare/performance_cpu_collect.py all --par --threads "$threads" --sf "$sf" --runs 5 --cap-seconds 100
  done
done
python3 experiments/gpu/compare/performance_report.py
```

`--database PATH` explicitly overrides the oracle/export database and records its resolved path in environment metadata; otherwise the existing `/data/tpch-gpu/sf{sf}/dbgen.duckdb` convention is unchanged. The SF0.1 commands assume the xtask-generated database already exists. For an initial parallel smoke subset, add repeatable `--query retail_hXX`; consolidation still preserves every catalog row, marking uncollected queries missing. This does not replace full 27-query exactness. `--default-spotcheck` accepts exactly five distinct `--query` overrides and defaults to one thread. Freeze compiler/runtime and worker sources before collecting; run every command serially, without concurrent builds or measurements.

The collector also exposes `adaptive`, `build`, `prepare`, `measure`, `consolidate`, and `cleanup` phases. Cleanup removes duplicate answers/oracles after exact checks while preserving measurement evidence; prepare regenerates the oracles. GPU reproduction (inside `experiments/gpu`):

```sh
CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9 CARGO_BUILD_JOBS=1 cargo oxide build --arch sm_89 -- --release -p cq-gpu-tpch
./target/release/cq-gpu-tpch compare/performance_gpu.json --mode advised
./target/release/cq-gpu-tpch compare/performance_gpu_sf10_remaining.json --mode advised --scale sf10 --query d27 --query d72_inner --query d85
./target/release/cq-gpu-tpch compare/gpu_tpch_intern.results.json --mode advised
```

DuckDB reproduction: `python3 experiments/gpu/compare/performance_duck.py`.
## Parallel SF0.1 exactness

One untimed execution per query and thread count, compared as an exact normalized set against authored DuckDB SQL. This is correctness evidence, not timing data.

| Configuration | Exact completed | Rows collected (of 27) | Unavailable |
|---|---:|---:|---:|
| par t1 | 27 | 27 | none |
| par t24 | 27 | 27 | none |

### Per-query SF0.1 exactness matrix

Cells show correctness / collection status; missing evidence is explicitly unavailable.

| Query | CPU-par t1 correctness / status | CPU-par t24 correctness / status |
|---|---:|---:|
| h01 | ok / complete | ok / complete |
| h02 | ok / complete | ok / complete |
| h03 | ok / complete | ok / complete |
| h04 | ok / complete | ok / complete |
| h04_not_exists | ok / complete | ok / complete |
| h05 | ok / complete | ok / complete |
| h06 | ok / complete | ok / complete |
| h07 | ok / complete | ok / complete |
| h07_block | ok / complete | ok / complete |
| h08 | ok / complete | ok / complete |
| h09 | ok / complete | ok / complete |
| h10 | ok / complete | ok / complete |
| h11 | ok / complete | ok / complete |
| h12 | ok / complete | ok / complete |
| h13 | ok / complete | ok / complete |
| h14 | ok / complete | ok / complete |
| h15 | ok / complete | ok / complete |
| h16 | ok / complete | ok / complete |
| h18 | ok / complete | ok / complete |
| h19 | ok / complete | ok / complete |
| h20 | ok / complete | ok / complete |
| h21_positive | ok / complete | ok / complete |
| h22 | ok / complete | ok / complete |
| d19 | ok / complete | ok / complete |
| d27 | ok / complete | ok / complete |
| d72_inner | ok / complete | ok / complete |
| d85 | ok / complete | ok / complete |

### Default SF0.1 exactness spotcheck

Five untimed default-query checks use isolated evidence, never the retained CPU performance measurements.

| Query | Configuration | Correctness | Status |
|---|---:|---:|---:|
| h01 | default t1 | ok | complete |
| h02 | default t1 | ok | complete |
| h06 | default t1 | ok | complete |
| h13 | default t1 | ok | complete |
| h19 | default t1 | ok | complete |

## SF1: phase comparison

All values are milliseconds. CPU phase columns are paired **instrumented profile** measurements; GPU and DuckDB are ordinary timings. Parallel **setup+build / final evaluation** are not the same accounting as default/LE/advised exclusive build / query residual. Final evaluation includes boundary/teardown residual. **Uninstrumented whole-query totals are primary** for speedups. CPU-LE is selected by the minimum plain median of `le` and `le_runs`.

| Query | GPU build | GPU query | GPU total | CPU-LE build | CPU-LE query | CPU-LE total | CPU-advised build | CPU-advised query | CPU-advised total | CPU-default build | CPU-default query | CPU-default total | CPU-par t24 setup+build | CPU-par t24 final evaluation | CPU-par t24 total | CPU-par t1 setup+build | CPU-par t1 final evaluation | CPU-par t1 total | DuckDB t1 | DuckDB t24 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| h01 | 0.002 | 66.294 | 66.295 | 0.000 | 5471.737 | 5471.737 (le) | 0.000 | 3013.778 | 3013.778 | 0.000 | 2846.514 | 2846.514 | 3418.814 | 32.502 | 3451.317 | 3894.723 | 28.561 | 3923.284 | 819.000 | 63.000 |
| h02 | 8.125 | 18.525 | 26.689 | 9.628 | 65.429 | 75.057 (le_runs) | 63.075 | 59.751 | 122.826 | 20.327 | 107.628 | 127.955 | 142.854 | 9.205 | 152.059 | 189.440 | 14.705 | 204.145 | 12.000 | 9.000 |
| h03 | 0.044 | 1.922 | 1.965 | 7.215 | 32.748 | 39.963 (le_runs) | 130.073 | 78.265 | 208.338 | 20.107 | 41.449 | 61.556 | 182.911 | 63.863 | 246.774 | 184.335 | 62.950 | 247.285 | 37.000 | 11.000 |
| h04 | 0.514 | 0.652 | 1.164 | 46.436 | 40.751 | 87.187 (le_runs) | 0.617 | 30.147 | 30.764 | 46.826 | 39.429 | 86.255 | 329.888 | 17.900 | 347.788 | 323.566 | 16.705 | 340.271 | 44.000 | 10.000 |
| h04_not_exists | 3.735 | 0.240 | 3.971 | 104.746 | 54.958 | 159.704 (le_runs) | 103.434 | 55.042 | 158.476 | 103.327 | 55.924 | 159.251 | 29.015 | 127.425 | 156.440 | 29.692 | 124.796 | 154.489 | 46.000 | 11.000 |
| h05 | 1.542 | 0.469 | 2.011 | 2.577 | 56.248 | 58.825 (le_runs) | 314.900 | 122.290 | 437.190 | 4.768 | 81.344 | 86.112 | 152.324 | 109.884 | 262.209 | 150.297 | 108.902 | 259.199 | 47.000 | 12.000 |
| h06 | 0.002 | 7.416 | 7.417 | 0.000 | 37.017 | 37.017 (le) | 0.000 | 36.187 | 36.187 | 0.000 | 36.102 | 36.102 | 35.543 | 0.003 | 35.546 | 34.471 | 0.003 | 34.474 | 36.000 | 7.000 |
| h07 | 1.665 | 1.172 | 2.838 | 3.826 | 26.844 | 30.670 (le_runs) | 202.133 | 109.758 | 311.891 | 14.560 | 33.244 | 47.804 | 126.450 | 11.085 | 137.535 | 109.352 | 10.957 | 120.309 | 48.000 | 14.000 |
| h07_block | 1.803 | 1.084 | 2.883 | 4.058 | 43.791 | 47.849 (le_runs) | 195.178 | 109.865 | 305.043 | 11.359 | 145.856 | 157.215 | 230.849 | 724.789 | 955.638 | 237.273 | 732.962 | 970.235 | 48.000 | 14.000 |
| h08 | 2.663 | 2.164 | 4.835 | 4.810 | 340.789 | 345.599 (le_runs) | 1089.728 | 142.679 | 1232.407 | 11.961 | 51.857 | 63.818 | 198.158 | 90.220 | 288.378 | 192.706 | 89.402 | 282.107 | 40.000 | 13.000 |
| h09 | 4.064 | 1.741 | 5.805 | — | — | >600 s timeout | 1074.692 | 377.499 | 1452.191 | — | — | >600 s timeout | 1218.442 | 314.598 | 1533.039 | 1321.946 | 320.345 | 1642.291 | 166.000 | 40.000 |
| h10 | 1.673 | 12.365 | 14.038 | 99.553 | 253.516 | 353.069 (le_runs) | 173.931 | 234.481 | 408.412 | 166.472 | 231.429 | 397.901 | 400.651 | 109.620 | 510.271 | 439.653 | 102.011 | 541.664 | 148.000 | 44.000 |
| h11 | 0.616 | 18.804 | 19.419 | 0.005 | 14.295 | 14.300 (le_runs) | 32.198 | 8.947 | 41.145 | 0.011 | 63.134 | 63.145 | 21.555 | 58.773 | 80.328 | 22.709 | 59.267 | 81.977 | 11.000 | 5.000 |
| h12 | 0.631 | 8.576 | 9.207 | 136.790 | 80.503 | 217.293 (le_runs) | 157.203 | 86.107 | 243.310 | 136.437 | 87.685 | 224.122 | 110.413 | 0.006 | 110.419 | 114.176 | 0.008 | 114.184 | 60.000 | 11.000 |
| h13 | 6.457 | 9.503 | 15.914 | 114.193 | 348.446 | 462.639 (le_runs) | 119.680 | 352.934 | 472.614 | 119.806 | 351.400 | 471.206 | 427.652 | 131.633 | 559.285 | 458.157 | 127.746 | 585.903 | 223.000 | 38.000 |
| h14 | CPU-only | CPU-only | CPU-only | 4.834 | 69.367 | 74.201 (le) | 4.865 | 71.352 | 76.217 | 5.027 | 69.937 | 74.964 | 79.649 | 0.003 | 79.652 | 95.244 | 0.004 | 95.247 | 44.000 | 18.000 |
| h15 | 0.261 | 14.633 | 14.895 | 0.262 | 64.234 | 64.496 (le_runs) | 0.358 | 63.309 | 63.667 | 0.264 | 63.576 | 63.840 | 84.359 | 0.330 | 84.690 | 102.124 | 0.121 | 102.245 | 48.000 | 11.000 |
| h16 | 1.353 | 0.251 | 1.604 | 16.878 | 25.702 | 42.580 (le) | 19.843 | 25.568 | 45.411 | 17.057 | 21.510 | 38.567 | 22.091 | 24.702 | 46.793 | 50.020 | 18.829 | 68.849 | 9.000 | 7.000 |
| h18 | 1.086 | 22.999 | 24.083 | 167.813 | 1375.637 | 1543.450 (le) | 165.853 | 1379.107 | 1544.960 | 170.014 | 1459.305 | 1629.319 | 1488.113 | 50.002 | 1538.115 | 2012.507 | 44.539 | 2057.046 | 722.000 | 81.000 |
| h19 | 0.002 | 9.253 | 9.254 | 9.996 | 66.763 | 76.759 (le) | 15.132 | 99.872 | 115.004 | 8.781 | 56.294 | 65.075 | 105.216 | 0.014 | 105.229 | 95.210 | 0.008 | 95.218 | 93.000 | 14.000 |
| h20 | 1.609 | 14.986 | 16.596 | 5.140 | 371.923 | 377.063 (le_runs) | 68.364 | 385.528 | 453.892 | 1.742 | 365.546 | 367.288 | 378.685 | 28.407 | 407.092 | 510.361 | 29.305 | 539.665 | 99.000 | 22.000 |
| h21_positive | 1.880 | 2.493 | 4.369 | 37.213 | 145.183 | 182.396 (le) | 426.302 | 320.309 | 746.611 | 45.927 | 239.165 | 285.092 | 526.433 | 206.019 | 732.452 | 681.246 | 207.110 | 888.357 | 111.000 | 20.000 |
| h22 | 0.868 | 0.148 | 1.015 | 17.905 | 3.770 | 21.675 (le) | 18.145 | 3.934 | 22.079 | 18.116 | 4.019 | 22.135 | 0.983 | 21.757 | 22.740 | 0.976 | 21.203 | 22.180 | 23.000 | 15.000 |
| d19 | 2.330 | 0.550 | 2.878 | 7.724 | 18.606 | 26.330 (le) | 346.406 | 75.643 | 422.049 | 7.833 | 20.569 | 28.402 | 175.990 | 43.341 | 219.331 | 174.168 | 43.574 | 217.742 | 35.000 | 11.000 |
| d27 | 0.793 | 1.812 | 2.610 | 1.527 | 28.502 | 30.029 (le_runs) | 1.732 | 40.275 | 42.007 | 1.645 | 31.935 | 33.580 | 142.416 | 139.479 | 281.895 | 139.558 | 147.381 | 286.938 | 42.000 | 10.000 |
| d72_inner | 1.331 | 2.988 | 4.319 | 57.646 | 126.881 | 184.527 (le_runs) | 177.116 | 109.836 | 286.952 | 79.904 | 163.406 | 243.310 | 344.905 | 195.525 | 540.431 | 370.929 | 186.265 | 557.194 | 116.000 | 70.000 |
| d85 | 2.106 | 1.465 | 3.571 | 8.647 | 29.113 | 37.760 (le) | 429.686 | 81.076 | 510.762 | 9.648 | 26.732 | 36.380 | 299.054 | 28.079 | 327.133 | 333.878 | 27.585 | 361.463 | 43.000 | 13.000 |

Coverage: 108 retained sequential configurations across 27 queries; 105 complete, 3 timeout; 0 configurations used three runs in at least one completed mode. Parallel coverage (of 27 each): par t24: 27 exact plain, 27 paired profiles, 0 configuration-budget timeouts; par t1: 27 exact plain, 27 paired profiles, 0 configuration-budget timeouts.

### SF1 uninstrumented CPU totals

These whole-query medians include construction and determine primary CPU speedups. Completed exact plain results remain shown when a later profile/counting mode times out.

| Query | CPU-LE total | CPU-advised total | CPU-default total | CPU-par t24 total | CPU-par t1 total | DuckDB t1 | DuckDB t24 |
|---|---:|---:|---:|---:|---:|---:|---:|
| h01 | 3017.642 | 2656.798 | 2675.986 | 3025.543 | 3565.095 | 819.000 | 63.000 |
| h02 | 49.400 | 128.540 | 63.620 | 96.532 | 131.162 | 12.000 | 9.000 |
| h03 | 32.536 | 133.097 | 57.806 | 134.693 | 137.294 | 37.000 | 11.000 |
| h04 | 49.934 | 23.244 | 50.404 | 221.383 | 222.459 | 44.000 | 10.000 |
| h04_not_exists | 166.242 | 167.519 | 167.514 | 165.734 | 167.403 | 46.000 | 11.000 |
| h05 | 36.805 | 390.747 | 41.987 | 132.849 | 139.833 | 47.000 | 12.000 |
| h06 | 25.249 | 24.538 | 24.971 | 24.091 | 24.433 | 36.000 | 7.000 |
| h07 | 27.923 | 257.435 | 39.322 | 84.936 | 95.932 | 48.000 | 14.000 |
| h07_block | 41.254 | 253.724 | 73.726 | 712.324 | 708.166 | 48.000 | 14.000 |
| h08 | 60.004 | 983.256 | 39.276 | 176.339 | 177.396 | 40.000 | 13.000 |
| h09 | >600 s timeout | 1162.877 | >600 s timeout | 1351.719 | 1429.470 | 166.000 | 40.000 |
| h10 | 305.760 | 348.885 | 330.515 | 420.571 | 447.412 | 148.000 | 44.000 |
| h11 | 10.328 | 45.122 | 17.479 | 42.747 | 42.078 | 11.000 | 5.000 |
| h12 | 178.829 | 203.075 | 185.366 | 113.796 | 116.259 | 60.000 | 11.000 |
| h13 | 451.932 | 473.216 | 463.084 | 496.609 | 540.895 | 223.000 | 38.000 |
| h14 | 65.537 | 67.724 | 67.493 | 75.784 | 76.887 | 44.000 | 18.000 |
| h15 | 37.281 | 37.375 | 36.780 | 65.615 | 89.329 | 48.000 | 11.000 |
| h16 | 43.328 | 40.924 | 40.938 | 35.814 | 49.141 | 9.000 | 7.000 |
| h18 | 1349.829 | 1330.657 | 1326.745 | 1144.134 | 1673.410 | 722.000 | 81.000 |
| h19 | 50.912 | 83.001 | 101.599 | 73.830 | 73.374 | 93.000 | 14.000 |
| h20 | 330.369 | 401.665 | 327.993 | 288.998 | 419.843 | 99.000 | 22.000 |
| h21_positive | 150.455 | 616.978 | 207.703 | 484.392 | 620.673 | 111.000 | 20.000 |
| h22 | 22.695 | 22.855 | 23.738 | 24.029 | 24.033 | 23.000 | 15.000 |
| d19 | 21.093 | 387.530 | 21.450 | 115.557 | 110.824 | 35.000 | 11.000 |
| d27 | 16.543 | 44.204 | 18.064 | 210.743 | 198.343 | 42.000 | 10.000 |
| d72_inner | 142.113 | 236.778 | 200.163 | 334.368 | 380.919 | 116.000 | 70.000 |
| d85 | 29.560 | 428.292 | 28.429 | 195.459 | 210.287 | 43.000 | 13.000 |

## SF10: phase comparison

All values are milliseconds. CPU phase columns are paired **instrumented profile** measurements; GPU and DuckDB are ordinary timings. Parallel **setup+build / final evaluation** are not the same accounting as default/LE/advised exclusive build / query residual. Final evaluation includes boundary/teardown residual. **Uninstrumented whole-query totals are primary** for speedups. CPU-LE is selected by the minimum plain median of `le` and `le_runs`.

| Query | GPU build | GPU query | GPU total | CPU-LE build | CPU-LE query | CPU-LE total | CPU-advised build | CPU-advised query | CPU-advised total | CPU-default build | CPU-default query | CPU-default total | CPU-par t24 setup+build | CPU-par t24 final evaluation | CPU-par t24 total | CPU-par t1 setup+build | CPU-par t1 final evaluation | CPU-par t1 total | DuckDB t1 | DuckDB t24 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| h01 | 0.002 | 591.959 | 591.961 | — | — | >600 s timeout | — | — | >600 s timeout | — | — | >600 s timeout | — | — | >100 s timeout | — | — | >100 s timeout | 9431.000 | 531.000 |
| h02 | 217.414 | 113.961 | 331.269 | 155.739 | 903.716 | 1059.455 (le_runs) | 1280.914 | 1157.691 | 2438.605 | 298.778 | 1396.583 | 1695.361 | 2143.338 | 251.300 | 2394.638 | 2437.349 | 239.315 | 2676.664 | 137.000 | 33.000 |
| h03 | 0.089 | 17.449 | 17.537 | 95.751 | 443.438 | 539.189 (le_runs) | 5098.104 | 1184.874 | 6282.978 | 204.295 | 539.140 | 743.435 | 1966.409 | 885.172 | 2851.580 | 2039.574 | 956.024 | 2995.597 | 566.000 | 57.000 |
| h04 | 5.014 | 5.403 | 10.420 | 5240.084 | 811.306 | 6051.390 (le) | 158.651 | 761.343 | 919.994 | 5229.588 | 812.524 | 6042.112 | — | — | >100 s timeout | — | — | >100 s timeout | 459.000 | 50.000 |
| h04_not_exists | 44.211 | 1.212 | 45.430 | 1646.462 | 565.264 | 2211.726 (le_runs) | 1643.291 | 550.408 | 2193.699 | 1683.748 | 554.727 | 2238.475 | 242.089 | 2167.517 | 2409.606 | 245.916 | 2285.151 | 2531.068 | 475.000 | 63.000 |
| h05 | 9.137 | 1.164 | 10.314 | 81.059 | 928.382 | 1009.441 (le) | 5136.249 | 1479.485 | 6615.734 | 72.995 | 1059.461 | 1132.456 | 1590.989 | 2052.785 | 3643.773 | 1672.396 | 2203.728 | 3876.124 | 523.000 | 56.000 |
| h06 | 0.002 | 13.439 | 13.441 | 0.000 | 535.059 | 535.059 (le) | 0.000 | 532.734 | 532.734 | 0.000 | 533.836 | 533.836 | 522.739 | 0.003 | 522.742 | 519.389 | 0.004 | 519.393 | 403.000 | 39.000 |
| h07 | 13.098 | 1.968 | 15.048 | 172.658 | 420.560 | 593.218 (le) | 4716.723 | 1692.505 | 6409.228 | 160.484 | 418.738 | 579.222 | 1397.695 | 179.567 | 1577.262 | 1452.652 | 197.089 | 1649.741 | 604.000 | 61.000 |
| h07_block | 13.170 | 1.911 | 15.084 | 175.441 | 706.900 | 882.341 (le) | 4973.764 | 1741.846 | 6715.610 | 152.279 | 1497.246 | 1649.525 | — | — | >100 s timeout | — | — | >100 s timeout | 605.000 | 61.000 |
| h08 | 24.904 | 2.766 | 27.662 | 99.327 | 4190.165 | 4289.492 (le) | — | — | >100 s timeout | 132.676 | 616.426 | 749.102 | 2098.815 | 1850.478 | 3949.293 | 2260.119 | 1867.227 | 4127.347 | 838.000 | 80.000 |
| h09 | 40.861 | 3.607 | 44.563 | — | — | >100 s timeout | — | — | >100 s timeout | — | — | >100 s timeout | — | — | >100 s timeout | — | — | >100 s timeout | 2643.000 | 250.000 |
| h10 | 11.808 | 46.979 | 58.787 | 1000.914 | 2886.995 | 3887.909 (le_runs) | 1966.679 | 3108.271 | 5074.950 | 1965.405 | 3114.413 | 5079.818 | — | — | >100 s timeout | — | — | >100 s timeout | 1658.000 | 207.000 |
| h11 | 2.336 | 41.860 | 44.196 | 0.048 | 156.610 | 156.658 (le_runs) | 715.580 | 249.265 | 964.845 | 0.114 | 625.529 | 625.643 | 344.461 | 725.695 | 1070.156 | 367.841 | 883.626 | 1251.466 | 100.000 | 15.000 |
| h12 | 6.541 | 21.362 | 27.904 | 2249.878 | 914.660 | 3164.538 (le_runs) | 2252.839 | 936.069 | 3188.908 | 2259.324 | 927.198 | 3186.522 | 1455.830 | 29.016 | 1484.846 | 1517.476 | 25.985 | 1543.461 | 544.000 | 55.000 |
| h13 | 90.545 | 24.698 | 115.215 | 4116.506 | 5831.543 | 9948.049 (le) | 4157.649 | 5891.410 | 10049.059 | 4065.632 | 5849.338 | 9914.970 | — | — | >100 s timeout | — | — | >100 s timeout | 3484.000 | 286.000 |
| h14 | CPU-only | CPU-only | CPU-only | 240.159 | 900.599 | 1140.758 (le_runs) | 241.916 | 887.772 | 1129.688 | 243.198 | 898.822 | 1142.020 | 1748.634 | 4.894 | 1753.528 | 1688.459 | 4.920 | 1693.379 | 583.000 | 60.000 |
| h15 | 1.709 | 19.257 | 20.966 | 3.260 | 924.013 | 927.273 (le_runs) | 4.035 | 910.366 | 914.401 | 3.216 | 923.515 | 926.731 | 1161.372 | 10.049 | 1171.421 | 1598.782 | 12.412 | 1611.194 | 591.000 | 64.000 |
| h16 | 9.561 | 0.460 | 10.023 | 439.635 | 536.980 | 976.615 (le_runs) | 436.414 | 373.434 | 809.848 | 442.066 | 391.892 | 833.958 | 388.637 | 446.786 | 835.422 | 651.788 | 385.739 | 1037.527 | 156.000 | 24.000 |
| h18 | 8.594 | 166.324 | 174.924 | 2794.608 | 23313.767 | 26108.375 (le) | 2751.547 | 22937.331 | 25688.878 | 2725.572 | 22927.674 | 25653.246 | — | — | >100 s timeout | — | — | >100 s timeout | 9221.000 | 746.000 |
| h19 | 0.002 | 24.373 | 24.375 | 179.105 | 898.733 | 1077.838 (le_runs) | 179.939 | 842.718 | 1022.657 | 181.271 | 832.290 | 1013.561 | — | — | >100 s timeout | — | — | >100 s timeout | 911.000 | 74.000 |
| h20 | 8.493 | 90.159 | 98.710 | 83.246 | 6002.543 | 6085.789 (le) | 1253.098 | 5679.675 | 6932.773 | 44.358 | 5876.727 | 5921.085 | 5622.902 | 494.390 | 6117.292 | — | — | >100 s timeout | 1439.000 | 128.000 |
| h21_positive | 19.950 | 20.200 | 40.158 | 509.424 | 1436.558 | 1945.982 (le_runs) | — | — | >100 s timeout | 673.218 | 2638.130 | 3311.348 | — | — | >100 s timeout | — | — | >100 s timeout | 1509.000 | 136.000 |
| h22 | 6.223 | 0.191 | 6.414 | 634.356 | 168.101 | 802.457 (le) | 627.281 | 162.779 | 790.060 | 643.993 | 164.743 | 808.736 | 21.009 | 706.112 | 727.122 | 21.309 | 762.429 | 783.737 | 463.000 | 62.000 |
| d19 | 18.631 | 0.708 | 19.341 | 170.908 | 238.883 | 409.791 (le_runs) | 5525.091 | 886.363 | 6411.454 | 173.044 | 252.049 | 425.093 | 1984.107 | 600.705 | 2584.812 | 1946.541 | 662.120 | 2608.661 | 420.000 | 45.000 |
| d27 | 5.315 | 14.029 | 19.343 | 170.077 | 936.197 | 1106.274 (le) | 159.967 | 410.910 | 570.877 | 159.253 | 902.955 | 1062.208 | 1529.851 | 1560.059 | 3089.910 | 1649.511 | 2140.197 | 3789.707 | 774.000 | 68.000 |
| d72_inner | 8.078 | 17.540 | 25.623 | 1506.139 | 1877.459 | 3383.598 (le_runs) | 4327.880 | 1564.585 | 5892.465 | 1903.303 | 2660.757 | 4564.060 | — | — | >100 s timeout | — | — | >100 s timeout | 1507.000 | 159.000 |
| d85 | 17.782 | 9.149 | 26.935 | 179.014 | 403.227 | 582.241 (le_runs) | — | — | >100 s timeout | 173.567 | 339.132 | 512.699 | 3066.547 | 376.099 | 3442.646 | 4104.969 | 466.320 | 4571.289 | 519.000 | 57.000 |

Coverage: 108 retained sequential configurations across 27 queries; 88 complete, 9 plain_only_not_selected, 11 timeout; 1 configurations used three runs in at least one completed mode. Parallel coverage (of 27 each): par t24: 24 exact plain, 17 paired profiles, 10 configuration-budget timeouts; par t1: 23 exact plain, 16 paired profiles, 11 configuration-budget timeouts.

### SF10 uninstrumented CPU totals

These whole-query medians include construction and determine primary CPU speedups. Completed exact plain results remain shown when a later profile/counting mode times out.

| Query | CPU-LE total | CPU-advised total | CPU-default total | CPU-par t24 total | CPU-par t1 total | DuckDB t1 | DuckDB t24 |
|---|---:|---:|---:|---:|---:|---:|---:|
| h01 | 42990.346 | 42655.486 | 43062.649 | >100 s timeout | >100 s timeout | 9431.000 | 531.000 |
| h02 | 1078.245 | 1973.574 | 1329.774 | 1646.082 | 1917.986 | 137.000 | 33.000 |
| h03 | 503.634 | 6193.515 | 681.095 | 1900.952 | 1967.630 | 566.000 | 57.000 |
| h04 | 5727.454 | 835.487 | 5755.165 | 7489.179 | 8428.934 | 459.000 | 50.000 |
| h04_not_exists | 2171.690 | 2178.073 | 2242.424 | 2248.551 | 2319.659 | 475.000 | 63.000 |
| h05 | 750.828 | 5811.544 | 679.672 | 2308.683 | 2698.216 | 523.000 | 56.000 |
| h06 | 442.199 | 443.559 | 441.270 | 437.950 | 436.121 | 403.000 | 39.000 |
| h07 | 544.212 | 5314.043 | 524.141 | 1321.348 | 1404.279 | 604.000 | 61.000 |
| h07_block | 689.782 | 5541.788 | 783.912 | >100 s timeout | >100 s timeout | 605.000 | 61.000 |
| h08 | 1270.142 | 18587.764 | 622.492 | 2889.549 | 2936.354 | 838.000 | 80.000 |
| h09 | >100 s timeout | >100 s timeout | >100 s timeout | >100 s timeout | >100 s timeout | 2643.000 | 250.000 |
| h10 | 3341.593 | 4171.500 | 4139.860 | 5647.261 | 6210.944 | 1658.000 | 207.000 |
| h11 | 146.347 | 697.315 | 209.818 | 462.378 | 616.666 | 100.000 | 15.000 |
| h12 | 2798.779 | 2773.156 | 2774.910 | 1188.692 | 1249.352 | 544.000 | 55.000 |
| h13 | 9687.810 | 10138.350 | 9670.642 | 10350.055 | 10954.063 | 3484.000 | 286.000 |
| h14 | 994.480 | 982.689 | 994.722 | 1512.036 | 1569.134 | 583.000 | 60.000 |
| h15 | 849.570 | 853.291 | 856.435 | 927.179 | 1318.449 | 591.000 | 64.000 |
| h16 | 810.836 | 737.909 | 771.604 | 517.588 | 821.356 | 156.000 | 24.000 |
| h18 | 23709.349 | 23643.970 | 23540.553 | 18096.441 | >100 s timeout | 9221.000 | 746.000 |
| h19 | 925.780 | 1029.923 | 1105.112 | 2350.588 | 2640.158 | 911.000 | 74.000 |
| h20 | 5690.554 | 6641.867 | 5489.040 | 4584.979 | 6565.655 | 1439.000 | 128.000 |
| h21_positive | 1878.550 | 8926.413 | 2501.338 | 5461.767 | 7118.547 | 1509.000 | 136.000 |
| h22 | 1264.351 | 1269.022 | 1257.905 | 1212.002 | 1256.249 | 463.000 | 62.000 |
| d19 | 353.958 | 5541.231 | 366.826 | 1423.642 | 1438.923 | 420.000 | 45.000 |
| d27 | 871.748 | 558.667 | 883.822 | 2083.609 | 2989.249 | 774.000 | 68.000 |
| d72_inner | 2825.684 | 5354.696 | 3752.724 | 5654.296 | 6685.689 | 1507.000 | 159.000 |
| d85 | 510.415 | 8029.475 | 445.691 | 2222.800 | 3129.687 | 519.000 | 57.000 |

## Geometric-mean speedups

Speedup = comparator total / GPU total; **greater than 1× favors GPU**. The geometric mean is `exp(mean(log(comparator_ms / gpu_ms)))`, equally weighting queries. The denominator counts common exact GPU queries with a completed numeric comparator measurement. h14 is excluded, as are unavailable/timed-out measurements; a timeout's elapsed wall time is never included. Different denominator sets are not directly comparable aggregate rankings. **Uninstrumented CPU comparisons are primary**; profile comparisons only describe the phase-table measurements.

| Scale | Comparator | Common exact queries (n) | Geometric-mean GPU speedup |
|---|---:|---:|---:|
| SF1 | duckdb_t1_ms | 26 | 10.314× |
| SF1 | duckdb_t24_ms | 26 | 2.768× |
| SF1 | cpu_par_t24_uninstrumented_total_ms | 26 | 31.227× |
| SF1 | cpu_par_t24_profile_total_ms | 26 | 43.740× |
| SF1 | cpu_par_t1_uninstrumented_total_ms | 26 | 34.540× |
| SF1 | cpu_par_t1_profile_total_ms | 26 | 46.862× |
| SF1 | cpu_le_uninstrumented_total_ms | 25 | 12.999× |
| SF1 | cpu_advised_uninstrumented_total_ms | 26 | 33.067× |
| SF1 | cpu_default_uninstrumented_total_ms | 25 | 14.881× |
| SF1 | cpu_le_profile_total_ms | 25 | 17.863× |
| SF1 | cpu_advised_profile_total_ms | 26 | 38.739× |
| SF1 | cpu_default_profile_total_ms | 25 | 20.061× |
| SF10 | duckdb_t1_ms | 26 | 23.686× |
| SF10 | duckdb_t24_ms | 26 | 2.492× |
| SF10 | cpu_par_t24_uninstrumented_total_ms | 23 | 76.215× |
| SF10 | cpu_par_t24_profile_total_ms | 16 | 76.832× |
| SF10 | cpu_par_t1_uninstrumented_total_ms | 22 | 88.230× |
| SF10 | cpu_par_t1_profile_total_ms | 15 | 87.042× |
| SF10 | cpu_le_uninstrumented_total_ms | 25 | 47.429× |
| SF10 | cpu_advised_uninstrumented_total_ms | 25 | 103.589× |
| SF10 | cpu_default_uninstrumented_total_ms | 25 | 49.287× |
| SF10 | cpu_le_profile_total_ms | 24 | 53.675× |
| SF10 | cpu_advised_profile_total_ms | 21 | 96.064× |
| SF10 | cpu_default_profile_total_ms | 24 | 58.023× |

## Common-set parallel speedups

These summaries use the **same completed exact query set for CPU-default, CPU-par t1/t24 and DuckDB t1/t24** at each scale. GPU common-set summaries intersect that set with completed exact GPU results. h14 may participate in CPU/DuckDB comparisons but never GPU comparisons. Missing, failed, timed-out and zero-resolution timings are excluded from geometric means; no capped wall duration is substituted. Each JSON summary retains its query list. Speedup is numerator / denominator, so **greater than 1× favors the named denominator**. The historical pairwise GPU summaries above remain available and retain their original CPU metrics.

| Scale | Common set | Numerator | Denominator (favored above 1×) | n | Geometric-mean speedup |
|---|---:|---:|---:|---:|---:|
| SF1 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_default_uninstrumented_total_ms | cpu_par_t1_uninstrumented_total_ms | 26 | 0.478× |
| SF1 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_par_t24_uninstrumented_total_ms | cpu_par_t1_uninstrumented_total_ms | 26 | 0.906× |
| SF1 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t1_ms | cpu_par_t1_uninstrumented_total_ms | 26 | 0.318× |
| SF1 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t24_ms | cpu_par_t1_uninstrumented_total_ms | 26 | 0.087× |
| SF1 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_default_uninstrumented_total_ms | cpu_par_t24_uninstrumented_total_ms | 26 | 0.527× |
| SF1 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_par_t1_uninstrumented_total_ms | cpu_par_t24_uninstrumented_total_ms | 26 | 1.104× |
| SF1 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t1_ms | cpu_par_t24_uninstrumented_total_ms | 26 | 0.351× |
| SF1 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t24_ms | cpu_par_t24_uninstrumented_total_ms | 26 | 0.096× |
| SF1 | gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_default_uninstrumented_total_ms | gpu_total_ms | 25 | 14.881× |
| SF1 | gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_par_t1_uninstrumented_total_ms | gpu_total_ms | 25 | 31.931× |
| SF1 | gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_par_t24_uninstrumented_total_ms | gpu_total_ms | 25 | 28.816× |
| SF1 | gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t1_ms | gpu_total_ms | 25 | 9.902× |
| SF1 | gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t24_ms | gpu_total_ms | 25 | 2.668× |
| SF10 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_default_uninstrumented_total_ms | cpu_par_t1_uninstrumented_total_ms | 23 | 0.527× |
| SF10 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_par_t24_uninstrumented_total_ms | cpu_par_t1_uninstrumented_total_ms | 23 | 0.856× |
| SF10 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t1_ms | cpu_par_t1_uninstrumented_total_ms | 23 | 0.251× |
| SF10 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t24_ms | cpu_par_t1_uninstrumented_total_ms | 23 | 0.028× |
| SF10 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_default_uninstrumented_total_ms | cpu_par_t24_uninstrumented_total_ms | 23 | 0.616× |
| SF10 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_par_t1_uninstrumented_total_ms | cpu_par_t24_uninstrumented_total_ms | 23 | 1.168× |
| SF10 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t1_ms | cpu_par_t24_uninstrumented_total_ms | 23 | 0.293× |
| SF10 | cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t24_ms | cpu_par_t24_uninstrumented_total_ms | 23 | 0.032× |
| SF10 | gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_default_uninstrumented_total_ms | gpu_total_ms | 22 | 46.150× |
| SF10 | gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_par_t1_uninstrumented_total_ms | gpu_total_ms | 22 | 88.230× |
| SF10 | gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24 | cpu_par_t24_uninstrumented_total_ms | gpu_total_ms | 22 | 75.164× |
| SF10 | gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t1_ms | gpu_total_ms | 22 | 21.777× |
| SF10 | gpu_cpu_default_par_t1_par_t24_duckdb_t1_t24 | duckdb_t24_ms | gpu_total_ms | 22 | 2.402× |

## Queries where GPU is slower

- SF1, duckdb_t1_ms: h02, h11.
- SF1, duckdb_t24_ms: h01, h02, h06, h11, h15.
- SF1, cpu_le_uninstrumented_total_ms: h11.
- SF1, cpu_advised_uninstrumented_total_ms: none among completed measurements.
- SF1, cpu_default_uninstrumented_total_ms: h11.
- SF1, cpu_par_t1_uninstrumented_total_ms: none among completed measurements.
- SF1, cpu_par_t24_uninstrumented_total_ms: none among completed measurements.
- SF10, duckdb_t1_ms: h02.
- SF10, duckdb_t24_ms: h01, h02, h11.
- SF10, cpu_le_uninstrumented_total_ms: none among completed measurements.
- SF10, cpu_advised_uninstrumented_total_ms: none among completed measurements.
- SF10, cpu_default_uninstrumented_total_ms: none among completed measurements.
- SF10, cpu_par_t1_uninstrumented_total_ms: none among completed measurements.
- SF10, cpu_par_t24_uninstrumented_total_ms: none among completed measurements.

h02 includes host fallback/intermediate publication; h11 and h01 spend substantial GPU time in grouping/reduction/publication. These phase allocations are not hardware-counter diagnoses: occupancy, bandwidth and cache misses are not inferred. Differences near DuckDB's 1 ms timer resolution should not be treated as precise rankings.

Machine-readable outputs: [comparison TSV](performance_comparison.tsv), [speedups JSON](performance_speedups.json), [CPU SF1](performance_cpu_sf1.json), [CPU SF10](performance_cpu_sf10.json), [GPU summary](performance_gpu_summary.json). Retained raw CPU measurement artifacts are under `performance_cpu_raw/sf1` and `performance_cpu_raw/sf10`. Parallel JSON/TSV inputs are `performance_cpu_par_t{1,24}_sf{0.1,1,10}.{json,tsv}`, with per-thread raw artifacts under `performance_cpu_raw/par/t{1,24}/sf{0.1,1,10}`. Speedups JSON retains historical `results` and adds `common_results` with explicit numerator, denominator, query lists and counts.
