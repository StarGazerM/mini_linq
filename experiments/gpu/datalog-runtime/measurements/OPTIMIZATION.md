# Packed-run Xalan optimization

## Protocol and initial profile

Real trace: 131 states, 25,026,847 final rows, 2,161 admissions. B=256 unless stated otherwise. Every replay checks subtraction, final contents and prefix ranges. Timings below are milliseconds; memory is decimal MB. Runs use CUDA 13.0 via `/dev/shm/gpu-env.sh`, sm_89 materialized cubins, the prescribed build/GPU locks, and before/after compute-tenant queries. `timing_indicative=false` identifies uncontended runs. Profile timings are not used as throughput measurements.

Three fresh baseline runs (`baseline-{1,2,3}.json`), no foreign tenant:

| Metric | Plain median [range] | Packed median [range] |
|---|---:|---:|
| Sort + subtraction | 161.414 [161.316,161.695] | 175.709 [175.285,176.081] (sort 111.993; subtract 63.716) |
| Seal/adopt | 0.483 [0.481,0.488] | 45.676 [45.664,45.848] |
| Merge | 17.246 [17.237,17.320] | 85.706 [85.687,85.883] |
| Admission | 179.141 [179.041,179.498] | 307.091 [306.637,307.812] |
| Prefix lookup | 17.481 [17.466,17.488] | 49.123 [49.113,49.172] |
| Final / peak pool MB | 269.662 / 291.989 | 94.071 / 100.342 |

Packed merge CUDA-event intervals: partition 14.944, analyze 13.868, pack 14.072 ms. Their sum leaves 42.822 ms of wall time outside these intervals (including offsets, size readback, allocation and host gaps). CUDA events include stream scheduling gaps, not just kernel execution.

`initial-cuda_gpu_kern_sum.csv`, `initial-cuda_api_sum.csv`, `initial-cuda_gpu_mem_time_sum.csv` contain the initial nsys reports. They cover BOTH representations and validation/lookup, not admission alone. Whole-replay API totals: 59,259 launches / 181.225 ms host API time; 12,431 D2H / 427.672 ms; 11,012 H2D / 90.093 ms; 48,521 stream synchronizations / 137.701 ms; 7,887 event synchronizations / 2.955 ms; 33,447 memsets / 83.222 ms; 17 device allocations / 2.828 ms and 17 frees / 7.789 ms. The large D2H total includes oracle downloads and must not be attributed wholly to admission. Kernel totals include merge partition 10.660 ms, analyze 11.033 ms, merge-pack 10.385 ms, and packed membership 24.160 ms.

Replay instrumentation now retains per-admission operation deltas. `baseline-classes.json` is a separate instrumented baseline (not one of the three original repetitions). Classes use fresh input rows, including zero-row admissions, and charge all chained merges to that admission:

| Fresh rows | Admissions | Sort ms | Subtract ms | Seal ms | Merge ms | Merges |
|---|---:|---:|---:|---:|---:|---:|
| 0–8 | 928 | 17.190 | 15.181 | 1.028 | 6.042 | 186 |
| 9–256 | 513 | 22.124 | 20.642 | 15.681 | 27.049 | 498 |
| 257–4096 | 308 | 18.637 | 13.699 | 10.127 | 22.097 | 323 |
| 4097–65536 | 321 | 40.795 | 13.159 | 18.396 | 26.160 | 273 |
| >65536 | 91 | 14.761 | 1.625 | 5.620 | 5.294 | 34 |

Small admissions dominate the number of round trips; 9–256-row admissions alone spend 42.730 ms sealing and merging. Hypothesis for the first change: a fresh batch immediately consumed by a merge should remain an immutable plain input, avoiding disposable sealing kernels and a size readback without changing final representation.

## Attempt log

