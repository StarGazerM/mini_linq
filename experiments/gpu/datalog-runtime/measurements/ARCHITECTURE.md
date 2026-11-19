# Packed-run architecture study

## Measurement protocol and hardware

This study uses the local RTX 6000 Ada Generation, driver 580.178.04, CUDA 13.0 builds and Nsight Compute 2025.2 from `/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/compilers/bin/ncu`. Raw profiles, binaries, logs and replay JSON are outside git in `/data/design-notes/doop-memory/packed-run/arch/`. Every timed process has before/after compute-tenant queries; the replay also queries tenants internally. Profiling times are not substituted for unprofiled benchmark times.

CUDA driver attribute queries measured **100,663,296 bytes (96 MiB) L2**, **142 SMs**, **1536 maximum resident threads/SM**, **102,400 bytes maximum shared memory/SM**, a 384-bit memory bus and 10,001,000 kHz reported memory clock. Thus the 4090's 72 MiB L2 is not this card's cache capacity. [INFERENCE] The bus/clock attributes imply approximately 960 GB/s theoretical DRAM bandwidth; Nsight's own percent-of-peak metric is used below instead of equating logical row bytes with physical traffic.

Nsight attaches successfully to the materialized cubins. Six baseline profiles cover states 137, 112 and 90 with packed and plain experimental storage. `--set full --cache-control none --clock-control none` preserves warm-cache conditions and application clocks. The kernel-id invocation filter `::regex:^(plain|packed|plain_scan|packed_scan|packed_serial_scan|packed_contains)_TID_:^(2|13|24)$` captures the second launch of each workload (after its validated warmup). Each report contains ten kernels: five engine and five experimental. Cache hit rates slightly above 100% are multi-pass measurement noise, not literal probabilities. PC percentages below normalize sampled issue states, including selected; they are not percentages of wall time.

The initial attachment smoke profile used default cache flushing/base clocks; it is deliberately excluded from the warmed comparisons. No source line information is present in these cubins: Nsight reports `No lineinfo available`. SASS-level PC samples are available. Source interpretations below are explicitly inference, not claimed source-line attribution.

An attempted source-correlated build used `cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin --lineinfo -- --release -p gpu-datalog-runtime --bin packed-replay`. It failed in CUDA 13.0 ptxas with repeated `Unresolved label '$L__info_string0' used in function_name attribute.` (also suffixes 1 and 2). Versions: rustc **1.99.1 (58cf8f9c3 2026-10-13)**, cargo-oxide **0.2.1**, ptxas **V13.0.48**, build `cuda_13.0.r13.0/compiler.36260728_0`, under `/opt/nvidia/hpc_sdk/Linux_x86_64/25.9/cuda/13.0`. The full command/error log is `arch/build-local32.log`. No compiler, proc-macro, RCCL or engine code was changed to work around it; source-line claims are consequently withheld.

### NVTX scope, without an alternate instrumentation mechanism

The final profiling builds enable the existing **`cq-gpu-runtime/nvtx`** feature and use `cq_gpu_runtime::nvtx::range(c"static_name")`; normal timing builds leave it off. The existing `profile/profile.sh` wrapper captures CUDA+NVTX, exports SQLite and invokes `analyze.py`. Reads have `read_prefix_sorted`, `read_prefix_shuffled`, `read_prefix_mixed`, `read_range_scan`, `read_membership` ranges containing `state_90`, `state_112`, `state_137` or `state_147` ranges. These enclose only ten warmed measured calls and their completion synchronization, not input uploads or host/oracle downloads. The enclosing configuration is `engine`, `new_plain` or `new_packed`.

Packed admission has static sort, fused-sort/subtract, subtract, seal/adopt, merge, partition, analysis/offsets and pack ranges. **Engine files remain untouched**: replay-level `engine_sort_subtract`, `engine_adopt`, and `engine_merge_publication` surround existing calls. Sort versus set-difference, and engine merge stages, are split by kernel name *within the launch-correlated replay range*, not by invented engine-internal ranges. A fused kernel is reported as fused, never assigned twice to independent phases.

Device attribution joins CUDA activities to the host APIs' correlation IDs and enclosing same-thread NVTX ranges, including kernels that complete after a launch-only range pops. API time is an interval union, not a sum of nested driver/runtime wrappers. Nested ranges are not additive. Nsight-scoped device/API/transfer numbers are labeled separately from synchronized host timers, and enabled-build timings are used only for instrumentation-overhead assessment, never for the optimization decision.

