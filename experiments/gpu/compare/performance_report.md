# CQ TPC-H-shaped performance comparison

## Scope and methodology

These are the **27 authored CQ-shaped catalog queries**, not full standard TPC-H coverage: 26 GPU-suite queries plus **h14, CPU-only**, at SF1 and SF10. GPU name `hXX`/`dXX` maps to CPU catalog name `retail_hXX`/`retail_dXX`. Numerical results require exact answers; no timeout is interpreted as an execution time. All times are milliseconds, lower is better.

Hardware: **AMD Ryzen Threadripper PRO 5945WX, 12 cores / 24 hardware threads**, approximately **125.77 GiB RAM**, **NVIDIA RTX 6000 Ada Generation (49,140 MiB)**, driver **580.178.04**. GPU compilation targeted `sm_89` with CUDA 12.9 (NVIDIA HPC SDK 25.5). See [hardware](performance_hardware.json). CPU workers are sequential, and timed CPU windows are serialized with no concurrent compilation.

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
python3 experiments/gpu/compare/performance_report.py
```

The collector also exposes `adaptive`, `build`, `prepare`, `measure`, `consolidate`, and `cleanup` phases. Cleanup removes duplicate answers/oracles after exact checks while preserving measurement evidence; prepare regenerates the oracles. GPU reproduction (inside `experiments/gpu`):

```sh
CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9 CARGO_BUILD_JOBS=1 cargo oxide build --arch sm_89 -- --release -p cq-gpu-tpch
./target/release/cq-gpu-tpch compare/performance_gpu.json --mode advised
./target/release/cq-gpu-tpch compare/performance_gpu_sf10_remaining.json --mode advised --scale sf10 --query d27 --query d72_inner --query d85
./target/release/cq-gpu-tpch compare/gpu_tpch_intern.results.json --mode advised
```

DuckDB reproduction: `python3 experiments/gpu/compare/performance_duck.py`.
## SF1: phase comparison

All values are milliseconds. CPU build/query/total are paired **instrumented profile** measurements; GPU and DuckDB are ordinary timings. CPU-LE is selected by the minimum **plain** median of `le` and `le_runs`.

| Query | GPU build | GPU query | GPU total | CPU-LE build | CPU-LE query | CPU-LE total | CPU-advised build | CPU-advised query | CPU-advised total | CPU-default build | CPU-default query | CPU-default total | DuckDB t1 | DuckDB t24 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| h01 | 0.002 | 66.294 | 66.295 | 0.000 | 5471.737 | 5471.737 (le) | 0.000 | 3013.778 | 3013.778 | 0.000 | 2846.514 | 2846.514 | 819.000 | 63.000 |
| h02 | 8.125 | 18.525 | 26.689 | 9.628 | 65.429 | 75.057 (le_runs) | 63.075 | 59.751 | 122.826 | 20.327 | 107.628 | 127.955 | 12.000 | 9.000 |
| h03 | 0.044 | 1.922 | 1.965 | 7.215 | 32.748 | 39.963 (le_runs) | 130.073 | 78.265 | 208.338 | 20.107 | 41.449 | 61.556 | 37.000 | 11.000 |
| h04 | 0.514 | 0.652 | 1.164 | 46.436 | 40.751 | 87.187 (le_runs) | 0.617 | 30.147 | 30.764 | 46.826 | 39.429 | 86.255 | 44.000 | 10.000 |
| h04_not_exists | 3.735 | 0.240 | 3.971 | 104.746 | 54.958 | 159.704 (le_runs) | 103.434 | 55.042 | 158.476 | 103.327 | 55.924 | 159.251 | 46.000 | 11.000 |
| h05 | 1.542 | 0.469 | 2.011 | 2.577 | 56.248 | 58.825 (le_runs) | 314.900 | 122.290 | 437.190 | 4.768 | 81.344 | 86.112 | 47.000 | 12.000 |
| h06 | 0.002 | 7.416 | 7.417 | 0.000 | 37.017 | 37.017 (le) | 0.000 | 36.187 | 36.187 | 0.000 | 36.102 | 36.102 | 36.000 | 7.000 |
| h07 | 1.665 | 1.172 | 2.838 | 3.826 | 26.844 | 30.670 (le_runs) | 202.133 | 109.758 | 311.891 | 14.560 | 33.244 | 47.804 | 48.000 | 14.000 |
| h07_block | 1.803 | 1.084 | 2.883 | 4.058 | 43.791 | 47.849 (le_runs) | 195.178 | 109.865 | 305.043 | 11.359 | 145.856 | 157.215 | 48.000 | 14.000 |
| h08 | 2.663 | 2.164 | 4.835 | 4.810 | 340.789 | 345.599 (le_runs) | 1089.728 | 142.679 | 1232.407 | 11.961 | 51.857 | 63.818 | 40.000 | 13.000 |
| h09 | 4.064 | 1.741 | 5.805 | — | — | >600 s timeout | 1074.692 | 377.499 | 1452.191 | — | — | >600 s timeout | 166.000 | 40.000 |
| h10 | 1.673 | 12.365 | 14.038 | 99.553 | 253.516 | 353.069 (le_runs) | 173.931 | 234.481 | 408.412 | 166.472 | 231.429 | 397.901 | 148.000 | 44.000 |
| h11 | 0.616 | 18.804 | 19.419 | 0.005 | 14.295 | 14.300 (le_runs) | 32.198 | 8.947 | 41.145 | 0.011 | 63.134 | 63.145 | 11.000 | 5.000 |
| h12 | 0.631 | 8.576 | 9.207 | 136.790 | 80.503 | 217.293 (le_runs) | 157.203 | 86.107 | 243.310 | 136.437 | 87.685 | 224.122 | 60.000 | 11.000 |
| h13 | 6.457 | 9.503 | 15.914 | 114.193 | 348.446 | 462.639 (le_runs) | 119.680 | 352.934 | 472.614 | 119.806 | 351.400 | 471.206 | 223.000 | 38.000 |
| h14 | CPU-only | CPU-only | CPU-only | 4.834 | 69.367 | 74.201 (le) | 4.865 | 71.352 | 76.217 | 5.027 | 69.937 | 74.964 | 44.000 | 18.000 |
| h15 | 0.261 | 14.633 | 14.895 | 0.262 | 64.234 | 64.496 (le_runs) | 0.358 | 63.309 | 63.667 | 0.264 | 63.576 | 63.840 | 48.000 | 11.000 |
| h16 | 1.353 | 0.251 | 1.604 | 16.878 | 25.702 | 42.580 (le) | 19.843 | 25.568 | 45.411 | 17.057 | 21.510 | 38.567 | 9.000 | 7.000 |
| h18 | 1.086 | 22.999 | 24.083 | 167.813 | 1375.637 | 1543.450 (le) | 165.853 | 1379.107 | 1544.960 | 170.014 | 1459.305 | 1629.319 | 722.000 | 81.000 |
| h19 | 0.002 | 9.253 | 9.254 | 9.996 | 66.763 | 76.759 (le) | 15.132 | 99.872 | 115.004 | 8.781 | 56.294 | 65.075 | 93.000 | 14.000 |
| h20 | 1.609 | 14.986 | 16.596 | 5.140 | 371.923 | 377.063 (le_runs) | 68.364 | 385.528 | 453.892 | 1.742 | 365.546 | 367.288 | 99.000 | 22.000 |
| h21_positive | 1.880 | 2.493 | 4.369 | 37.213 | 145.183 | 182.396 (le) | 426.302 | 320.309 | 746.611 | 45.927 | 239.165 | 285.092 | 111.000 | 20.000 |
| h22 | 0.868 | 0.148 | 1.015 | 17.905 | 3.770 | 21.675 (le) | 18.145 | 3.934 | 22.079 | 18.116 | 4.019 | 22.135 | 23.000 | 15.000 |
| d19 | 2.330 | 0.550 | 2.878 | 7.724 | 18.606 | 26.330 (le) | 346.406 | 75.643 | 422.049 | 7.833 | 20.569 | 28.402 | 35.000 | 11.000 |
| d27 | 0.793 | 1.812 | 2.610 | 1.527 | 28.502 | 30.029 (le_runs) | 1.732 | 40.275 | 42.007 | 1.645 | 31.935 | 33.580 | 42.000 | 10.000 |
| d72_inner | 1.331 | 2.988 | 4.319 | 57.646 | 126.881 | 184.527 (le_runs) | 177.116 | 109.836 | 286.952 | 79.904 | 163.406 | 243.310 | 116.000 | 70.000 |
| d85 | 2.106 | 1.465 | 3.571 | 8.647 | 29.113 | 37.760 (le) | 429.686 | 81.076 | 510.762 | 9.648 | 26.732 | 36.380 | 43.000 | 13.000 |

Coverage: 108 configurations attempted across 27 queries; 105 complete, 3 timeout; 0 configurations used three runs in at least one completed mode.

### SF1 uninstrumented CPU totals

These whole-query medians include index construction and determine the primary CPU speedups.

| Query | CPU-LE total | CPU-advised total | CPU-default total |
|---|---:|---:|---:|
| h01 | 3017.642 | 2656.798 | 2675.986 |
| h02 | 49.400 | 128.540 | 63.620 |
| h03 | 32.536 | 133.097 | 57.806 |
| h04 | 49.934 | 23.244 | 50.404 |
| h04_not_exists | 166.242 | 167.519 | 167.514 |
| h05 | 36.805 | 390.747 | 41.987 |
| h06 | 25.249 | 24.538 | 24.971 |
| h07 | 27.923 | 257.435 | 39.322 |
| h07_block | 41.254 | 253.724 | 73.726 |
| h08 | 60.004 | 983.256 | 39.276 |
| h09 | >600 s timeout | 1162.877 | >600 s timeout |
| h10 | 305.760 | 348.885 | 330.515 |
| h11 | 10.328 | 45.122 | 17.479 |
| h12 | 178.829 | 203.075 | 185.366 |
| h13 | 451.932 | 473.216 | 463.084 |
| h14 | 65.537 | 67.724 | 67.493 |
| h15 | 37.281 | 37.375 | 36.780 |
| h16 | 43.328 | 40.924 | 40.938 |
| h18 | 1349.829 | 1330.657 | 1326.745 |
| h19 | 50.912 | 83.001 | 101.599 |
| h20 | 330.369 | 401.665 | 327.993 |
| h21_positive | 150.455 | 616.978 | 207.703 |
| h22 | 22.695 | 22.855 | 23.738 |
| d19 | 21.093 | 387.530 | 21.450 |
| d27 | 16.543 | 44.204 | 18.064 |
| d72_inner | 142.113 | 236.778 | 200.163 |
| d85 | 29.560 | 428.292 | 28.429 |

## SF10: phase comparison

All values are milliseconds. CPU build/query/total are paired **instrumented profile** measurements; GPU and DuckDB are ordinary timings. CPU-LE is selected by the minimum **plain** median of `le` and `le_runs`.

| Query | GPU build | GPU query | GPU total | CPU-LE build | CPU-LE query | CPU-LE total | CPU-advised build | CPU-advised query | CPU-advised total | CPU-default build | CPU-default query | CPU-default total | DuckDB t1 | DuckDB t24 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| h01 | 0.002 | 591.959 | 591.961 | — | — | >600 s timeout | — | — | >600 s timeout | — | — | >600 s timeout | 9431.000 | 531.000 |
| h02 | 217.414 | 113.961 | 331.269 | 155.739 | 903.716 | 1059.455 (le_runs) | 1280.914 | 1157.691 | 2438.605 | 298.778 | 1396.583 | 1695.361 | 137.000 | 33.000 |
| h03 | 0.089 | 17.449 | 17.537 | 95.751 | 443.438 | 539.189 (le_runs) | 5098.104 | 1184.874 | 6282.978 | 204.295 | 539.140 | 743.435 | 566.000 | 57.000 |
| h04 | 5.014 | 5.403 | 10.420 | 5240.084 | 811.306 | 6051.390 (le) | 158.651 | 761.343 | 919.994 | 5229.588 | 812.524 | 6042.112 | 459.000 | 50.000 |
| h04_not_exists | 44.211 | 1.212 | 45.430 | 1646.462 | 565.264 | 2211.726 (le_runs) | 1643.291 | 550.408 | 2193.699 | 1683.748 | 554.727 | 2238.475 | 475.000 | 63.000 |
| h05 | 9.137 | 1.164 | 10.314 | 81.059 | 928.382 | 1009.441 (le) | 5136.249 | 1479.485 | 6615.734 | 72.995 | 1059.461 | 1132.456 | 523.000 | 56.000 |
| h06 | 0.002 | 13.439 | 13.441 | 0.000 | 535.059 | 535.059 (le) | 0.000 | 532.734 | 532.734 | 0.000 | 533.836 | 533.836 | 403.000 | 39.000 |
| h07 | 13.098 | 1.968 | 15.048 | 172.658 | 420.560 | 593.218 (le) | 4716.723 | 1692.505 | 6409.228 | 160.484 | 418.738 | 579.222 | 604.000 | 61.000 |
| h07_block | 13.170 | 1.911 | 15.084 | 175.441 | 706.900 | 882.341 (le) | 4973.764 | 1741.846 | 6715.610 | 152.279 | 1497.246 | 1649.525 | 605.000 | 61.000 |
| h08 | 24.904 | 2.766 | 27.662 | 99.327 | 4190.165 | 4289.492 (le) | — | — | >100 s timeout | 132.676 | 616.426 | 749.102 | 838.000 | 80.000 |
| h09 | 40.861 | 3.607 | 44.563 | — | — | >100 s timeout | — | — | >100 s timeout | — | — | >100 s timeout | 2643.000 | 250.000 |
| h10 | 11.808 | 46.979 | 58.787 | 1000.914 | 2886.995 | 3887.909 (le_runs) | 1966.679 | 3108.271 | 5074.950 | 1965.405 | 3114.413 | 5079.818 | 1658.000 | 207.000 |
| h11 | 2.336 | 41.860 | 44.196 | 0.048 | 156.610 | 156.658 (le_runs) | 715.580 | 249.265 | 964.845 | 0.114 | 625.529 | 625.643 | 100.000 | 15.000 |
| h12 | 6.541 | 21.362 | 27.904 | 2249.878 | 914.660 | 3164.538 (le_runs) | 2252.839 | 936.069 | 3188.908 | 2259.324 | 927.198 | 3186.522 | 544.000 | 55.000 |
| h13 | 90.545 | 24.698 | 115.215 | 4116.506 | 5831.543 | 9948.049 (le) | 4157.649 | 5891.410 | 10049.059 | 4065.632 | 5849.338 | 9914.970 | 3484.000 | 286.000 |
| h14 | CPU-only | CPU-only | CPU-only | 240.159 | 900.599 | 1140.758 (le_runs) | 241.916 | 887.772 | 1129.688 | 243.198 | 898.822 | 1142.020 | 583.000 | 60.000 |
| h15 | 1.709 | 19.257 | 20.966 | 3.260 | 924.013 | 927.273 (le_runs) | 4.035 | 910.366 | 914.401 | 3.216 | 923.515 | 926.731 | 591.000 | 64.000 |
| h16 | 9.561 | 0.460 | 10.023 | 439.635 | 536.980 | 976.615 (le_runs) | 436.414 | 373.434 | 809.848 | 442.066 | 391.892 | 833.958 | 156.000 | 24.000 |
| h18 | 8.594 | 166.324 | 174.924 | 2794.608 | 23313.767 | 26108.375 (le) | 2751.547 | 22937.331 | 25688.878 | 2725.572 | 22927.674 | 25653.246 | 9221.000 | 746.000 |
| h19 | 0.002 | 24.373 | 24.375 | 179.105 | 898.733 | 1077.838 (le_runs) | 179.939 | 842.718 | 1022.657 | 181.271 | 832.290 | 1013.561 | 911.000 | 74.000 |
| h20 | 8.493 | 90.159 | 98.710 | 83.246 | 6002.543 | 6085.789 (le) | 1253.098 | 5679.675 | 6932.773 | 44.358 | 5876.727 | 5921.085 | 1439.000 | 128.000 |
| h21_positive | 19.950 | 20.200 | 40.158 | 509.424 | 1436.558 | 1945.982 (le_runs) | — | — | >100 s timeout | 673.218 | 2638.130 | 3311.348 | 1509.000 | 136.000 |
| h22 | 6.223 | 0.191 | 6.414 | 634.356 | 168.101 | 802.457 (le) | 627.281 | 162.779 | 790.060 | 643.993 | 164.743 | 808.736 | 463.000 | 62.000 |
| d19 | 18.631 | 0.708 | 19.341 | 170.908 | 238.883 | 409.791 (le_runs) | 5525.091 | 886.363 | 6411.454 | 173.044 | 252.049 | 425.093 | 420.000 | 45.000 |
| d27 | 5.315 | 14.029 | 19.343 | 170.077 | 936.197 | 1106.274 (le) | 159.967 | 410.910 | 570.877 | 159.253 | 902.955 | 1062.208 | 774.000 | 68.000 |
| d72_inner | 8.078 | 17.540 | 25.623 | 1506.139 | 1877.459 | 3383.598 (le_runs) | 4327.880 | 1564.585 | 5892.465 | 1903.303 | 2660.757 | 4564.060 | 1507.000 | 159.000 |
| d85 | 17.782 | 9.149 | 26.935 | 179.014 | 403.227 | 582.241 (le_runs) | — | — | >100 s timeout | 173.567 | 339.132 | 512.699 | 519.000 | 57.000 |

Coverage: 108 configurations attempted across 27 queries; 88 complete, 9 plain_only_not_selected, 11 timeout; 1 configurations used three runs in at least one completed mode.

### SF10 uninstrumented CPU totals

These whole-query medians include index construction and determine the primary CPU speedups.

| Query | CPU-LE total | CPU-advised total | CPU-default total |
|---|---:|---:|---:|
| h01 | 42990.346 | 42655.486 | 43062.649 |
| h02 | 1078.245 | 1973.574 | 1329.774 |
| h03 | 503.634 | 6193.515 | 681.095 |
| h04 | 5727.454 | 835.487 | 5755.165 |
| h04_not_exists | 2171.690 | 2178.073 | 2242.424 |
| h05 | 750.828 | 5811.544 | 679.672 |
| h06 | 442.199 | 443.559 | 441.270 |
| h07 | 544.212 | 5314.043 | 524.141 |
| h07_block | 689.782 | 5541.788 | 783.912 |
| h08 | 1270.142 | 18587.764 | 622.492 |
| h09 | >100 s timeout | >100 s timeout | >100 s timeout |
| h10 | 3341.593 | 4171.500 | 4139.860 |
| h11 | 146.347 | 697.315 | 209.818 |
| h12 | 2798.779 | 2773.156 | 2774.910 |
| h13 | 9687.810 | 10138.350 | 9670.642 |
| h14 | 994.480 | 982.689 | 994.722 |
| h15 | 849.570 | 853.291 | 856.435 |
| h16 | 810.836 | 737.909 | 771.604 |
| h18 | 23709.349 | 23643.970 | 23540.553 |
| h19 | 925.780 | 1029.923 | 1105.112 |
| h20 | 5690.554 | 6641.867 | 5489.040 |
| h21_positive | 1878.550 | 8926.413 | 2501.338 |
| h22 | 1264.351 | 1269.022 | 1257.905 |
| d19 | 353.958 | 5541.231 | 366.826 |
| d27 | 871.748 | 558.667 | 883.822 |
| d72_inner | 2825.684 | 5354.696 | 3752.724 |
| d85 | 510.415 | 8029.475 | 445.691 |

## Geometric-mean speedups

Speedup = comparator total / GPU total; **greater than 1× favors GPU**. The geometric mean is `exp(mean(log(comparator_ms / gpu_ms)))`, equally weighting queries. The denominator counts common exact GPU queries with a completed numeric comparator measurement. h14 is excluded, as are unavailable/timed-out measurements; a timeout's elapsed wall time is never included. Different denominator sets are not directly comparable aggregate rankings. **Uninstrumented CPU comparisons are primary**; profile comparisons only describe the phase-table measurements.

| Scale | Comparator | Common exact queries (n) | Geometric-mean GPU speedup |
|---|---:|---:|---:|
| SF1 | duckdb_t1_ms | 26 | 10.314× |
| SF1 | duckdb_t24_ms | 26 | 2.768× |
| SF1 | cpu_le_uninstrumented_total_ms | 25 | 12.999× |
| SF1 | cpu_advised_uninstrumented_total_ms | 26 | 33.067× |
| SF1 | cpu_default_uninstrumented_total_ms | 25 | 14.881× |
| SF1 | cpu_le_profile_total_ms | 25 | 17.863× |
| SF1 | cpu_advised_profile_total_ms | 26 | 38.739× |
| SF1 | cpu_default_profile_total_ms | 25 | 20.061× |
| SF10 | duckdb_t1_ms | 26 | 23.686× |
| SF10 | duckdb_t24_ms | 26 | 2.492× |
| SF10 | cpu_le_uninstrumented_total_ms | 25 | 47.429× |
| SF10 | cpu_advised_uninstrumented_total_ms | 25 | 103.589× |
| SF10 | cpu_default_uninstrumented_total_ms | 25 | 49.287× |
| SF10 | cpu_le_profile_total_ms | 24 | 53.675× |
| SF10 | cpu_advised_profile_total_ms | 21 | 96.064× |
| SF10 | cpu_default_profile_total_ms | 24 | 58.023× |

## Queries where GPU is slower

- SF1, duckdb_t1_ms: h02, h11.
- SF1, duckdb_t24_ms: h01, h02, h06, h11, h15.
- SF1, cpu_le_uninstrumented_total_ms: h11.
- SF1, cpu_advised_uninstrumented_total_ms: none among completed measurements.
- SF1, cpu_default_uninstrumented_total_ms: h11.
- SF10, duckdb_t1_ms: h02.
- SF10, duckdb_t24_ms: h01, h02, h11.
- SF10, cpu_le_uninstrumented_total_ms: none among completed measurements.
- SF10, cpu_advised_uninstrumented_total_ms: none among completed measurements.
- SF10, cpu_default_uninstrumented_total_ms: none among completed measurements.

h02 includes host fallback/intermediate publication; h11 and h01 spend substantial GPU time in grouping/reduction/publication. These phase allocations are not hardware-counter diagnoses: occupancy, bandwidth and cache misses are not inferred. Differences near DuckDB's 1 ms timer resolution should not be treated as precise rankings.

Machine-readable outputs: [comparison TSV](performance_comparison.tsv), [speedups JSON](performance_speedups.json), [CPU SF1](performance_cpu_sf1.json), [CPU SF10](performance_cpu_sf10.json), [GPU summary](performance_gpu_summary.json). Raw CPU measurement artifacts are under `performance_cpu_raw/sf1` and `performance_cpu_raw/sf10`.