| Attempt / hypothesis | Sort / subtract / seal / merge ms (median) | Admission ms [range] | Lookup ms [range] | Final / peak MB | Decision |
|---|---|---|---|---|---|
| Merge the fresh plain batch directly; avoid disposable seal | 112.947 / 63.981 / 23.654 / 83.812 | 284.393 [281.749,285.450] | 49.146 [48.816,49.291] | 94.071 / 100.342 | Keep: −22.698 ms admission vs fresh baseline |
| Allocate fully overwritten codec/merge outputs without clearing; remove redundant memset API/device work | 112.956 / 64.146 / 19.968 / 72.787 | 269.881 [269.147,275.216] | 49.252 [49.249,49.352] | 94.071 / 100.342 | Keep: −14.512 ms vs direct merge |
| Keep runs through 256 rows plain; use eight-row serial merge tiles | 113.357 / 61.562 / 16.390 / 61.432 | 252.899 [252.418,252.921] | 48.499 [48.417,48.528] | 94.054 / 100.335 | Keep: −16.982 ms vs uninitialized outputs |
| Stage field/view metadata asynchronously on the consuming stream; avoid upload barriers | 112.468 / 57.976 / 16.718 / 59.320 | 246.288 [246.128,247.383] | 45.048 [45.023,45.181] | 94.054 / 100.335 | Keep: −6.611 ms admission, −3.451 ms lookup |
| Raise plain cutoff to 16,384 rows; amortize packed round trips while bounding retained bytes | 112.733 / 53.243 / 12.634 / 44.325 | 222.676 [222.315,223.808] | 41.748 [41.561,41.758] | 95.514 / 101.705 | Keep: −23.612 ms admission, +1.460 MB live |
| Translate a probe once into concatenated residual space for blocks with ≤64 residual bits | 112.428 / 47.786 / 12.523 / 44.157 | 216.870 [216.745,217.109] | 30.278 [30.153,30.357] | 95.514 / 101.705 | Keep: −5.806 ms admission, −11.470 ms lookup |
| Raise plain cutoff to 32,768 rows | 112.240 / 47.366 / 11.508 / 44.002 | 215.078 [213.915,216.545] | 30.072 [29.915,30.263] | 96.674 / 103.330 | Keep: −1.792 ms admission, +1.160 MB live |
| Launch enough CTAs for the plain merge's eight-row tiles | 112.713 / 47.928 / 11.559 / 40.655 | 212.854 [210.190,219.808] | 30.034 [29.918,30.214] | 96.674 / 103.330 | Keep: merge −3.347 ms; kernel evidence −3.452 ms |
| B=512: reduce block metadata and offset scan work | 113.004 / 48.083 / 11.029 / 38.219 | 210.335 [209.215,211.547] | 31.575 [31.400,31.865] | 99.474 / 106.233 | Reject as default: modest admission gain costs +2.799 MB live and +1.541 ms lookup |
| B=1024: further reduce block metadata | 113.277 / 48.255 / 10.988 / 39.702 | 212.541 [211.892,212.689] | 32.684 [32.521,36.426] | 104.110 / 111.862 | Reject as default: no clear admission gain, +7.436 MB live and slower lookups |
| Retain transient fresh prefixes without copying; trim only published plain runs with a native D2D copy | 113.354 / 48.411 / 8.715 / 41.034 | 211.837 [209.454,215.827] | 30.184 [29.984,30.335] | 96.674 / 103.330 | Keep: seal −2.844 ms; no whole-run decode/copy in admission |

Direct-merge evidence: `direct-{1,2,3}.json`, `profile-direct.json`, `direct-cuda_*.csv`. No foreign tenants. All 21 current tests pass (20 original plus the direct mixed-input test across four word widths and three block sizes); all three real-trace oracles pass. An initial test command selected an older 18-test executable; that result was discarded and the exact newly built 21-test executable was run instead.

Direct merge class seal/merge ms (`direct-2.json`): 0–8: 0.877/6.105; 9–256: 6.788/26.116; 257–4096: 4.435/21.483; 4097–65536: 7.041/25.323; >65536: 4.596/5.128. Final contents and pool bytes are unchanged.

Uninitialized-output evidence: `uninit-{1,2,3}.json`, `profile-uninit.json`, `uninit-cuda_*.csv`. Every header, frame, fence, partition, total and payload word is fully written by its producing kernel on the same stream before consumption; empty payloads are never read. All 21 tests and real-trace oracles pass. No foreign tenants. Class seal/merge ms (`uninit-2.json`): 0–8: 0.868/5.118; 9–256: 5.598/22.215; 257–4096: 3.770/18.445; 4097–65536: 5.845/22.309; >65536: 3.902/4.700.

256-row cutoff evidence: `small256-{1,2,3}.json`, `profile-small256.json`, `small256-cuda_*.csv`. All 21 tests pass, including the expanded tiny cutover sweep through 258 rows for all four widths. All real-trace oracles pass, no foreign tenants. Class seal/merge ms (`small256-2.json`): 0–8: 0.868/2.864; 9–256: 1.905/12.958; 257–4096: 3.701/18.304; 4097–65536: 5.951/22.373; >65536: 4.114/4.716. This cutoff slightly reduces pool bytes because tiny frame/header allocations disappear.

Host `perf` sampling (`host-perf.txt`) identifies replay oracle B-tree insertion as 27.85% of sampled cycles, reporting 9.58%, and host candidate filtering 5.78%. These are outside timed admission; whole-process CPU time is not an admission metric. Nsys kernel timestamps give a more useful pipeline-gap measure: initial 1,226 packed merge pipelines span 69.365 ms, of which kernels consume 37.420 ms and inter-kernel gaps 31.944 ms; after direct merge and clearing removal, spans are 61.223 ms, kernels 34.345 ms, gaps 26.878 ms. These gaps include host submission, transfer and synchronization rather than computation kernels.

Asynchronous metadata evidence: `async-{1,2,3}.json`, `profile-async.json`, `async-cuda_*.csv`. All 21 tests and real-trace oracles pass; no foreign tenants. Pool-owned pinned staging retains short-lived host descriptors until transfer completion. Class seal/merge ms (`async-2.json`): 0–8: 0.873/2.863; 9–256: 1.909/12.665; 257–4096: 3.489/17.671; 4097–65536: 6.799/21.726; >65536: 3.745/4.660. The 789 packed merge pipelines span 45.713 ms: kernels 28.906 ms, gaps 16.808 ms.

16K cutoff evidence: `small16k-{1,2,3}.json`, `profile-small16k.json`, `small16k-cuda_*.csv`. All 21 tests pass with threshold-relative direct-merge and boundary coverage; all real-trace oracles pass, no foreign tenants. Class seal/merge ms (`small16k-2.json`): 0–8: 0.851/2.772; 9–256: 1.876/7.891; 257–4096: 1.253/8.640; 4097–65536: 3.517/20.305; >65536: 4.983/4.665. Packed merge pipelines fall from 789 to 300, spanning 24.210 ms (17.647 ms kernels, 6.563 ms gaps). Final live remains close to 94 MB and well below 105 MB.

