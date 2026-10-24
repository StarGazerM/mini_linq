# CQ TPC-H-shaped performance comparison

## Scope and methodology

Measurements were collected on 2026-10-24 from `5e03047784c357dcdbbef3bd0f253879f964ee85`, with the dictionary-literal loader correction committed alongside this report. These are the **27 authored CQ-shaped catalog queries**, not a claim of full standard TPC-H coverage. GPU name `hXX`/`dXX` maps to CPU/SQL catalog name `retail_hXX`/`retail_dXX`. GPU and DuckDB use **one warmup and five measured executions**. CPU normally uses five, subject to the explicitly authorized runtime cap below. Times below are milliseconds; lower is better.

Hardware: **AMD Ryzen Threadripper PRO 5945WX, 12 cores / 24 hardware threads**, approximately **125.77 GiB RAM**, and **NVIDIA RTX 6000 Ada Generation, 49,140 MiB reported device memory**, NVIDIA driver **580.178.04**. GPU compilation targeted `sm_89` with CUDA 12.9 from the NVIDIA HPC SDK 25.5. See [the hardware snapshot](performance_hardware.json) for the raw inventory. Timed GPU, CPU, and DuckDB windows were serialized; CPU builds were paused during DuckDB measurements and finished before GPU measurements.

### A: CQ GPU, statistics-derived advised plans

The suite runs `--mode advised`: the checked statistics-derived order, reduction, storage, and launch advice already present in the repository. “Advised” is not a proof of global optimality and no plans were changed for this comparison. Every numerical GPU row passed exact answer comparison for its warmup and all five executions; SF1 also checks the equivalent sequential physical program.

- **Build** is the suite's `build_ms`: CUDA-event elapsed time for query-required structure construction, selection/index preparation and associated scheduled work. In staged queries, the suite sums stage measurements. Its existing accounting also charges complete host-graded stages and nullable intermediate host publication to build. Thus this label is **not exclusively GPU index-construction time**.
- **Query** is the median of each sample's `kernels_ms + dedup_ms`. Kernels includes scheduled query traversal/expansion; dedup includes canonicalization, grouping/reduction, and device output publication. In grouped queries, substantial query computation is attributed to dedup, so it must not be read as only a final DISTINCT operation.
- **Total** is the median of `total_ms`. Build, query, and total are independent medians and can differ slightly from their arithmetic sum. Raw samples retain full precision.
- Input file loading, input upload/compression, module initialization, final answer download/decoding, and answer comparison are excluded. Query-required intermediate publication is included, unlike final answer retrieval. These are resident-input execution measurements, not end-to-end application latency.
- **Hybrid caveat:** h02 stage 1 and h15 stage 2 run on the host after checked device rejection. Other staged queries can publish nullable intermediates to the host. Their measured work is retained, not hidden, but these rows must not be described as entirely device execution.

Timing boundaries are in [`benchmark.rs`](../tpch/src/benchmark.rs), [`build_stages.rs`](../tpch/build_stages.rs), [`build.rs`](../tpch/build.rs), and [`emit.rs`](../sequence/core/src/emit.rs).

All GPU measurements are fresh from this session. The initial complete-suite process exhausted device memory at SF10 d27/d72_inner/d85 after earlier queries had completed. A fresh process executing those same three queries, without changing plans, completed all five samples exactly. The report uses these recovered measurements, preserves both invocations, and does not treat the initial allocation failures as timings. Process-lifetime retention is a plausible explanation, not a proved leak diagnosis.

**Historical unsupported GPU queries:** the measurements above originally rejected h14 and h19 at both scales. **h14 removed for now: prefix predicate inside the aggregate; needs order-preserving interning.** Its CPU workload remains available. H19 now uses author-controlled `u32` interning and statistics-derived advised execution; the validation and new measurements below supersede its historical unsupported status. The original timing artifacts retain their original rejected entries.

### B/C: CQ CPU LE and default