The two full NVTX captures (`nvtx-packed`, `nvtx-plain`) passed every replay oracle and observed no foreign tenant. `architecture-nvtx.json` contains launch-correlated phase and representative-state kernel, API and memcpy attribution. Read ranges contain exactly **1310 launches/workload/configuration**, except scans **1350** (four sparse states × ten extra reduction launches). Every measured read range has **zero memcpy calls/bytes**.

| Workload | E device / synchronized-host ms | P device / synchronized-host ms | C device / synchronized-host ms |
|---|---:|---:|---:|
| Sorted prefix | 11.276 / 22.316 | 11.543 / 22.257 | 15.529 / 26.683 |
| Shuffled prefix | 33.465 / 44.478 | 33.314 / 43.989 | 38.205 / 49.550 |
| Mixed prefix | 41.346 / 52.503 | 42.004 / 52.810 | 47.558 / 58.804 |
| Scan | 52.983 / 64.646 | 53.648 / 64.974 | 83.292 / 95.016 |
| Membership | 13.961 / 25.418 | 14.924 / 25.948 | 21.194 / 32.633 |

These are paired device/host values **inside the profiled enabled build**, not the final benchmark table. Whole NVTX host-range duration is slightly larger than the timer sum (for packed sorted: 29.093 ms versus 26.683 ms) because it also encloses loop, buffer-drop and range instrumentation overhead. Host API interval unions for E/P/C are sorted **17.914/18.081/22.252 ms**, shuffled **40.231/39.944/45.370 ms**, mixed **48.133/48.640/54.456 ms**, scan **59.658/60.149/89.983 ms**, membership **20.676/21.442/27.887 ms**. These overlap GPU execution and must not be added to device time.

| Admission phase | E device / synchronized-host ms | P device / synchronized-host ms | C device / synchronized-host ms |
|---|---:|---:|---:|
| Sort + subtract, including fused | 73.200 / 251.447 | 55.582 / 155.719 | 57.164 / 160.421 |
| Seal/adopt | 0 / 2.171 | 0 / 4.383 | 1.295 / 10.830 |
| Merge/publication | 6.136 / 30.202 | 5.775 / 32.250 | 15.868 / 49.185 |

Inside the engine sort/subtract range, kernel names attribute **47.451 ms / 8082 launches** to radix sort and **9.205 ms / 1516 launches** to set-membership difference. The remaining **16.544 ms / 12,779 launches** are shared compaction/lane preparation; they are reported separately rather than falsely forcing ambiguous kernels into sort or subtract. Engine merge kernels divide into partition **2.341 ms / 1314 launches** and merge **3.795 ms / 1314 launches**. Packed merge's own NVTX ranges attribute partition **2.808 ms / 243 launches**, analysis plus offsets **4.708 ms / 486 launches**, pack **5.011 ms / 243 launches**, and small/plain merge **3.342 ms / 1071 launches**. New-plain merge uses **5.775 ms / 1314 launches** of the plain merge kernel; it has no codec-analysis/pack phase. Packed merge analysis has 243 eight-byte D2H size reads; timed reads have none. All other phase memcpy counts/bytes and API unions are retained in the compact JSON.

### Annotation overhead control

Three interleaved old-binary / annotated-feature-off / annotated-feature-on runs used the same partitioned kernels and no attached profiler. All tenant checks were clean. For packed sorted/shuffled/mixed/scan/membership, feature-off versus old median ratios are **.965/.981/.981/.990/.971**; feature-on versus off **1.000/.999/.999/1.001/.998**. The intervals overlap: packed sorted old **21.736–22.612 ms**, off **21.607–22.233 ms**; enabled **21.697–39.842 ms**, with a large third-run host-timing outlier (the engine also slows in that run). Thus there is no resolvable positive median overhead without an attached profiler, not a claim of precise negative overhead. `architecture-nvtx-overhead.json` stores medians; raw per-process repetitions remain under `arch/overhead-*`.

The disabled executable has **no `cq_nvtx_*` symbols**; the enabled executable contains `cq_nvtx_push`, `cq_nvtx_pop`, and `cq_nvtx_stream`. Together with the existing feature-gated empty inline guard, this confirms that disabled builds make no NVTX calls. Profiling itself is measurably intrusive: packed sorted's profiled synchronized host total is 26.683 ms, versus 21.794 ms enabled but unprofiled median. That difference includes Nsight interception/capture, not just intrinsic push/pop cost. Optimization decisions and final throughput use feature-off executables only. No `PACKED_READ_STAGES` run supplies the new attribution: NVTX replaces host-stage subtraction for phase evidence; the older opt-in device cutoff experiments are distinct kernels, not independent additive timings.

### NVTX-selected hardware-counter confirmation