Residual-search evidence: `residual-{1,2,3}.json`, `profile-residual.json`, `residual-cuda_*.csv`. All 22 tests pass, including new frame-underflow/overflow, zero-width residual, empty-schema, full-u64 residual, plain-encoding and wide-word bound cases. A small-domain exhaustive CPU model also checked both bound modes. All real-trace oracles pass; no foreign tenants. Larger-than-u64 residuals and uncompressed blocks retain the exact generic decoder. Class seal/merge ms (`residual-2.json`): 0–8: 0.834/2.747; 9–256: 1.843/7.882; 257–4096: 1.247/8.637; 4097–65536: 3.518/20.225; >65536: 4.970/4.630. Profile packed membership falls from 16.208 to 11.656 ms, lookup kernels from 36.003 to 23.520 ms.

32K cutoff evidence: `small32k-{1,2,3}.json`, `profile-small32k.json`, `small32k-cuda_*.csv`. All 22 tests and real-trace oracles pass; no foreign tenants. Class seal/merge ms (`small32k-2.json`): 0–8: 0.856/2.736; 9–256: 1.882/7.873; 257–4096: 1.270/8.429; 4097–65536: 2.566/20.328; >65536: 4.934/4.636. The remaining 243 packed pipelines span 21.279 ms (15.927 kernels, 5.352 gaps), but plain merge kernels rise from 4.465 to 6.788 ms. The single-CTA plain launch inherited from the eight-row cutoff now serializes larger plain outputs; the next attempt sizes that grid to the actual eight-row tiles.

Plain-grid and block-size evidence: `grid-{1,2,3}.json`, `b512-{1,2,3}.json`, `b1024-{1,2,3}.json`, `profile-grid.json`, `grid-cuda_*.csv`. All 22 tests and all nine real-trace oracles pass; no foreign tenants. Grid class seal/merge ms (`grid-2.json`): 0–8: 0.849/2.674; 9–256: 1.866/7.582; 257–4096: 1.255/6.945; 4097–65536: 2.486/18.217; >65536: 4.856/4.567. Plain merge kernel device time falls from 6.788 to 3.336 ms with identical launch count (1,071). One grid repetition has a sort-time outlier (118.511 ms versus 111.962–112.713 ms); no foreign tenant was observed and it remains included in the reported range. Keep B=256 for its smaller footprint and faster rule-read proxy.

Zero-copy evidence and final selected implementation: `nocopy-{1,2,3}.json`, `profile-nocopy.json`, `nocopy-cuda_*.csv`. All 22 tests pass, now including surplus-capacity fresh batches retained across a mixed merge. All real-trace subtraction, final-content and prefix-range oracles pass; no foreign tenants. Previously `from_plain` copied spare-capacity inputs using the diagnostic unpack kernel, even when a merge immediately consumed them. Admission now retains that immutable prefix directly, and only trims a retained plain run's spare capacity using a native D2D transfer. Diagnostic whole-run downloads remain confined to explicit validation.

## Final results and remaining gap

Selected: B=256, plain cutoff 32,768 rows. Three final unprofiled repetitions, no foreign tenants. Values are medians [min,max]; memory is identical in all three runs. “Initial” is the freshly remeasured original implementation, not the earlier supplied measurement.

| Metric | Final plain | Initial packed | Final packed |
|---|---:|---:|---:|
| Sort + subtraction ms | 163.603 [161.831,164.215] | 175.709 [175.285,176.081] | 162.031 [160.395,166.071] |
| Seal/adopt ms | 0.498 [0.481,0.504] | 45.676 [45.664,45.848] | 8.715 [8.484,8.772] |
| Merge ms | 17.655 [17.240,17.777] | 85.706 [85.687,85.883] | 41.034 [40.575,41.042] |
| Admission ms | 181.762 [179.552,182.490] | 307.091 [306.637,307.812] | 211.837 [209.454,215.827] |
| Prefix lookup ms | 17.671 [17.593,17.713] | 49.123 [49.113,49.172] | 30.184 [29.984,30.335] |
| Final pool MB | 269.662 | 94.071 | 96.674 |
| Peak pool MB | 291.989 | 100.342 | 103.330 |

Admission improves 31.0%, lookup 38.6%; final pool remains 64.1% smaller than plain. The live-memory trade is +2.604 MB versus the initial packed implementation, still below 105 MB. The 179 ms admission target is **not met**: final median is 32.837 ms above it and 30.075 ms above the contemporaneous plain median. Independent component medians need not sum to the median total.

The residual gap is publication, not subtraction: sort+subtract is 1.572 ms faster than plain, but sealing adds about 8.217 ms over adoption and merging adds 23.379 ms. Final merge CUDA-event intervals are partition 5.412, analyze 6.191 and pack 12.586 ms; the remaining 16.845 ms of merge wall time lies outside those intervals, including offsets, exact-size D2H readbacks, allocation and submission. Nsys directly measures 243 packed merge pipelines spanning 21.357 ms: 15.942 ms in kernels and 5.415 ms between them. The 1,071 plain merges now consume only 3.335 ms of kernel time. Exact payload sizing still executes 347 scalar readbacks (104 seals + 243 packed merges) and a serial offset kernel; removing readbacks through retained worst-case allocations would trade away the compact-memory contract unless a different allocation/layout mechanism is introduced. No such unmeasured trade is claimed here.