The generated worker's `query_median_ms`/`median_ms` times the complete `query::run`, **including query-required structure/index construction**. It excludes input loading and excludes canonicalization, answer serialization, and exact comparison. The separate `index_builds` counter counts operations, not milliseconds. It cannot by itself split the measured total.

The phase table uses the existing operator-profile measurement to obtain a genuine measured split: **build** is the exclusive build time, **query** is the paired profile total minus that build time, and **total** is that same profile execution's total. The selected execution is the median-total profile execution. Exclusive build accounting avoids counting the upstream input stream twice. **These CPU phase totals are instrumented**, not the uninstrumented bench median: profiling materializes build inputs before insertion and can change runtime substantially. No profile time is subtracted from an unrelated uninstrumented total.

The split uses `BUILD_EXCLUSIVE_TOTAL` from the profile tree: for each operator whose kind ends in `_BUILD`, elapsed construction time minus its separately timed input production, using saturating subtraction. The tree prints this total to 0.001 ms, so the residual has corresponding rounding uncertainty. Query-local preprocessing remains in total/query unless covered by such an operator; one-time Parquet loading and load-time zone preparation are excluded. This is an execution-phase accounting comparison, not identical internal operator boundaries across engines.

The CPU search includes authored configurations, including `no-reduce`, subject to the later-authorized stopping rule: cap a configuration worker at 600 seconds, use three rather than five measured runs when an execution exceeds 60 seconds, and skip more expensive configurations in a family once a cheaper one is already over 10× slower than GPU. C is exactly `default`. B is the fastest successfully measured authored **`le` or `le_runs`** configuration, selected by its **uninstrumented** median total, not by the profile total. These choose `Planner::Le`; `transfer`, `candidate`, `advised`, and `hypergraph_zone` do not become LE merely because they share downstream Sequence machinery. Queries without an authored LE configuration are marked “no LE config”. “Best” is limited to completed measurements under this stopping rule, not an exhaustive search over all join orders. The uninstrumented medians are reported separately and are the primary basis for CPU speedups; profile-based speedups are explicitly distinguished.

CPU timeouts/skips are not correctness passes or numeric query timings. In particular, “worker >600 s” denotes exhaustion of a configuration measurement budget, including its loading, warmup, repeated executions and untimed comparisons; it is **not** a lower bound of 600 seconds on an individual query execution. Numeric CPU timings are admitted only when the worker's exact correctness check passes. Censored or unavailable values are excluded from geometric means with the denominator stated.

CPU timing/correctness boundaries are in [`worker.rs`](../../../features/workloads/xtask/src/tpch/worker.rs). CPU workers are sequential; DuckDB is shown with explicit t1 and t24 settings. CPU SF10 is unavailable: the CPU benchmark accepts only SF0.1 and SF1.

### D: DuckDB

DuckDB **v1.4.4 (Andium), `6ddac802ff`**, with separate `SET threads=1` and `SET threads=24` processes. Fresh CLI `.timer` measurements execute the complete authored SQL in **`.mode trash`**, with one warmup followed by five samples. This avoids answer CSV formatting/output and comparison; intrinsic engine result materialization/consumption remains part of execution. CLI timer resolution is 1 ms.

Database generation/export, process startup/opening, and setting the thread count are outside the timer. Query-required scans, reads/decompression, hash-table builds, aggregation and DISTINCT are inside. **DuckDB has no separately measured external index-build phase**: its displayed time already includes its query-time hash/index construction. Historical CSV-output timers embedded by the GPU suite are not used for either table or speedups; fresh no-output timings are used instead.

Reproduction commands (from the repository root unless noted):

```sh
# In experiments/gpu:
CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9 CARGO_BUILD_JOBS=1 \
  cargo oxide build --arch sm_89 -- --release -p cq-gpu-tpch
./target/release/cq-gpu-tpch compare/performance_gpu.json --mode advised
./target/release/cq-gpu-tpch compare/performance_gpu_sf10_remaining.json \
  --mode advised --scale sf10 --query d27 --query d72_inner --query d85
# At repository root:
python3 experiments/gpu/compare/performance_duck.py
```