The existing `profile/ncu.py --nvtx-range read_prefix_shuffled/state_137 --range-instance 2 --top 1 ...` successfully profiles the packed shuffled range from its launch-correlated nsys kernel selection (`arch/nvtx-wrapper`). For full sections, the same NVTX include and occurrence filters are passed to ncu with `--set full --cache-control none --clock-control none`, a matching kernel-name filter and one selected warmed launch. Twenty-four range-scoped profiles cover all five state-137 workloads plus state-112 scan and state-90 scan/membership, across E/P/C. All before/after tenant checks are clean.

| NVTX-selected counter, E / P / C | State 137 sorted | State 137 shuffled | State 112 scan |
|---|---|---|---|
| Device us, main kernel only | 46.08/50.59/73.22 | 295.84/291.26/339.55 | 202.43/227.55/526.98 |
| Occupancy % | 91.1/61.8/61.3 | 98.2/63.9/62.6 | 79.2/48.6/47.2 |
| L1 hit % | 73.3/74.0/79.2 | 33.3/33.3/51.6 | 89.9/92.7/98.4 |
| L2 hit % | 99.9/99.5/99.5 | 100.1/100.0/100.1 | 99.4/98.8/95.8 |
| Long-scoreboard PC % | 33.0/29.9/25.0 | 94.3/92.3/90.9 | 20.3/17.2/18.1 |
| ALU active-cycle % | 88.8/74.7/82.7 | 13.6/12.7/17.9 | 85.9/78.2/76.0 |

These confirm the baseline search classification using explicit state/workload ranges, rather than whole-process kernel ordinals. Partitioning improves state-112 packed occupancy **33.0→47.2%** and main-kernel duration **2288.96→526.98 us**; long-scoreboard falls **33.4→18.1%**, ALU utilization rises **52.0→76.0%**. The remaining scan is more compute/dependency dominated, not DRAM limited (packed DRAM peak **0.046%**). The additional reduction launch is included in nsys and host totals, not hidden inside this main-kernel-only table. `architecture-nvtx-counters.json` retains cache, coalescing, registers, thread instructions, occupancy and all sampled stall reasons. These profiles precede the final reversed-empty-interval guard; the guarded final source has separate exactness and default-build timing runs.

## Why packed is slower at xalan size

All numbers in this section are from this study's baseline profiles, not imported from READ-PATH.md. E/P/C denote engine / new-set plain / new-set packed, respectively. Each P and C value is paired with the engine in its own process.

### State 137: 1,224,859 one-row prefixes

| Counter, E / P / C | Sorted prefixes | Shuffled prefixes | Mixed prefixes | Membership |
|---|---|---|---|---|
| Device microseconds | 45.76 / 50.40 / 74.08 | 295.49 / 290.40 / 340.45 | 362.27 / 353.18 / 416.19 | 20.70 / 20.99 / 29.82 |
| Achieved occupancy % | 91.1 / 61.8 / 61.3 | 97.5 / 65.0 / 63.0 | 98.0 / 64.6 / 63.6 | 56.5 / 56.4 / 55.5 |
| L1 hit % | 73.3 / 74.0 / 79.2 | 33.2 / 33.6 / 51.7 | 33.3 / 33.8 / 52.1 | 37.2 / 38.1 / 52.5 |
| L2 hit % | 99.8 / 99.5 / 99.5 | 100.2 / 100.0 / 99.8 | 100.1 / 100.0 / 100.1 | 99.6 / 99.5 / 99.7 |
| Global-load 32-byte sectors/request | 2.34 / 2.22 / 1.58 | 19.64 / 19.40 / 17.85 | 19.57 / 19.33 / 17.81 | 19.19 / 17.98 / 17.72 |
| Long-scoreboard PC samples % | 31.0 / 31.3 / 24.6 | 94.3 / 92.4 / 91.1 | 94.7 / 92.1 / 90.8 | 90.2 / 90.6 / 89.0 |
| ALU active-cycle utilization % | 88.8 / 74.7 / 82.7 | 13.6 / 12.8 / 18.0 | 13.4 / 12.9 / 18.3 | 11.8 / 11.1 / 14.5 |
| Executed warp instructions, millions | 37.32 / 41.88 / 62.86 | 37.32 / 41.88 / 64.62 | 46.57 / 52.26 / 80.64 | 2.13 / 2.47 / 4.53 |

**Sorted search is not simply memory-bound.** Neighbouring probes share paths, yielding coalescing/broadcast and L1 locality. Packed executes 1.68x the engine's warp instructions with two-thirds of its theoretical warp capacity (64 versus 32 registers/thread, 32 versus 48 theoretical warps/SM). ALU pressure and dependency waits are substantial; decode arithmetic is not free here. Sorted packed PC states include 16.4% math-pipe throttle, 19.4% not-selected and 23.6% fixed-latency wait, versus 20.3/19.5/18.2% engine.