Lookup remains 1.71× plain. The profile records 23.220 ms of packed lookup kernels versus 12.514 ms plain (both include the warmup plus ten timed repetitions). Fence traversal and residual extraction still add device work, and >64-bit residual blocks retain generic decoding. The ≤64-bit path substantially reduces this gap without adding bytes. Larger blocks made the lookup proxy slower, so B=256 stays selected.

Final admission classes (`nocopy-2.json`, representative run; full per-class medians/ranges for every attempt are in `admission-classes.csv`):

| Fresh rows | Admissions | Sort ms | Subtract ms | Seal ms | Merge ms |
|---|---:|---:|---:|---:|---:|
| 0–8 | 928 | 17.240 | 11.368 | 0.491 | 2.810 |
| 9–256 | 513 | 22.058 | 15.010 | 0.883 | 7.956 |
| 257–4096 | 308 | 18.453 | 10.406 | 0.554 | 7.101 |
| 4097–65536 | 321 | 41.046 | 10.308 | 1.921 | 18.437 |
| >65536 | 91 | 14.824 | 1.319 | 4.923 | 4.731 |

## Final CUDA profile

The following nsys totals cover both representations, validation, and warmed lookup calls. They are not admission-only API totals. All per-kernel specialization timings/counts and every API type are retained in `nocopy-cuda_*.csv`; `profile-summary.csv` aggregates kernel specializations and compares every accepted step. `optimization-summary.csv` contains unprofiled timing medians and ranges.

| API | Initial count / host ms | Final count / host ms |
|---|---:|---:|
| Kernel launch | 59,259 / 181.225 | 52,808 / 165.525 |
| D2H | 12,431 / 427.672 | 10,319 / 398.698 |
| H2D | 11,012 / 90.093 | 8,900 / 85.432 |
| D2D | 2,590 / 16.388 | 2,833 / 17.870 |
| Stream synchronize | 48,521 / 137.701 | 40,993 / 102.470 |
| Event synchronize | 7,887 / 2.955 | 7,888 / 2.934 |
| Event record | 75,162 / 35.148 | 65,922 / 32.376 |
| Memset | 33,447 / 83.222 | 19,926 / 56.106 |
| Device allocate | 17 / 2.828 | 17 / 2.832 |
| Device free | 17 / 7.789 | 17 / 7.694 |
| Pinned host allocate | 1 / 1.330 | 2 / 2.336 |

Final device memory-operation times: D2H 228.286 ms, H2D 58.376 ms, D2D 3.049 ms, memset 15.241 ms. Oracle transfers dominate D2H; these are deliberately not charged to admission. There are no stream-wait-event calls in the report; event synchronization is shown above.

| Packed operation kernels | Initial launches / device ms | Final launches / device ms |
|---|---:|---:|
| Seal analysis | 1,233 / 3.572 | 104 / 0.512 |
| Seal packing | 1,233 / 3.327 | 104 / 0.546 |
| Merge partition | 1,226 / 10.660 | 243 / 2.810 |
| Merge analysis | 1,226 / 11.033 | 243 / 4.031 |
| Merge packing | 1,226 / 10.385 | 243 / 5.038 |
| Offsets (seal + merge) | 2,459 / 8.450 | 347 / 5.340 |
| Plain tiny-run merge | 88 / 0.135 | 1,071 / 3.335 |
| Membership | 1,516 / 24.160 | 1,516 / 11.278 |
| Lookup (including warmup) | 1,441 / 40.886 | 1,441 / 23.220 |

Reproduction: source `/dev/shm/gpu-env.sh`; from `experiments/gpu`, build with `flock -s /tmp/mini_linq-timing.lock cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release -p gpu-datalog-runtime --bin packed-replay`. Build tests similarly with `oxide test ... -- --release -p gpu-datalog-runtime --test packed_run --no-run`, then run the exact emitted test executable under the GPU lock with `timeout 100` and `--test-threads=1`. Run `target/release/packed-replay --output PATH --block 256` under `flock /tmp/mini_linq-gpu.lock`, with the required compute-process query before and after each repetition. Profile with `/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/compilers/bin/nsys profile -t cuda --sample=none --cpuctxsw=none`; export `cuda_gpu_kern_sum`, `cuda_api_sum`, `cuda_gpu_mem_time_sum` with `nsys stats --format csv --force-export=true`. Raw session traces live under `/dev/shm/packed-run/profiles`; CSV/JSON evidence under `/data/design-notes/doop-memory/` does not depend on those temporary files.

## Round 2: admission controls and fused small batches

Round-2 baseline profiles and measurements are `r2-base-*`. The replay now has `--plain-storage`: it uses exactly the experimental set's admission, geometric merges, profiling and lookup code, but retains every run plain. Its results remain under `packed_*` JSON keys for compatibility with the measurement schema, with `"storage": "plain"` explicitly recorded. This is not the engine's original plain path, which remains separately measured in every replay.

| Round-2 baseline | Engine plain | Experimental admission, plain storage | Experimental admission, packed storage |
|---|---:|---:|---:|
| Admission ms, median [range] | 183.891 [183.259,184.406] | 187.580 [186.645,192.535] | 218.298 [214.528,218.352] |
| Lookup ms, median [range] | 17.732 [17.713,17.794] | 20.457 [20.450,20.721] | 30.310 [30.301,30.347] |
| Final / peak MB | 269.662 / 291.989 | 269.656 / 291.940 | 96.674 / 103.330 |