The loader fix rewrites dictionary string literals in equality/disjunction/membership guards and nested conditional/arithmetic expressions, including aggregate heads normalized into Bind later. Opaque or ordered string uses retain String storage. The synthetic-manifest regression covers conditional bindings, aggregate heads, reversed inequality, and unsafe string operations. The authorized release build and targeted synthetic-manifest test passed before measurement.

## SF1: phase comparison

**CPU columns are instrumented profile phases; GPU/DuckDB are ordinary
benchmark timings.** Use the uninstrumented CPU table and primary speedups
below for whole-query comparison. All 89 completed CPU measurements have
correctness **ok**, five plain samples and five profile samples; no completed
row needed the adaptive three-run option. The table selects 26 defaults
and 14 best-observed LE configurations.

| Query | GPU build | GPU query | GPU total | CPU-LE build | CPU-LE query | CPU-LE total (config) | CPU-default build | CPU-default query | CPU-default total | DuckDB t1 | DuckDB t24 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| h01 | 0.002 | 66.294 | 66.295 | no LE config | no LE config | no LE config | 0.000 | 2823.732 | 2823.732 | 819.000 | 63.000 |
| h02 | 8.125 | 18.525 | 26.689 | no LE config | no LE config | no LE config | 18.958 | 110.268 | 129.226 | 12.000 | 9.000 |
| h03 | 0.044 | 1.922 | 1.965 | 7.332 | 32.382 | 39.714 (le_runs) | 17.085 | 37.638 | 54.723 | 37.000 | 11.000 |
| h04 | 0.514 | 0.652 | 1.164 | 45.775 | 40.958 | 86.733 (le) | 46.209 | 41.427 | 87.636 | 44.000 | 10.000 |
| h04_not_exists | 3.735 | 0.240 | 3.971 | 104.021 | 57.818 | 161.839 (le_runs) | 105.759 | 58.262 | 164.021 | 46.000 | 11.000 |
| h05 | 1.542 | 0.469 | 2.011 | 2.445 | 56.519 | 58.964 (le_runs) | 5.541 | 83.285 | 88.826 | 47.000 | 12.000 |
| h06 | 0.002 | 7.416 | 7.417 | no LE config | no LE config | no LE config | 0.000 | 36.984 | 36.984 | 36.000 | 7.000 |
| h07 | 1.665 | 1.172 | 2.838 | 2.976 | 30.259 | 33.235 (le_runs) | 13.233 | 33.063 | 46.296 | 48.000 | 14.000 |
| h07_block | 1.803 | 1.084 | 2.883 | 3.689 | 37.702 | 41.391 (le_runs) | 15.199 | 143.843 | 159.042 | 48.000 | 14.000 |
| h08 | 2.663 | 2.164 | 4.835 | 6.511 | 340.951 | 347.462 (le_runs) | 9.229 | 50.425 | 59.654 | 40.000 | 13.000 |
| h09 | 4.064 | 1.741 | 5.805 | — | — | worker >600 s (le/le_runs) | — | — | worker >600 s (default) | 166.000 | 40.000 |
| h10 | 1.673 | 12.365 | 14.038 | no LE config | no LE config | no LE config | 167.441 | 231.001 | 398.442 | 148.000 | 44.000 |
| h11 | 0.616 | 18.804 | 19.419 | no LE config | no LE config | no LE config | 0.010 | 62.908 | 62.918 | 11.000 | 5.000 |
| h12 | 0.631 | 8.576 | 9.207 | no LE config | no LE config | no LE config | 139.257 | 83.520 | 222.777 | 60.000 | 11.000 |
| h13 | 6.457 | 9.503 | 15.914 | no LE config | no LE config | no LE config | 123.323 | 358.763 | 482.086 | 223.000 | 38.000 |
| h14 | removed | removed | removed | no LE config | no LE config | no LE config | 5.383 | 68.725 | 74.108 | 44.000 | 18.000 |
| h15 | 0.261 | 14.633 | 14.895 | no LE config | no LE config | no LE config | 0.260 | 63.739 | 63.999 | 48.000 | 11.000 |
| h16 | 1.353 | 0.251 | 1.604 | 19.718 | 28.939 | 48.657 (le) | 16.821 | 22.820 | 39.641 | 9.000 | 7.000 |
| h18 | 1.086 | 22.999 | 24.083 | no LE config | no LE config | no LE config | 155.259 | 1388.154 | 1543.413 | 722.000 | 81.000 |
| h19 | see addendum | see addendum | see addendum | no LE config | no LE config | no LE config | 17.758 | 122.990 | 140.748 | 93.000 | 14.000 |
| h20 | 1.609 | 14.986 | 16.596 | no LE config | no LE config | no LE config | 2.940 | 365.410 | 368.350 | 99.000 | 22.000 |
| h21_positive | 1.880 | 2.493 | 4.369 | 34.787 | 143.442 | 178.229 (le) | 45.978 | 249.999 | 295.977 | 111.000 | 20.000 |
| h22 | 0.868 | 0.148 | 1.015 | 18.059 | 3.834 | 21.893 (le) | 18.146 | 4.032 | 22.178 | 23.000 | 15.000 |
| d19 | 2.330 | 0.550 | 2.878 | 7.813 | 18.346 | 26.159 (le) | 7.993 | 20.515 | 28.508 | 35.000 | 11.000 |
| d27 | 0.793 | 1.812 | 2.610 | 1.386 | 28.267 | 29.653 (le_runs) | 1.800 | 32.037 | 33.837 | 42.000 | 10.000 |
| d72_inner | 1.331 | 2.988 | 4.319 | 69.768 | 127.739 | 197.507 (le) | 78.324 | 177.522 | 255.846 | 116.000 | 70.000 |
| d85 | 2.106 | 1.465 | 3.571 | 8.613 | 28.940 | 37.553 (le) | 9.372 | 25.350 | 34.722 | 43.000 | 13.000 |