**Shuffled and mixed search are L2-latency/dependency limited, not DRAM-bandwidth limited.** Engine long-scoreboard rises above 94% while ALU falls to 13–14%; sectors/request jump from 2.34 to 19.64. Packed improves L1 hits and sectors/request, but its dependent decode instructions and lower occupancy erase that advantage. The warmed DRAM percentages are only 0.03–0.06% for these searches. Membership behaves similarly: roughly 89–91% long-scoreboard, nearly all L2 hits, twice as many packed instructions and another decode for equality.

The one-row scan is search-dominated: E/P/C device time 314.21/291.65/377.95 us, occupancy 98.0/47.9/41.5%, L1 hit 34.2/37.6/56.9%, and long-scoreboard 93.1/89.7/87.2%. Calling all scans bandwidth-limited would miss this case.

### Sparse fanout and largest-state scans

| Counter, E / P / C | State 112 scan | State 90 scan |
|---|---|---|
| Rows; probes | 1,435,800; 730 | 2,741,586; 29,795 |
| Device microseconds | 426.40 / 454.75 / 2288.96 | 149.50 / 199.07 / 258.56 |
| Achieved occupancy % | 59.6 / 39.9 / 33.0 | 82.5 / 41.2 / 39.0 |
| L1 hit % | 56.1 / 57.3 / 94.2 | 79.7 / 81.0 / 90.8 |
| L2 hit % | 99.4 / 97.8 / 89.1 | 99.7 / 99.3 / 98.6 |
| Global-load sectors/request | 1.70 / 1.67 / 1.12 | 1.09 / 1.09 / 1.03 |
| Long-scoreboard PC % | 40.6 / 32.8 / 33.4 | 22.8 / 21.4 / 20.3 |
| Fixed-latency wait PC % | 24.3 / 23.9 / 27.1 | 21.5 / 31.4 / 32.7 |
| Not-selected PC % | 9.2 / 9.9 / 8.3 | 21.3 / 10.6 / 10.5 |
| Executed warp instructions, millions | 22.96 / 27.45 / 44.31 | 126.59 / 156.54 / 200.03 |
| DRAM percent of peak | .00185 / .00059 / .00919 | .01617 / .00141 / .00092 |

State 112 is primarily **skewed work scheduling plus decode/dependency cost**, not insufficient DRAM bandwidth or poor coalescing. A host count of the real trace found **722,289 rows in one prefix and 285,997 in the next**; one CTA owns each, so even excellent L1 hits do not distribute their work across SMs. State 136 is worse proportionally: 228,399 of 364,594 rows belong to one of only 26 prefixes. State 90's scan instead has enough probes; its engine ALU utilization is 85.8%, packed 68.1%, with more instructions and lower occupancy for packed. Prefix/membership reads of state 90 have only 16–42% occupancy and 43–80% long-scoreboard: too few concurrent probes plus dependent cached loads, rather than DRAM saturation.

Small workloads also incur fixed submission/completion cost. The replay now measures a minimal one-CTA launch through the same output-allocation path, reports per-repeat and per-probe times, explicitly tags batches with at least 30,000 probes, and aggregates those batches separately. The floor-adjusted estimate is supplementary: a one-CTA baseline cannot remove the scheduling cost of a large grid and must not replace raw times.

### Instruction hotspots and attribution limits

Normalized predicated-on **thread instructions** use Nsight's executed-warp count times active/predicated-on threads per executed instruction, divided by probes (or scanned rows). This avoids mistaking inactive cooperative-search lanes for useful row work.

| Baseline workload | Instructions/unit E / P / C | Short-scoreboard PC % E / P / C | Branch-resolving PC % E / P / C |
|---|---|---|---|
| 137 sorted, per probe | 918.6 / 1006.1 / 1525.3 | .16/.64/.40 | 1.48/6.23/3.82 |
| 137 shuffled, per probe | 918.6 / 1006.1 / 1525.2 | .08/.11/.06 | .16/.93/.93 |
| 137 mixed, per probe | 918.7 / 1006.2 / 1525.1 | .05/.12/.09 | .18/.93/.94 |
| 137 membership, per probe | 500.3 / 557.6 / 996.7 | .07/.40/.31 | .98/1.20/1.07 |
| 137 scan, per row | 966.4 / 1053.0 / 1701.4 | .14/.12/.15 | .13/1.32/1.24 |
| 112 scan, per row | 66.7 / 89.9 / 304.9 | .65/1.25/.99 | 3.39/6.33/6.24 |
| 90 scan, per row | 187.9 / 251.8 / 305.0 | .78/1.96/1.86 | 2.04/8.17/7.38 |