The first round-2 change sorts arbitrary-order small candidates with a shared-memory bitonic network, removes adjacent duplicates, probes all existing runs, and compacts the survivors in **one kernel and one count readback**. Publication continues through the existing one-kernel plain merge when the geometric merge output stays small. Zero candidate rows return without any CUDA launch, copy or synchronization. No host knowledge of trace ordering is used.

Thresholds 64, 256 and 1024 were each measured three times (`r2-small{64,256,1024}-*.json`), then interleaved with threshold zero in three rounds (`r2-balanced*.json`). Host contention varied substantially even without a foreign GPU tenant: the recorded load average was 12.78 and unchanged engine-plain admission ranged from 179.205 to 237.662 ms in the balanced 1024 measurements. Do not interpret that host variation as a storage effect. The full distributions are retained, not filtered.

| Interleaved threshold | Packed admission median [range] ms | Lookup median [range] ms | Final / peak MB |
|---|---:|---:|---:|
| 0 (generic sorter) | 233.109 [210.625,237.672] | 33.358 [30.220,35.792] | 96.674 / 103.330 |
| 64 | 201.898 [192.824,231.127] | 31.832 [30.552,33.916] | 96.674 / 103.330 |
| 256 | 178.325 [172.533,182.790] | 31.043 [30.076,34.037] | 96.674 / 103.330 |
| 1024 | 194.900 [166.420,238.781] | 32.226 [30.010,35.573] | 96.674 / 103.330 |

Keep 1024 based on the affected candidate class, not the noisy unrelated large sorts: for 257–1024 candidates, sort+subtract is 10.730 ms median [10.137,12.089] with threshold 1024 versus 16.876 [16.610,17.688] at threshold 256. The >1024 candidate class varied from 87.348 to 136.139 ms despite running identical code. At thresholds 0/64/256/1024 the 1–64 candidate class medians are 45.243/14.695/14.147/15.243 ms, confirming the fusion benefit.

`profile-r2-small.json` and `r2-small-cuda_*.csv` show 1,051 fused launches (908 scalar, 143 wider) taking 15.090 ms device time, while generic membership launches fall from 1,516 to 482. Full-replay kernel launches fall from the preceding 52,808 to 44,270, D2H calls from 10,319 to 9,285, and stream synchronizations from 40,993 to 34,750. All 23 tests pass, including arbitrary order, duplicates, empty candidates, partial tiles, full-width values, packed probes and the plain-storage control. All trace oracles passed.

`r2-small-plain-3.json` observed a foreign RustDesk compute process and is **indicative only**, excluded from clean control summaries. The replacement is `r2-small-plain-4.json`; the original contaminated result is retained.

### RCCL alternating-buffer histogram defect

The varying-digit experiment initially failed the real trace oracle at state 86 (`formalparam`), admission sequence 9, with 111,950 u128 rows. Length was correct, but row zero was `0xa800000000002d2af` instead of `0x6800000000002d2ae`. This was a correctness failure, not a timing result.

In the installed RCCL checkout `a74cb59`, `rccl-cub/src/radix.rs:725–730` calls the u64 pair histogram with `&*buffers.first().0` (line 727), regardless of the current selector. The 32-bit pair branch has the same issue at line 719. The onesweep loop below it correctly selects first/second at lines 739–743. The single-tile branch at lines 681–707 also honors the selector.

Exact triggering condition: a **multi-tile** radix call starts with the DoubleBuffer selector on the alternate buffer, and the current keys have been changed since the preceding sort (as the next-lane gather does). An odd number of preceding radix passes leaves that alternate selector. The histogram reads stale physical-first keys while onesweep scatters current alternate keys, so its bucket offsets describe the wrong distribution.

Observed two-call sequence: sort the low lane over `0..37` (five eight-bit passes), gather the high lane into the selected alternate key buffer while retaining its row indices, then sort `0..18` (three passes) using the same DoubleBuffer. State 86's XOR masks are `0x1f0007ffff` and `0x3ffff`.

Smallest call sequence for a standalone RCCL regression is two sorts with an intervening write:

1. Start with current=first, `first_keys[i] = i & 1`, `first_values[i] = i`.
2. `sort_pairs_buffer(..., 0..1, ...)` leaves current=alternate after one onesweep pass.
3. Overwrite **only current keys** with `1`, retaining current values.
4. Call `sort_pairs_buffer(..., 0..1, ...)` again without rebasing.

[INFERENCE, code-derived] For u64/u32 pairs the current single-tile limit is 2,304 rows (`workspace.rs:54–64`: 256 × floor(19×4/8)), so 2,305 rows is the smallest multi-tile size for this policy. The second histogram counts the old mixture of zeros and ones instead of all ones, making the ones bucket start after the old zeros; output positions can exceed the logical extent. This minimal synthetic sequence is a proposed upstream regression, not a separately executed unsafe-library test. The real-trace sequence above was executed and failed.

The original local experimental sorter worked around this without copying by rebasing each lane's DoubleBuffer to the selected pair. That workaround was removed after the histogram selector was fixed and regression-tested on the isolated RCCL `mini-linq/packed-primitives` branch; see [DECODE.md](DECODE.md#alternating-buffer-radix-histogram--fixed-upstream-branch). Regression coverage retains independent lane values and large odd-pass windows.

The existing engine call sites do not trigger this defect today: keys-only sorts construct a fresh DoubleBuffer per call; large multi-lane row sorts use full `0..64` windows (eight passes) on every non-final lane, and only the last lane may have a partial odd-pass window. There is no subsequent lane gather after that final window. Small single-tile calls are unaffected because that dispatch honors the selector.