Build scopes observed: `INDEX_BUILD`, `JOIN_FILTER_BUILD`, `KEYSET_BUILD`,
`ROWS_BUILD`. Zero means no recorded time in these explicit build scopes,
not absence of allocation, grouping state, or hashing anywhere in the query.
Twelve default-only queries (h01, h02, h06, h10, h11, h12, h13, h14, h15,
h18, h19, h20) passed exact answers, operator-graph and top-level clock
checks, but primitive `distinct_insertions` disagreed with profiled
set-insertion accounting. Their JSON records
`primitive_build_counts_verified=false`; independent insertion-count
verification is not claimed. This limits profile interpretation, without
changing the exact-answer result.

### SF1 uninstrumented CPU totals

These include index construction and determine the primary CPU speedups.

| Query | CPU-LE total | CPU-default total |
|---|---:|---:|
| h01 | no LE config | 2656.013 |
| h02 | no LE config | 65.371 |
| h03 | 30.914 | 44.045 |
| h04 | 49.530 | 49.732 |
| h04_not_exists | 169.839 | 173.460 |
| h05 | 35.699 | 43.504 |
| h06 | no LE config | 24.567 |
| h07 | 30.188 | 39.758 |
| h07_block | 33.072 | 66.872 |
| h08 | 60.726 | 36.907 |
| h09 | worker >600 s | worker >600 s |
| h10 | no LE config | 326.971 |
| h11 | no LE config | 17.022 |
| h12 | no LE config | 177.882 |
| h13 | no LE config | 459.702 |
| h14 | no LE config | 67.976 |
| h15 | no LE config | 37.090 |
| h16 | 43.980 | 41.194 |
| h18 | no LE config | 1350.122 |
| h19 | no LE config | 109.992 |
| h20 | no LE config | 320.665 |
| h21_positive | 144.744 | 202.969 |
| h22 | 23.238 | 23.717 |
| d19 | 20.762 | 21.477 |
| d27 | 16.333 | 17.907 |
| d72_inner | 156.919 | 197.245 |
| d85 | 28.950 | 28.892 |