In particular, packed state-112 scanning performs **4.57x** the engine's active thread instructions per row. Poor scheduling amplifies that arithmetic/dependency cost; it is not an alternative explanation that makes decoding free.

The state-137 mixed-prefix packed SASS samples concentrate on `SHF.R.U64` immediately consuming payload loads (6,699 and 6,500 long-scoreboard samples at relative instruction sites in the saved profile), then fence comparison `ISETP` sites (3,758 and 3,306). The engine's corresponding comparisons collect 13,833 and 13,212 samples. [INFERENCE] These are the dependent packed extraction at `packed_run.rs::device::read_bits` and binary-search comparison chains, respectively; absolute SASS addresses are process-specific. State-112 packed scan hot sites include an `IMAD.IADD` consuming loaded metadata (431 long-scoreboard samples), a payload `SHF.R.U64` (314), and range-search comparisons (216/215). [INFERENCE] They correspond to block/frame cursor setup and extraction in `read_support.rs::scan_sum` plus the repeated bound search. The raw SASS CSV retains exact addresses and counts. Short-scoreboard is recorded in the compact counter data rather than presumed to be a limiting shared-memory stall.

## Scale study before reader changes

Each entry is **engine / new plain / new packed**, milliseconds per repetition (ten warmed repetitions), all three in the same process. Synthetic states are not claimed to reproduce xalan admission/run fragmentation: they deliberately isolate one sorted run. The source state's complete value distribution repeats, with only the leading-field offset changing. `architecture-scale-base.json` retains all five workloads, same-process ratios, probes/s, rows/s, plain-width-equivalent GB/s and representation-width-equivalent GB/s. The equivalent bandwidths are workload units times logical bytes/unit divided by time, **not physical DRAM traffic**; membership samples do not read the entire dataset.

| Source; rows | Retained MB, E / P / C | Sorted prefix ms | Shuffled prefix ms | Membership ms | Scan ms |
|---|---|---|---|---|---|
| 90; 2^22 | 33.55 / 33.55 / 9.86 | .0119/.0124/.0130 | .0178/.0173/.0192 | .0216/.0226/.0263 | .0635/.0920/.1384 |
| 90; 2^24 | 134.22 / 134.22 / 39.39 | .0174/.0205/.0237 | .1275/.1628/.0853 | .0487/.0498/.0300 | .3101/.5093/.6153 |
| 90; 2^26 | 536.87 / 536.87 / 157.67 | .0729/.0797/.0610 | .7596/.7061/.3826 | .0608/.0608/.0418 | 1.2526/2.1739/2.7136 |
| 90; 2^28 | 2147.48 / 2147.48 / 630.75 | .5480/.5371/.3382 | 3.8101/3.5594/2.1014 | .1043/.1107/.0545 | 6.0071/9.2360/10.8072 |
| 112; 2^22 | 67.11 / 67.11 / 10.06 | .0128/.0118/.0137 | .0153/.0137/.0168 | .0400/.0385/.0299 | 3.3591/3.4393/7.5059 |
| 112; 2^24 | 268.44 / 268.44 / 40.14 | .0135/.0123/.0150 | .0170/.0152/.0192 | .0579/.0546/.0343 | 6.6285/6.6629/7.7374 |
| 112; 2^26 | 1073.74 / 1073.74 / 160.79 | .0149/.0136/.0160 | .0443/.0433/.0302 | .0790/.0861/.0453 | 6.8605/6.9558/13.6956 |
| 112; 2^28 | 4294.97 / 4294.97 / 643.31 | .0218/.0248/.0284 | .1645/.2335/.1619 | .1537/.1504/.0604 | 8.4511/8.8169/15.0801 |

**A crossover exists, but is workload-dependent.** For source 90, shuffled lookup and membership first beat both plain configurations at the measured 2^24 point (134 MB plain exceeds 96 MiB L2; 39 MB packed fits). Sorted prefixes first win at 2^26. At 2^28 packed shuffled and membership cost 0.552x and 0.523x engine, respectively. Source 112 membership already wins at 2^22, shuffled prefixes at 2^26, while sorted remains slower at every measured size. Neither baseline scan overtakes plain: compression's traffic savings do not cure skew and decoder instruction cost. The largest observed pool high-water allocation is **4,938,277,632 bytes**, well below 12 GiB.