### Varying-digit sort attempts

The device computes the OR of each lane's differences from the first candidate, then reads back only four or fewer masks. Constant lanes are skipped; each remaining stable radix pass selects `trailing_zeros(mask)..64-leading_zeros(mask)`. Full keys remain unchanged. The replay records selected and full-schema eight-bit digit budgets per admission and state; `r2-sort-digits.csv` contains the state breakdown. The selected budget falls from 5,760 to 4,924 digits. Nsys confirms onesweep key launches fall 4,552→4,261 and pair launches 4,888→4,486 (whole replay includes unchanged engine-plain work). No trace ordering is assumed.

| Sort attempt | Packed sort ms | Packed admission median [range] ms | Lookup ms | Final / peak MB | Decision |
|---|---:|---:|---:|---:|---|
| Fused-small + generic large sort, adjacent control (`r2-varytrim-generic-*`) | 68.579 | 165.625 [164.238,166.324] | 29.999 | 96.674 / 103.330 | Control |
| Varying sort, retain all row scratch (`r2-vary-packed-*`) | 64.821 | 163.686 [161.432,164.322] | 29.980 | 96.674 / 144.447 | Reject scratch lifetime |
| Retire key/index buffers, retain compaction scratch (`r2-varytrim-packed-*`) | 65.364 | 162.336 [161.832,163.845] | 29.951 | 96.674 / 116.099 | Reject remaining scratch lifetime |
| Retire all row-sized scratch; retain only radix controls (`r2-sort-packed-*`) | 64.400 | 161.347 [161.210,168.206] | 29.853 | 96.674 / 103.331 | Keep |

The retained-control design uses the existing pool for transient row buffers and compaction workspace. Caching their maxima across unrelated states unnecessarily inflated peak live bytes. The final version adds only 1,024 peak bytes versus its generic-sort control. All 24 tests and all nine corrected varying-sort trace replays passed; the final six replays also passed, all without foreign tenants. Final sort's 70.848 ms outlier remains included in the admission range. The new plain-storage control (`r2-sort-plain-*`) measures 138.868 [138.722,139.714] ms admission and 20.289 [20.287,20.298] ms lookup, final/peak 269.656/291.941 MB. The default now uses varying sort; `--generic-sort` selects the adjacent control.

### Lookup instruction and merge occupancy profiles

`r2-lookup-{137,192}-ncu.txt` and the corresponding `*-sass.csv` report Nsight Compute counters on scalar and three-lane lookup workloads. NCU locked SM clocks near 915 MHz, so these durations are diagnostic, not throughput measurements. Both use 64 registers/thread and have 66.67% theoretical occupancy; achieved occupancy is 61.96% for state 137 and 47.79% for state 192. Scalar compute utilization is 80.95%; wide lookup has 57.65% memory and 52.42% compute utilization. Branch efficiency is 92.76%/94.45%.

The cubins have no lineinfo, so direct source-line attribution to fence search versus in-block search is unavailable (recorded in `*-source.csv`). SASS samples identify comparisons and 64-bit shifts as hot consumers: the scalar kernel's hottest instruction is `SHF.R.U64` with 562 samples; the wide kernel's hottest instruction is another `SHF.R.U64` with 246 samples. These are instruction samples, not additive stage durations or proof that load latency is absent. Subsequent lookup experiments separately change extraction and search structure.

`r2-merge-90-ncu.txt` is tenant-free. A foreign `VLLM::EngineCore` process (PID 302583) appeared during the state-112 NCU run; `r2-merge-112-ncu.txt` is **indicative only** and must not support timing conclusions.

### Exact replay configurations and pod driver

The replay always runs an unchanged engine-plain path followed by the experimental path and validates both. Therefore the engine and new-packed configurations use the same command but select different report columns:

| Configuration | Command flags after `packed-replay TRACE` | JSON metric prefix |
|---|---|---|
| Engine's existing plain path | `--block 256 --output engine.json` | `plain_*` |
| New admission path, every run plain | `--block 256 --plain-storage --output new-plain.json` | `packed_*`, with `storage="plain"` |
| New admission path, packed storage | `--block 256 --output new-packed.json` | `packed_*`, with `storage="packed"` |

All three commands use the default 1,024-candidate fused path and varying-digit sort for their experimental half. `--generic-sort` disables only varying-digit selection; `--small-admission 0` disables nonempty fused admission. Neither flag changes the engine-plain half. Replace 256 with another measured block size as needed.

`experiments/gpu/compare/pod_packed_replay.py` follows the bundle/source, manifest-binary and library-path conventions of the historical `pod_doop_phases.py`. Run:

```sh
export BUNDLE_DIR=/workspace/bundle
python3 "$BUNDLE_DIR/source/experiments/gpu/compare/pod_packed_replay.py" \
  --results /workspace/packed-results \
  --trace-archive /workspace/trace.tar.zst --reps 3 --blocks 256,512
```

The driver requires GNU tar with zstd support, extracts the supplied `trace/` archive under `/dev/shm/packed-run`, resolves `manifest.json` → `binaries` → `packed-replay`, runs from the source snapshot, and sets the requested bundle/NVIDIA `LD_LIBRARY_PATH`. It holds the GPU lock, rotates configuration and block order, and makes **three separate fresh-process invocations per repetition/block**. Although each invocation contains both halves, only its selected configuration contributes to the summary, giving exactly N samples for each column. Every original JSON and process log remains in the results directory.