**Coverage:** 105 configurations inventoried; 89 completed, six h09 timeouts
(`default`, `no-reduce`, `transfer`, `le`, `le_runs`, `generic_smallest`),
and ten authorized family skips. Skipped: h16/h22/h21_positive:
`no-reduce`, `le_runs`; d72_inner: `no-reduce`, `le_runs`, `advised`;
d85: `hypergraph_zone`. The h16, h22, h21_positive and d72_inner LE winners
are best observed, not fully searched LE winners. h09 default ran about
44 minutes before the cap was authorized and applied; no numeric result
is inferred from that elapsed worker time.

## SF10: GPU versus DuckDB

CPU SF10 is not supported by the CPU benchmark and is not extrapolated.
Every numerical GPU row is exact; query = kernels + dedup as defined above.

| Query | GPU build | GPU query | GPU total | DuckDB t1 | DuckDB t24 |
|---|---:|---:|---:|---:|---:|
| h01 | 0.002 | 591.959 | 591.961 | 9431.000 | 531.000 |
| h02 | 217.414 | 113.961 | 331.269 | 137.000 | 33.000 |
| h03 | 0.089 | 17.449 | 17.537 | 566.000 | 57.000 |
| h04 | 5.014 | 5.403 | 10.420 | 459.000 | 50.000 |
| h04_not_exists | 44.211 | 1.212 | 45.430 | 475.000 | 63.000 |
| h05 | 9.137 | 1.164 | 10.314 | 523.000 | 56.000 |
| h06 | 0.002 | 13.439 | 13.441 | 403.000 | 39.000 |
| h07 | 13.098 | 1.968 | 15.048 | 604.000 | 61.000 |
| h07_block | 13.170 | 1.911 | 15.084 | 605.000 | 61.000 |
| h08 | 24.904 | 2.766 | 27.662 | 838.000 | 80.000 |
| h09 | 40.861 | 3.607 | 44.563 | 2643.000 | 250.000 |
| h10 | 11.808 | 46.979 | 58.787 | 1658.000 | 207.000 |
| h11 | 2.336 | 41.860 | 44.196 | 100.000 | 15.000 |
| h12 | 6.541 | 21.362 | 27.904 | 544.000 | 55.000 |
| h13 | 90.545 | 24.698 | 115.215 | 3484.000 | 286.000 |
| h14 | removed | removed | removed | 583.000 | 60.000 |
| h15 | 1.709 | 19.257 | 20.966 | 591.000 | 64.000 |
| h16 | 9.561 | 0.460 | 10.023 | 156.000 | 24.000 |
| h18 | 8.594 | 166.324 | 174.924 | 9221.000 | 746.000 |
| h19 | see addendum | see addendum | see addendum | 911.000 | 74.000 |
| h20 | 8.493 | 90.159 | 98.710 | 1439.000 | 128.000 |
| h21_positive | 19.950 | 20.200 | 40.158 | 1509.000 | 136.000 |
| h22 | 6.223 | 0.191 | 6.414 | 463.000 | 62.000 |
| d19 | 18.631 | 0.708 | 19.341 | 420.000 | 45.000 |
| d27 | 5.315 | 14.029 | 19.343 | 774.000 | 68.000 |
| d72_inner | 8.078 | 17.540 | 25.623 | 1507.000 | 159.000 |
| d85 | 17.782 | 9.149 | 26.935 | 519.000 | 57.000 |

## Where GPU is slower than DuckDB