The generator's initial run failed its membership oracle because `Word::insert` ORs rather than replaces a field. The generator now clears the original leading field before offset insertion and checks strict sorted uniqueness before publishing the unsafe sorted batch. All eight corrected scale runs passed their full read oracles; no failed run contributes timing.

### Counter check beyond L2

Whole-run size is not the lookup working-set size. Additional warmed full profiles at source-90 2^24 and 2^28 rows show that the 134 MB plain run still has **99.7% L2 hits for shuffled prefix lookups**: the queried endpoints/search pivots fit even though a full row scan does not. Its scan has only 52.9% L2 hits and 47.1% DRAM utilization. Thus attributing the first crossover solely to total bytes crossing L2 would be incorrect.

At 2^28 rows the evidence changes:

| Metric, E / P / C | Sorted | Shuffled | Membership | Scan |
|---|---|---|---|---|
| L2 hit % | 43.2/44.2/43.2 | 87.1/86.4/94.6 | 72.5/72.6/96.0 | 50.6/44.0/68.0 |
| DRAM peak % | 45.0/45.9/63.3 | 15.7/16.5/14.9 | 21.2/22.1/13.5 | 50.0/32.4/9.4 |
| Occupancy % | 81.6/55.7/60.2 | 95.5/64.8/65.9 | 37.9/37.4/43.0 | 80.2/37.5/36.9 |

Sorted traversal now benefits from reduced physical footprint and approaches a bandwidth regime. Shuffled and membership retain strong dependent-load latency costs; packed's smaller queried footprint increases L2 hit rates. The packed scan still fails to exploit its potential traffic saving, with only 9.4% DRAM peak despite being slower. `architecture-scale-counters.json` records all five workloads and PC stall distributions for all three configurations at both sizes.

## Attempt log

### Fixed 32-way sparse scan partitioning — rejected setting

Split each sparse high-fanout interval into 32 contiguous pieces, one CTA per piece; reduce partial checksums in a second launch. Bounds and checksums remain per-probe, including overlapping intervals and multiple runs. This applies identically to all three adapters, not to the engine implementation. Three tenant-free processes/configuration and all 28 tests passed. Xalan E/P/C scans change **62.638/63.105/118.344 → 64.737/65.954/95.510 ms**. State 136 improves **1.329/1.381/7.891 → .228/.228/.506 ms**, but state 112 is **4.327/4.402/23.454 → 7.313/8.579/11.024 ms**. The extra searches/empty pieces outweigh parallelism for the plain state-112 scan. This fixed setting is rejected in favor of evaluating limited partition concurrency; no layout or retained-byte change is involved.

### Limited partition concurrency — kept

Use 32 pieces for at most 64 probes, eight for 65–1024 probes, and the existing thread/warp policy otherwise. Three clean repetitions/configuration and all 28 tests pass, including engine-adapter checks added to the overlapping-range, empty-range, singleton and publication-invalidation regression across all four word widths. Xalan E/P/C scan totals are **59.621/59.854/90.043 ms**, versus baseline **62.638/63.105/118.344 ms**. State 112 becomes **2.112/2.372/5.364 ms**; state 136 **.226/.232/.507 ms**. This keeps the scheduling gain without the fixed-32 setting's plain-path regression. Prefix/membership movement is about 1% across the batches; those kernels were not changed. The scan retains no new device storage: temporary partials are `probes × partitions × 24` bytes (140,160 bytes at state 112), freed stream-ordered after reduction. One additional launch is included in all scan times.

### Reversed empty intervals — correctness guard

Partition length uses saturating subtraction so an interval with `high < low` cannot wrap into an out-of-bounds scan. The regression now includes such an interval for every word width and both storage modes, alongside the engine adapter. Its first assertion exposed the engine's existing clamp-to-empty bounds contract versus packed's independent bounds; the oracle now checks each existing contract explicitly, without changing engine behavior. The final default build passes **all 28 tests in 4.47 s**, all six complete xalan replay runs, and both largest-scale three-configuration oracles. Normal-range timing remains consistent with the kept partition setting.

## Final default-build results

Three fresh, interleaved tenant-free processes per experimental storage configuration. Times are **milliseconds for ten repetitions over all 131 xalan states**. Parentheses are median **same-process experimental/engine ratios**, not ratios of unrelated medians. All admission, final-content and five read oracles pass. `architecture-final.json` includes min/max, steady-state and representative-state details.