Any failed replay/oracle, missing/incomplete JSON, tenant-query failure, observed foreign process before/after, or the replay's own `timing_indicative` flag aborts with a nonzero exit and an incomplete summary. It does not silently retry or include contaminated samples. It prints and writes medians/ranges for operation classes, lookup, and exact final/peak byte counts, grouping admissions by both candidate and fresh sizes. Driver `--help` and aggregation were checked locally against nine real replay reports; both class bases conserved all 2,161 admissions and total admission time. Full bundle execution is reserved for the uncontended pod.

### Parallel payload offsets (pod timing candidate)

Replace the serial header walk with one 256-thread CTA scanning 256 headers per tile: u64 warp scans, eight shared warp totals, and a carried total between tiles. It uses 64 shared bytes, adds no launch or allocation, and preserves exact payload sizing. New tests cross 255/256/257/513 header tiles with nonzero payloads.

All 25 tests and three real-trace replays pass; final/peak remain 96.674/103.331 MB. Every local replay observed `VLLM::EngineCore` and is **indicative only** (`r2-scan-local-*`, `profile-r2-scan.json`, `r2-scan-cuda_*.csv`). Contention inflated admission to 10,394.846 [10,377.726,10,400.394] ms and lookup to 2,909.619 [2,901.609,2,922.613] ms, so those wall times must not be compared with the clean baseline. The profile records 347 offset launches totaling 1.629 ms device time, versus 5.336 ms before this change; other device kernels are also perturbed by the tenant. This commit is handed to the parent for the required uncontended pod comparison before its keep/revert decision.

### Rejected: payload bound without size readbacks

An experiment allocated `rows × word_lanes` payload words and retained device-computed offsets without downloading the total. It removed exactly 347 D2H calls (9,881→9,534 in the adjacent full profiles), and the real-trace content/subtraction/prefix oracles passed. But final pool live rose to **277.609 MB**, peak to **300.389 MB**, losing the packed memory saving. The existing allocation assertion in `crossed_fields_and_empty_schema` failed; 24 other tests passed. The experiment is reverted, not a candidate for pod timing. `profile-r2-bound.json` and `r2-bound-cuda_*.csv` preserve the evidence; its wall timings are VLLM-contaminated and indicative only.

The available RCCL `DeviceBuffer::shrink_to_fit` (`rccl-memory/src/buffer.rs:186–190`) requires a host logical length and calls `reallocate`, which allocates a new block and performs a D2D copy (`:213–250`). It does not release a device-selected tail in place. The runtime `Buffer::shared_slice` also retains the entire owner allocation. Consequently, simply shrinking/slicing the bound cannot both remove this readback and retain compact live bytes with the current pool API; no logical-byte accounting shortcut is used.

### Partition + analysis fusion (pod timing candidate)

Each analysis CTA now computes its two co-ranks in two lanes, temporarily reusing the first 16 shared bytes before row staging, and publishes partition starts for merge-pack. The separate partition launch/kernel is removed. Existing merge tests now include B=256 in addition to B=1/17/1024, including empty, one-row and partial-block endpoints; all 25 tests and the real-trace oracles pass. Final/peak bytes remain 96.674/103.331 MB.

The local profile (`profile-r2-fuse.json`, `r2-fuse-cuda_*.csv`) observed no foreign process, but is still treated as diagnostic rather than the requested pod throughput comparison. It removes 243 launches. Fused analysis takes 7.875 ms device time, versus the earlier separate partition+analysis 2.820+4.030 ms: duplicated endpoint searches add about 1.025 ms device work, so fusion is not assumed to win. Its 243 pipelines span 18.186 ms, with 13.612 ms kernels and 4.574 ms inter-kernel gaps. The simultaneously preceding parallel scan change reduces offset device total to 0.937 ms across all 347 scans. Keep/revert awaits the same-pod adjacent-commit comparison.

### Folding publication sizes into fresh counts (pod timing candidate)

Analysis now runs while the fresh count is still resident. Initial seals share the sort's unique-count download; nonempty admissions share subtraction's fresh-count download. Candidate count bounds the metadata grid, while the kernels read the actual count and skip unused blocks. One 16-byte D2H returns fresh rows and exact payload words. Only metadata can retain bounded spare capacity; payload allocation remains exact. The first packed publication is prepared this way; later geometric-chain merges retain their own size readbacks.

All **26 tests** and the real-trace subtraction/content/prefix oracles pass. Added cases exercise initial sealing, a first packed merge, nonmerging adoption at the ratio boundary, duplicate candidates, zero-fresh speculative preparation, zero-word payloads (B=1), and the all-plain control across four word widths.

`profile-r2-fold.json` and `r2-fold-cuda_*.csv` observe **246 preparation attempts, 237 folded size readbacks, and nine speculative no-ops**. Full-profile D2H calls fall **9,881→9,644**; only **110** later-chain standalone size scans remain. Final pool live stays exactly **96,674,304 bytes**; peak is **103,333,376 bytes**, +2,560 bytes versus the prior candidate. The local profile observed no foreign tenant, but throughput acceptance is reserved for the parent's same-pod comparison.