- **h02, both scales, both thread counts:** SF1 total 26.689 ms versus DuckDB 12/9 ms; SF10 total 331.269 ms versus 137/33 ms. The hybrid plan's host fallback and intermediate publication are charged to build: 217.414 ms at SF10, approximately 66% of total; kernels add 97.023 ms and dedup 16.863 ms. This is not a purely GPU index-build bottleneck.
- **h11, SF1 versus t1/t24 and SF10 versus t24:** SF1 19.419 ms versus 11/5 ms; SF10 44.196 ms versus t24's 15 ms. SF1 is dominated by the dedup/group/publication bucket (16.196 ms); SF10 has both substantial kernels (23.240 ms) and dedup (18.627 ms), rather than large build cost.
- **h01 versus t24 at both scales:** 66.295 versus 63 ms at SF1; 591.961 versus 531 ms at SF10. Almost all suite time is in the grouping/reduction/output bucket labeled dedup (66.292/591.957 ms), not index building.
- **h15 versus t24 at SF1 only:** 14.895 versus 11 ms; 14.630 ms is attributed to dedup/grouping/publication. The staged plan also has a host-graded terminal stage. At SF10 it is faster than both DuckDB settings.
- **h06 versus t24 at SF1 only:** 7.417 versus 7 ms, with 7.414 ms in dedup/grouping. This small difference is below the 1 ms resolution of the DuckDB CLI timer and should not be treated as a precise ranking.

The phase breakdown establishes where elapsed time is charged, not a hardware-counter diagnosis. No claims about occupancy, bandwidth saturation, or cache misses are inferred from these timings.

Against uninstrumented CPU, **h11 is the only measured query where GPU is
slower**: 19.419 ms versus default's 17.022 ms (GPU 1.14× slower). Its GPU
dedup/group/publication bucket alone is 16.196 ms. No measured LE row is
faster than GPU; no claim is made about censored h09 CPU configurations.
The larger profile-based speedups below demonstrate why profiling overhead
must not be confused with ordinary CPU runtime.

## Speedup definition

Speedup is comparator total / GPU total, so **greater than 1× favors GPU**.
The geometric mean is `exp(mean(log(comparator_ms / gpu_ms)))`, giving each
query equal weight rather than weighting by runtime. Unsupported GPU rows
are excluded, and CPU comparisons additionally exclude absent LE configs,
timeouts, skips, and failed/unavailable measurements. Different coverage
sets must not be interpreted as directly comparable aggregate rankings.

| Scale | Comparator | Common exact GPU queries | Geometric-mean GPU speedup |
|---|---|---:|---:|
| SF1 | DuckDB t1 | 25 | 10.325× |
| SF1 | DuckDB t24 | 25 | 2.835× |
| SF10 | DuckDB t1 | 25 | 23.258× |
| SF10 | DuckDB t24 | 25 | 2.472× |
| SF1 | CPU LE, uninstrumented (primary) | 14 | 17.302× |
| SF1 | CPU default, uninstrumented (primary) | 24 | 14.781× |
| SF1 | CPU LE, instrumented phase-table total | 14 | 24.546× |
| SF1 | CPU default, instrumented phase-table total | 24 | 20.908× |

## Raw data and provenance

- [CPU summary JSON](performance_cpu_sf1.json), [successful timing TSV](performance_cpu_sf1.tsv) (89 rows, every correctness value `ok`), [complete status TSV](performance_cpu_sf1_status.tsv), [raw CPU metrics](performance_cpu_raw/), [CPU collector](performance_cpu_collect.py), and [CPU environment](performance_cpu_environment.json). The collector records the mechanically adapted generated harness used for the authorized timeout/run-count rule; authored queries/compiler plans are unchanged.
- [Combined comparison TSV](performance_comparison.tsv) retains full-precision phase/total values and explicit unavailable statuses. [Speedup JSON](performance_speedups.json) records the formula, exact query membership, denominator and full-precision geometric mean for every comparison.