| Workload | Engine ms | New plain ms (ratio) | New packed ms (ratio) |
|---|---:|---:|---:|
| Sorted prefix | 17.507 | 17.656 (1.010) | 21.772 (1.243) |
| Shuffled prefix | 39.755 | 39.244 (.990) | 44.654 (1.122) |
| Mixed prefix | 47.743 | 47.802 (1.003) | 53.982 (1.131) |
| Scan | 59.664 | 59.814 (1.004) | 90.010 (1.509) |
| Membership | 20.371 | 21.122 (1.041) | 27.531 (1.355) |

The measured scheduling gain versus this study's baseline is **23.94% packed**, **5.21% new plain**, **4.75% engine** scan time. Scan throughput is **4194.634 / 4184.086 / 2780.466 million rows/s**. State 112 scan is **2.112 / 2.388 / 5.369 ms**, versus baseline **4.327 / 4.402 / 23.454 ms**; state 136 is **.226 / .231 / .508 ms**, versus **1.329 / 1.381 / 7.891 ms**. State 90's non-sparse scan instead moves **1.550 / 2.031 / 2.632 → 1.623 / 2.084 / 2.707 ms**: the shared kernel's extra scheduling arithmetic/register footprint is a small regression retained for the larger net win, not hidden.

Final retained pool bytes stay **269,729,792 / 269,691,648 / 96,710,144**; admission peaks **291,988,736 / 291,941,376 / 103,330,816**. No layout/index bytes were added.

### Fixed floor versus sufficiently large batches

Median one-CTA launch/output-allocation floor is **5.213 / 5.137 / 5.190 us**, E/P/C. The following costs use only states with at least 30,000 probes; they are raw synchronized costs, not unstable ratios after subtracting a nearly equal floor.

| Workload | Qualifying states | Engine ns/probe | New plain ns/probe | New packed ns/probe |
|---|---:|---:|---:|---:|
| Sorted prefix | 67 | .0834 | .0866 | .1138 |
| Shuffled prefix | 67 | .2569 | .2558 | .2917 |
| Mixed prefix | 76 | .2411 | .2430 | .2764 |
| Scan | 67 | .3158 | .3023 | .4172 |
| Membership | 103 | .1863 | .1941 | .2606 |

The probe threshold deliberately does not describe sparse long scans: use their row throughput/state breakdown instead. Per-state JSON also reports floor-adjusted ns/probe as a supplementary estimate, repeats, probe counts and unadjusted time.

### Largest synthetic size: 2^28 rows

All three configurations share each process. The largest-size rows below are from the final guarded default build; `architecture-scale-final.json` also records the repeated 2^22/2^24/2^26 scale points from the partitioned feature-off build before that empty-interval guard. Both complete scale series have the same crossover conclusions. Values are **ms per repetition**; parentheses are same-process engine ratios.

| Source; workload | Engine ms | New plain ms (ratio) | New packed ms (ratio) |
|---|---:|---:|---:|
| 90; sorted | .5488 | .5367 (.978) | .3402 (.620) |
| 90; shuffled | 3.8211 | 3.5602 (.932) | 2.1171 (.554) |
| 90; mixed | 6.5542 | 6.0995 (.931) | 3.6003 (.549) |
| 90; membership | .1042 | .1106 (1.061) | .0548 (.526) |
| 90; scan | 6.6582 | 9.3324 (1.402) | 10.9282 (1.641) |
| 112; sorted | .0215 | .0247 (1.151) | .0282 (1.315) |
| 112; shuffled | .1662 | .2160 (1.299) | .1613 (.970) |
| 112; mixed | .3869 | .3211 (.830) | .2419 (.625) |
| 112; membership | .1540 | .1506 (.978) | .0641 (.416) |
| 112; scan | 8.5462 | 8.8787 (1.039) | 15.0463 (1.761) |

| Largest scan throughput | Engine GB/s | New plain GB/s | New packed GB/s |
|---|---:|---:|---:|
| Source 90, logical plain-row bytes | 322.53 | 230.11 | 196.51 |
| Source 90, representation bytes | 322.53 | 230.11 | 57.72 |
| Source 112, logical plain-row bytes | 502.56 | 483.74 | 285.45 |
| Source 112, representation bytes | 502.56 | 483.74 | 42.76 |

These are full-scan dataset-equivalent rates, not hardware bus counters. Retained bytes at source 90 are **2,147,483,704 / 2,147,483,712 / 630,745,528**, and at source 112 **4,294,967,352 / 4,294,967,360 / 643,309,764**. Peak pool allocation remains at most **4,938,277,632 bytes**. Packed search wins once its queried footprint/locality advantage matters; scans do not cross over in this study, and fixed sparse scheduling does not address the many-probe replicated-skew case.

## Techniques and references