Timing attribution changes with this fusion: initial analysis is inside `packed_sort_ns`, later prepared analysis is inside `packed_subtract_ns`, and allocation/packing remains in seal/merge publication. Compare end-to-end admission, not a publication phase alone. `packed_prepare_analyze_gpu_ns` and `packed_prepare_pack_gpu_ns` expose the preparation intervals (4.262 and 3.825 ms in this diagnostic run); they overlap merge GPU fields when preparation performs a merge and must not be added twice. With this candidate, `--generic-sort` uses the engine sorter, so it cannot fold an initial seal into that sorter's already-returned count; the default three configurations use the new sorter and do fold it.

### Parent-run clean pod acceptance and three-column result

The parent ran both commits on the same RunPod RTX 6000 Ada, **five fresh processes per configuration**, no foreign tenant, and all oracles passed. These are parent-run measurements, not local timings. Exact summaries and admission-class tables are preserved in `pod-4af76a9c-summary.json` and `pod-6b3e4861-summary.json`, with their original raw-result locations recorded.

The parent accepted **6b3e4861**: packed admission 185.051→171.936 ms, merge 46.195→39.099 ms, seal 10.199→8.944 ms, unchanged bytes. Bundles ran sequentially; unchanged plain columns moved by 2–3%, so the whole 13.115 ms admission delta is not attributed solely to the scan. The publication improvement is nevertheless substantially larger than the drift.

Latest **clean accepted** result, B=256, medians [min,max]:

| Metric | Engine existing plain | New admission, plain storage | New admission, packed storage |
|---|---:|---:|---:|
| Admission ms | 203.993 [199.391,219.276] | 152.793 [151.748,153.592] | **171.936 [170.553,180.813]** |
| Fresh preparation ms | 183.474 [179.347,197.085] | 123.092 [122.264,124.569] | 123.893 [122.419,126.292] |
| Seal/adopt ms | 0.688 [0.664,0.701] | 3.164 [3.104,3.175] | 8.944 [8.835,9.735] |
| Merge ms | 20.307 [19.380,21.494] | 26.190 [25.054,27.080] | 39.099 [38.814,45.966] |
| Lookup ms | 18.512 [18.181,18.672] | 22.340 [21.537,22.409] | 32.133 [32.025,32.715] |
| Final pool MB | 269.662 | 269.656 | **96.674** |
| Peak pool MB | 291.989 | 291.941 | **103.331** |

Packed is 32.057 ms faster than the unchanged engine, and 19.143 ms slower than the new plain-storage control. The latter is the honest remaining packing cost, concentrated in merge and seal. The 179 ms target is met by this accepted median, though the upper observed repetition is 180.813 ms. New-plain lookup itself is 3.828 ms slower than engine lookup; packing adds a further 9.792 ms.

The partition/analysis fusion commit and resident-count folding candidate are exactness-checked but still await the parent's clean adjacent-commit timing decisions. Lookup instruction profiling is complete; additional reader changes have not been accepted in round 2. [INFERENCE] Reusing the experimental reader's repeatedly uploaded run-view descriptors is the first targeted candidate for the new-plain lookup gap, before changing fence layout or residual extraction.

## Clean adjacent-commit decisions (RunPod RTX 6000 Ada, 2026-11-15)

Five fresh replay processes per configuration and bundle, no other GPU process, all oracles passed. Bundles ran one after another on one pod; the first bundle of a session is consistently slower in every column (pod warm-up), and absolute times drift by a few percent between bundles. Each replay process measures the unchanged engine path as well, so the stable comparison is the ratio of the experimental admission to the engine admission measured in the same process (median [range] over the five packed-storage processes):

| Session, position | Commit | Packed admission / engine | Packed lookup / engine |
|---|---|---:|---:|
| 20261115T223001, 1 | `6b3e4861` | 0.839 [0.809, 0.859] | 1.697 |
| 20261115T223001, 2 | `c5661db4` (partition/analysis fusion) | 0.854 [0.811, 0.862] | 1.724 |
| 20261115T223001, 3 | `6b3e4861` | 0.849 [0.844, 0.871] | 1.709 |
| 20261115T224542, 1 | `6b3e4861` | 0.841 [0.826, 0.855] | 1.718 |
| 20261115T224542, 2 | `22aeb5f3` (fusion + readback folding) | 0.832 [0.827, 0.846] | 1.703 |
| 20261115T224542, 3 | `6b3e4861` | 0.851 [0.829, 0.864] | 1.712 |
| 20261115T224542, 4 | `22aeb5f3` | 0.854 [0.847, 0.861] | 1.718 |

The plain-storage control of the new admission path is 0.757–0.771 of the engine admission in the same sessions, with lookups 1.17–1.21 of the engine's.

Decisions:
- `c5661db4` (partition/analysis fusion): no measurable gain (0.854 against 0.839–0.851 for its base). Reverted.
- `22aeb5f3` (readback folding on top of the fusion): seal falls from about 8.9 to 4.2–4.7 ms and the packed-minus-plain merge gap from about 13–18 to about 9 ms, but the preparation it moves into the sort/subtract phase costs about as much; total admission is 0.832 and 0.854 against 0.841–0.851 for the base. No measurable gain. Reverted.
- The accepted implementation is `6b3e4861`: packed admission about 0.845 of the engine's, lookups about 1.71, 96.67 MB final and 103.33 MB peak against 269.66 and 291.99 MB.

This pod resolves admission differences of roughly 2% (about 4 ms); smaller effects need more repetitions or a quieter machine. Raw reports: `/data/mini_linq-wt/comp-phase1/target/remote-gpu/20261115T22{1621,3001,4542}*/pod-results/`.