- [Full fresh GPU suite invocation](performance_gpu.json): five raw samples per completed query, resolved plans/stages, exactness, unsupported-query errors, and the initial three SF10 allocation failures. Embedded historical DuckDB values are **not used** in this comparison.
- [Fresh-process SF10 recovery](performance_gpu_sf10_remaining.json): exact d27, d72_inner, d85, five samples each.
- [Derived GPU phase summary](performance_gpu_summary.json): explicit raw-file selection and per-sample aggregation rules.
- [Fresh DuckDB SF1](performance_duck_trash_sf1.json) and [SF10](performance_duck_trash_sf10.json): all 27 SQL statements, both thread counts, warmup, five samples, medians, and raw CLI output.
- [DuckDB reproduction script](performance_duck.py), [hardware snapshot](performance_hardware.json).
- Historical [SF1](performance_duck_sf1.json) and [SF10](performance_duck_sf10.json) DuckDB files preserve SQL/provenance and CSV-output timings used as inputs to the no-output collector. Their historical elapsed times are excluded from all reported tables and speedups.

No older GPU timing artifact supplies any number in this report.

## Author-interned H19 addendum

H19 now declares shipping mode, shipping instruction, brand and container as
`u32`, with author-side `cq_load::StringDictionary` preparation. Its setup
`intern(table, column, literal)` calls resolve against the same per-column
sorted-distinct, zero-based dictionaries on CPU and GPU. Missing literals use
`u32::MAX`, outside the source-code domain. The exporter and existing SF1/SF10
files already use this ordering for all four columns: no data re-export or
exporter change was needed. The other queries' automatic String equality
encoding remains unchanged. See [the H19 statistics derivation](tpch_statistics.md).

**h14 removed for now: prefix predicate inside the aggregate; needs order-preserving interning.**
The CPU h14 catalog entry and workload are unchanged.

The [new full-suite artifact](gpu_tpch_intern.results.json) comes from one
unfiltered process:

```sh
./target/release/cq-gpu-tpch compare/gpu_tpch_intern.results.json --mode advised
```

All **26 currently supported queries at both scales (52/52 results)** were
exact, with no rejections or runtime failures. The inventory is 26, rather than
the requested 24: the starting tree had 27 entries, and only h14 was removed.
SF1 also passed equivalent sequential-program conformance. The timings use one
warmup and five measured executions; query is the median of each sample's
`kernels_ms + dedup_ms`, including aggregate reduction. The DuckDB figures below
are the original report's t1/t24 values, not the different historical values
embedded in the GPU manifest.

| H19 scale | GPU total ms | GPU build ms | GPU query ms | DuckDB t1 ms | DuckDB t24 ms |
|---|---:|---:|---:|---:|---:|
| SF1 | 9.254 | 0.002 | 9.253 | 93 | 14 |
| SF10 | 24.375 | 0.002 | 24.373 | 911 | 74 |

Earlier aggregate speedup summaries and machine-readable comparison artifacts
remain the original historical comparison, excluding its unsupported H19;
this addendum does not retroactively recompute them.

Release GPU library verification (only `cq-gpu-tpch`, not `gpu-h05-compiled`):
`cargo oxide test --arch sm_89 -- --release -p cq-gpu-tpch --lib -- --test-threads=1`
passed **36 tests**, with one pre-existing ignored idle-GPU memory regression.
This includes manifest/CPU dictionary agreement at both scales, retained `u32`
input types, absent-literal handling, H19 exact conformance, and unchanged
automatic dictionary adaptation coverage.

The CPU catalog benchmark
`cargo xtask bench --tpch --sf 1 --runs 3 --query retail_h19`
passed exact comparison against DuckDB for the default configuration (one
answer row), also verifying SF0.1 before measuring SF1. Its new CPU query median
is **50.518 ms**, with **2316.171 ms** input loading/preparation excluded;
see [the preserved benchmark TSV](cpu_h19_intern.tsv). This benchmark uses
DuckDB v1.5.5 and reports its own t1/t24 medians of 166.354/21.394 ms; those
are separate from the report's historical DuckDB baselines in the GPU table.
The first cold CPU command reached its execution deadline while compiling the
generated worker; resuming with cached artifacts completed the exact benchmark.