- [Henneberg & Schuhknecht, Is Binary Search Really All You Need?](https://arxiv.org/html/2506.01576v2): locality, coalesced searches, arithmetic versus memory-latency regimes and limited concurrency. Our existing unpacked fences are already an upper-level pivot structure; the counters distinguish their cached search from the packed residual chain.
- [Osama et al., A Programming Model for GPU Load Balancing](https://arxiv.org/html/2301.04792v1): distribute finer-grained atoms of work instead of assigning an entire skewed tile to one worker. The scan experiment partitions each selected interval across CTAs rather than merely increasing the threads of its single CTA.

## Reproduction

Build from `experiments/gpu` after sourcing `/dev/shm/gpu-env.sh`, under `flock -s /tmp/mini_linq-timing.lock`, with `cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release -p gpu-datalog-runtime --bin packed-replay`. Run binaries alone under `flock /tmp/mini_linq-gpu.lock`, querying `nvidia-smi --query-compute-apps=pid,process_name --format=csv,noheader` before and after every process. `--state 90 --scale-rows 268435456` generates a single scaled run from real fresh batches and measures all three configurations in one process. It preserves field widths and suffix distributions, offsets the leading field by its real span, and truncates only the final replica. All prefix ranges and scanned checksums use the generated host oracle; membership has independently generated present/successor-miss probes. Construction and H2D are outside read timings. Pool high-water memory is checked against 12 GiB. Xalan replay admission and full-content oracles remain unchanged.

For NVTX-only profiling, add `--features cq-gpu-runtime/nvtx` **after** the cargo `--`, and copy that executable outside the normal timing target. Set `NSYS=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/compilers/bin/nsys` and `NCU=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/compilers/bin/ncu`. For a small reproducible state-137 selection, run the following inside the GPU lock with before/after tenant queries around each profiler invocation:

```sh
profile/profile.sh "$ARCH/state137" -- "$ARCH/replay-nvtx-on" --state 137
python3 profile/analyze.py "$ARCH/state137.sqlite" \
  --range read_prefix_shuffled --window-index 1 \
  --json "$ARCH/state137-shuffled.analysis.json"
python3 profile/ncu.py \
  --nvtx-range read_prefix_shuffled/state_137 --range-instance 2 --top 1 \
  "$ARCH/state137-shuffled.analysis.json" "$ARCH/state137-ncu" -- \
  "$ARCH/replay-nvtx-on" --state 137
# Full warm-cache sections, same NVTX range:
"$NCU" --set full --cache-control none --clock-control none \
  --nvtx --nvtx-include read_prefix_shuffled/state_137/ \
  --range-filter no::2 --kernel-name 'regex:^packed' \
  --launch-skip 1 --launch-count 1 -o "$ARCH/state137-full" \
  "$ARCH/replay-nvtx-on" --state 137
```

Occurrence 1 selects the engine; 2 selects the experimental set. Add `--plain-storage` to profile the plain control. Replace the workload/state with `read_range_scan/state_112` for the high-fanout scan. Admission-only subranges can end before their asynchronous kernels; attribute them through launch correlation, not a GPU timestamp containment test.

## Remaining limits and what also helps plain

- **Measured for plain, not merely proposed:** the same limited multi-CTA partitioning improves xalan scan cost and dramatically accelerates sparse state 136. Keeping range pieces contiguous preserves warp coalescing. Sorting the same state-137 probes changes engine device time from roughly 296 to 46 us; that is a locality-control experiment, not an end-to-end query-sort speedup because sorting/restoring outputs was not charged.
- **[INFERENCE] Search direction:** shuffled L2-latency-bound work is a candidate for partially sorted batches, small cooperating search groups, or cache-line stopping pivots; sorted work instead needs fewer instructions/dependency steps. A blanket bigger pivot buffer, more occupancy, or extra K-ary arithmetic is not justified in both regimes. The existing plain fence buffer was retained.
- **[INFERENCE] Scan direction:** a global prefix sum of actual interval lengths and fixed-size output tasks would avoid repeating bounds for empty fixed pieces and would address the scaled source-112 skew after probe count exceeds 1024. That more general scheduler was not implemented or claimed as a measured win; the committed change is bounded static interval partitioning with a measured overhead/gain tradeoff.
- Transposed/miniblock decoding and in-header separators would change format/bytes; they were not tested. The current row-major variable shifts and frame metadata still cost hundreds of active thread instructions per row. No whole run is unpacked by a timed read, and **no layout/index bytes were added**.
- The tests retain every existing exactness check and additionally cover engine-adapter ranges and reversed empty intervals. Source-line profiling remains blocked only by the documented lineinfo assembler failure; hardware counters and SASS PC sampling work.
