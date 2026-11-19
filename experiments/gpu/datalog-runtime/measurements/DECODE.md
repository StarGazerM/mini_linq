# Packed-run decoder experiments

## Protocol

RTX 6000 Ada; default-build timing, separately built NVTX instrumentation for Nsight Compute full warm-cache sections (`--cache-control none --clock-control none`). Raw binaries, logs, JSON and counter exports are under `/data/design-notes/doop-memory/packed-run/arch/decode/`. All numbers below were measured for this experiment, not copied from the earlier architecture study. Before/after compute-tenant queries are saved per process; all reported runs were tenant-free. E/P/C mean engine/new plain/new packed. Xalan totals cover ten repetitions over 131 states; scale times are per repetition. Oracles run before measurement. No engine implementation was modified.

## A: storage-specialized readers — kept

Host dispatch selects plain-only kernels when every retained run is plain (including small runs of the packed set). The existing resident descriptors remain unchanged. Plain kernels never carry packed headers, frames, decoder or fallback state. Shared const-specialized bodies avoid two independently maintained scan schedules. Word width remains a compile-time specialization. A new test exercises all four widths through plain-only, packed and mixed-run publication, both serial and cooperative reads. All 29 tests passed in 4.67 s; complete xalan and both 2^28-row replay oracles passed.

NVTX-selected second warmed experimental launch, before → after:

| Workload | Registers/thread P / C | Achieved occupancy % P / C | Device us P / C |
|---|---|---|---|
| 137 shuffled | 64→28 / 64→64 | 64.03→98.20 / 62.89→63.11 | 291.744→281.248 / 342.688→342.432 |
| 90 scan | 80→40 / 80→80 | 41.52→81.88 / 39.25→38.98 | 201.632→136.768 / 265.056→264.608 |
| 112 scan | 80→40 / 80→80 | 48.60→94.10 / 47.30→47.22 | 227.072→194.368 / 529.056→535.968 |

The freshly measured final-architecture scan baseline uses **80**, not 64, registers. The earlier architecture headline must not be substituted for this build's measured allocation.

| Xalan workload | Before E/P/C ms | After E/P/C ms |
|---|---|---|
| Sorted | 17.539/17.689/21.733 | 17.464/16.141/22.128 |
| Shuffled | 39.648/39.294/44.434 | 39.644/37.257/44.801 |
| Mixed | 47.523/47.749/53.765 | 47.575/44.909/54.323 |
| Scan | 59.398/59.587/89.625 | 59.594/56.555/91.785 |
| Membership | 20.344/21.181/27.495 | 20.336/19.664/27.772 |

E shown here is paired with C; P has its own same-process E. Raw JSON retains both pairs. At state 90, 2^28 rows, the plain scan changes **9.3314→5.8992 ms**, against engine **6.6374→6.6514 ms**. State 112 plain scan changes **8.8792→8.5167 ms**, against engine **8.5472→8.5236 ms**. Thus removing packed live state recovers the plain scan gap without changing payload bytes. Packed occupancy and device cost remain essentially unchanged; this lever does not solve packed decoding. No new device allocations or retained bytes.

Fresh full NVTX counter confirmation at 2^28 rows (`base-scale-*-scan-*` versus `a-scale-*-scan-*`):

| Source/configuration | Registers before→after | Occupancy % before→after | Device ms before→after | DRAM peak % before→after | Active instructions/row before→after |
|---|---|---|---|---|---|
| 90 E | 45→45 | 65.76→65.66 | 6.646→6.637 | 45.21→45.28 | 77.36→77.36 |
| 90 P | 80→40 | 37.67→79.54 | 9.344→5.883 | 32.21→51.09 | 97.23→77.97 |
| 90 C | 80→80 | 36.98→36.78 | 10.929→11.026 | 9.39→9.29 | 144.80→151.31 |
| 112 E | 48→48 | 13.89→13.73 | 8.465→8.470 | 56.10→56.06 | 23.51→23.51 |
| 112 P | 80→40 | 9.32→13.72 | 8.872→8.550 | 53.63→55.53 | 28.01→24.66 |
| 112 C | 80→80 | 7.88→7.70 | 15.036→15.438 | 4.69→4.58 | 95.59→112.15 |

The packed kernel's generated instruction count does change with specialization factoring despite unchanged format and algorithm; its small regression is not hidden. State 112 remains severely underoccupied at scale, motivating actual-length scheduling rather than another probe-count cutoff.

### Additional word-width bound specialization — rejected

The `a2-*` candidate used the valid u64-key invariant that residual rows cannot exceed 64 bits to compile away the wide fallback. Registers remained **64 for prefix / 48 for membership**. State-137 sorted/shuffled/membership device us changed **74.368/344.768/29.856 → 73.856/377.664/31.360**; state-90 2^28 shuffled changed **2.1163→2.4045 ms**. No occupancy benefit justified the regression, so the explicit condition was removed. All candidate replay oracles passed. These measurements predate the required SIMT/RCCL design gate; no kernel work continued before that notes commit.

## B: block-local 32-bit scan extraction — rejected

Kept row-major storage, hoisted the payload base and row-bit stride, narrowed block-local offsets to u32, and selected a u32 extraction loop once per field when its width was at most 32. Wider fields retained the exact u64 reader. No layout/admission changes; retained bytes remain 96,710,144 at xalan size (P 269,691,648; E 269,729,792). Raw artifacts are `b-*`; the rejected implementation was removed.

| Packed NVTX scan | Before → candidate |
|---|---|
| 90 active predicated instructions/row | 408.6→394.5 |
| 112 active predicated instructions/row | 1021.6→1010.4 |
| 90 device us; occupancy %; registers | 264.608→266.240; 38.98→39.06; 80→80 |
| 112 device us; occupancy %; registers | 535.968→537.792; 47.22→47.22; 80→80 |
| 90, 2^28 scan ms | 10.9826→10.9527 |
| 112, 2^28 scan ms | 15.6001→15.3077 |

Instructions use `smsp__inst_executed.sum × smsp__thread_inst_executed_pred_on_per_inst_executed.ratio / rows`. These counts include range searches and the retained partition schedule, unlike the earlier unpartitioned baseline. The small arithmetic saving did not reduce the selected kernel time or register allocation. [INFERENCE] Extra split u32 loads offset the cheaper shifts. No convincing scan win, so the extra width branch and duplicate loop are not retained. All candidate replay oracles passed. The no-format-change option was tested; column-major miniblocks were not represented as tested.

## C: independent residual pivots — rejected

Tested four-way in-block narrowing: load three independent row-major residual pivots before choosing the next quarter, then finish with binary search. Fences, header/frame layout and retained bytes were unchanged. This reduces dependency rounds without adding an index, but increases decode work. Raw artifacts `c-*`, paired counter baseline `a-137-*-pivots`; all replay oracles passed. Removed after the measured regression:

| State 137 packed | Binary → four-way |
|---|---|
| Sorted device us; warp instructions M | 74.432→95.296; 63.397→86.682 |
| Shuffled device us; warp instructions M | 342.432→453.504; 65.154→95.267 |
| Membership device us; registers | 30.080→32.992; 48→62 |
| Shuffled occupancy % | 62.57→58.15 |

Xalan sorted/shuffled/membership synchronized totals: **22.128/44.801/27.772 → 24.574/53.352/28.980 ms**. State 90 at 2^28: **.3397/2.1176/.0545 → .4276/2.8510/.0612 ms**. State 112 at 2^28: **.0276/.1590/.0603 → .0329/.1981/.0819 ms**. The added pivot loads and instructions overwhelm the shorter dependency chain. This rejects this concrete no-index implementation, not an untested colocated-header/separator format.

## D: actual-length task scheduling

The implemented scheduler computes every run/probe interval once, forms exclusive row offsets in 256-entry tiles, scans the tile totals, and uses a two-level load-balancing search to map fixed 1024-row tasks to warps. Tasks scan contiguous portions of the original packed payload; no run is unpacked. Per-probe wrapping checksums are combined with u64 atomics. Temporary pooled storage is `24 × runs × probes + 8 × (ceil(runs × probes / 256) + 1)` bytes, in addition to the existing output. No persistent layout/index bytes are added.

### Uniform warp-per-span mapping — rejected

The first implementation iterated short spans serially within each warp. It passed all replay oracles and improved 2^28 packed scans to **5.7841 ms (90)** and **2.7384 ms (112)**, but xalan scans regressed to **1066.010 ms P / 1617.031 ms C**. State-90 task-kernel time was **12.409 ms**, despite only **290.7 total active instructions/row** across its three kernels. Short ranges serialized the warp instead of filling its lanes. Raw artifacts: `d-*`.

### Lane mapping for short spans — insufficient alone

When a contiguous span has fewer than 32 rows, each lane locates and processes one atom; long spans retain the block-local projected decoder. The existing one-thread policy for batches averaging fewer than two rows/probe is preserved. State-90 pipeline device time falls to **126.688 us** (24.992 bounds + 3.200 offsets + 98.496 tasks); state 112 is **55.584 us** (13.056 + 1.856 + 40.672). Total active instructions/row are **200.8 / 132.3**, respectively, versus the A baseline **408.6 / 1021.6**. Task kernels use **64 registers**, versus the old scan's 80.

At 2^28 rows, E/P/C scan ms are **6.6903/6.8490/5.1606 (90)** and **8.5348/5.1305/2.1994 (112)**. However xalan P/C totals are **75.297/98.883 ms**, versus A's **56.555/91.785 ms**: additional bounds/offset launches and scratch allocation do not amortize on short-fanout batches. All 29 then-current tests passed in 4.70 s, including reversed/empty ranges, multiple runs, and publication invalidation. Raw artifacts: `d2-*`.

### Amortized actual-length scheduling — kept

Select the general scheduler at average fanout ≥8 for packed/mixed storage and ≥128 for plain-only storage; retain the existing short-fanout policy below those points. These thresholds amortize the additional launches, not a probe-count-dependent partition count: the actual scheduler still distributes the prefix sum of measured interval lengths, including overlaps, and scaled state 112 no longer loses parallelism when probe count exceeds 1024. Bounds and task buffers are pooled, and output zeroing/three launches are charged in synchronized timing.

Three clean interleaved repetitions, all oracles, and **30 tests in 4.85 s** pass. New regression coverage spans 257 prefix tiles, 1024-row task boundaries, empty/short/long overlapping ranges, four word widths and both storage modes.

| Scan | A E/P/C ms | Kept E/P/C ms |
|---|---|---|
| Xalan, ten repetitions | 59.594/56.555/91.785 | 59.60/56.625/79.263 |
| 90, 2^28, per repetition | 6.6514/5.8992/10.9826 | 6.6580/5.9087/5.1812 |
| 112, 2^28, per repetition | 8.5236/8.5167/15.6001 | 8.5218/5.1202/2.1944 |

Final packed NVTX pipeline sums (all three kernels) are **125.952 us / 200.8 active instructions per row** for xalan state 90 and **55.488 us / 132.3** for state 112. At 2^28 rows they are **5.208864 ms / 134.6** and **2.185856 ms / 123.4**. Task-kernel occupancy rises from the old packed scan's **36.78→58.85% (90)** and **7.70→58.56% (112)**; task registers fall **80→64**. In state 112 the scheduling improvement matters more than instruction count (old 112.2 versus new pipeline 123.4 instructions/row). Plain state-112 task occupancy is **90.73%**, using 40 registers. The retained layout and admission are unchanged. Raw final artifacts: `d3-*`.

The scheduler follows the atom/tile separation of [Osama et al., A Programming Model for GPU Load Balancing](https://arxiv.org/html/2301.04792v1). The uniform and unamortized variants were removed; only the measured dispatch remains.

## E: charged block-local probe ordering — rejected

Implemented a 256-probe shared-memory bitonic sort, preserving each probe's original ordinal, loading its matching upper endpoint and scattering the result back to that ordinal. All sorting, barriers, shared-memory traffic and scatter are inside the measured kernel. Shared storage is `256 × (word_bytes + 8)` bytes per CTA, with no global allocation or retained-byte change. Both experimental plain and packed were measured, and all xalan/scale oracles passed. Raw artifacts `e-*`; the implementation was removed.

| Shuffled lookup | P before→sorted-in-kernel | C before→sorted-in-kernel |
|---|---|---|
| Xalan total ms | 37.257→48.769 | 44.801→61.075 |
| 137 NVTX device us | 281.248→357.472 | 342.432→496.576 |
| 137 registers/thread | 28→32 | 64→64 |
| 137 occupancy % | 98.20→96.12 | 63.11→65.72 |
| 90 at 2^28, ms/repetition | 3.7588→3.3702 | 2.1176→1.9591 |
| 112 at 2^28, ms/repetition | .1596→.1806 | .1590→.1674 |

The reorder cost overwhelms the cache-resident locality improvement. State-137 long-scoreboard share falls to 57.94% P / 71.83% C, but total instructions and time rise. Beyond-cache state 90 does improve; retaining the algorithm universally would nevertheless regress the required xalan and state-112 workloads. It is rejected overall, rather than presenting the reordered search without charging the sort.

## RCCL migration

### Alternating-buffer radix histogram — fixed upstream branch

RCCL commit `d156455d9c68b7c130d94eeec48c369a3a7db917` selects the current DoubleBuffer keys for histogram generation in both 32/64-bit keys-only and pair paths. Its focused regression covers ascending/descending order, stable values, 2304/2305/4864/4865/8193 rows and two odd-pass sorts with an intervening current-key rewrite. The GPU CPU-oracle probe passes; memcheck reports zero errors.

Mini_linq now carries the selector across lane gathers instead of rebasing each radix call. All 30 packed-run tests passed in 4.80 s against the patched library; the real state-86 reproducer and its read oracles passed. The local Cargo path override is intentionally uncommitted. Neither `/data/rccl` working files nor `/data/rccl-wt/pool-workspaces` were edited; no branch was pushed.

### Checked expansion and fused sums — RCCL cutover

RCCL commit `a628a5ec358596c123d2e780a6b8852920c94461` adds pooled, stream-checked range workspaces, stable pair expansion, explicit count publication, and fused wrapping-u64 interval sums. Sync/async Thrust calls retain endpoint/output/workspace borrows. Prefix preparation reuses RCCL's u64 block scan, and task reduction reuses `warp_reduce`; no private prefix-sum implementation remains in the balanced packed-run path. Mini_linq supplies lowered interval/checksum ranges and preserves its independent reversed-range endpoints while normalizing only the work count.

RCCL's CPU-oracle example covers 0/1/31/32/33/255/256/257/65537 intervals, overlapping/empty ranges, one million-match range plus 10,000 singletons, lazy projections, strided outputs and completion drop. Five isolated device faults (reversed ranges, within-/between-tile overflow, insufficient capacity, value OOB) and host shape/workspace failures are rejected. All ten Thrust doctests pass; memcheck has **0 errors**, racecheck **0 hazards**. Device validation faults can invalidate a CUDA context, matching the existing permutation-range error contract; this is documented, not hidden.

The generalized form is **not free**:

| Packed scan pipeline | Private D3 → RCCL | Active thread instructions/row private → RCCL |
|---|---|---|
| Xalan state 90, device us | 125.952→152.736 | 200.8→206.9 |
| Xalan state 112, device us | 55.488→61.216 | 132.3→115.4 |
| State 90, 2^28, device ms | 5.208864→5.374816 | 134.6→136.8 |
| State 112, 2^28, device ms | 2.185856→2.181632 | 123.4→108.7 |

Task registers remain **64**. Achieved occupancy private→RCCL is **30.58→28.41% / 40.39→38.93%** at xalan states 90/112 and **58.85→58.19% / 58.56→58.15%** at scale. The checked slice/index paths and different shared/warp collective implementation are additional mechanisms; [INFERENCE] they contribute to the small-cache penalty, but these profiles do not isolate their individual costs. Whole-xalan packed scan rises **79.263→81.912 ms** (3.34%) versus the private prototype. The library cutover is retained as the requested reusable mechanism, **not advertised as another speedup**. It still improves the original measured 89.625 ms packed scan and both largest scales.

All **30 mini_linq tests pass in 5.11 s**, including prefix/task boundaries and four word widths. Three fresh tenant-free full replays per storage configuration and three per largest scale pass every oracle. The temporary allocation formula is unchanged, apart from allocator alignment; no decoded whole-run buffer or persistent index was added. `decode-counters.json` (about 63 KB) contains normalized stage counters and the following paired medians; bulky artifacts remain outside git.

## Delivered checkpoint: default-build E/P/C tables

Xalan values are milliseconds for ten repetitions over 131 states. E is paired with C; each P ratio uses the E measured in its own process. Ratios are medians of same-process ratios, not ratios of independent medians.

| Workload | E ms | P ms (P/E) | C ms (C/E) |
|---|---:|---:|---:|
| Sorted prefix | 17.6399 | 16.2179 (.9232) | 21.7291 (1.2318) |
| Shuffled prefix | 39.8692 | 37.3201 (.9374) | 44.5492 (1.1175) |
| Mixed prefix | 47.9264 | 45.0103 (.9425) | 53.9821 (1.1268) |
| Scan | 59.6731 | 57.3220 (.9582) | 81.9120 (1.3714) |
| Membership | 20.5242 | 19.6666 (.9581) | 27.4521 (1.3375) |

Largest-scale values are milliseconds per repetition, all three configurations in one process.

| Source/workload | E ms | P ms (P/E) | C ms (C/E) |
|---|---:|---:|---:|
| 90 sorted | .5493 | .5488 (.9999) | .3394 (.6186) |
| 90 shuffled | 3.8091 | 3.7524 (.9851) | 2.1094 (.5542) |
| 90 mixed | 6.5355 | 6.4398 (.9858) | 3.5981 (.5506) |
| 90 scan | 6.6493 | 5.8982 (.8881) | 5.3438 (.8034) |
| 90 membership | .1041 | .1065 (1.0194) | .0547 (.5228) |
| 112 sorted | .0216 | .0201 (.9460) | .0276 (1.2738) |
| 112 shuffled | .1654 | .1608 (.9767) | .1611 (.9778) |
| 112 mixed | .3839 | .4078 (1.0666) | .2390 (.6245) |
| 112 scan | 8.5322 | 5.1638 (.6056) | 2.2133 (.2597) |
| 112 membership | .1540 | .1508 (.9787) | .0617 (.3999) |

Retained E/P/C bytes: xalan **269,729,792 / 269,691,648 / 96,710,144**; state90 2^28 **2,147,483,704 / 2,147,483,712 / 630,745,528**; state112 2^28 **4,294,967,352 / 4,294,967,360 / 643,309,764**. Maximum observed pool peak is **4,938,277,632 bytes**. Format and packers have not changed, so there is no new-layout admission claim.

## Remaining scope and measured limits

- **The expanded brief is not complete:** the subsequent RCCL FOR range/packer, three-decoder and compact-loop comparisons, geometry bytes, and admission measurements are recorded in [LANES.md](LANES.md). The tested interleaved integrations failed the keep gate and production retains row-major storage. The proposed self-contained shared-memory tile design, exact load-source attribution, and search/admission/merge migration remain unfinished; the new primitive is not advertised as a production speedup.
- Lookup gaps persist at xalan size: C/E **1.2318 sorted / 1.1175 shuffled / 1.3375 membership**. The unchanged lookup-family profiles identify dependent cached loads and scalar decode; no unmeasured interleaved improvement is claimed.
- Packed task scans now reach **33.12% / 34.60% DRAM peak** at the largest states 90/112, with **71.11% / 65.58% long-scoreboard PC share**, versus the original full scan's much lower throughput. Bounds still cost **2.202 ms** of state90's **5.375 ms** profiled pipeline; reducing that search/decode work remains consequential.
- Plain parity is not universal: largest state112 mixed probes remain **1.0666x E**, and state90 membership **1.0194x E**. The plain scan register gap was removed; do not extrapolate that to every lookup.
- RCCL's strict standalone reference/skew-throughput gates and native SM86 execution were not run. No claims are made for those gates.
- The worktree intentionally retains uncommitted `experiments/gpu/Cargo.toml` path overrides and corresponding `Cargo.lock` source changes. Build against `/data/rccl-wt/packed-primitives` until its commits are pushed and the pin is updated.

Reproduce with the build/lock/tenant protocol in [ARCHITECTURE.md](ARCHITECTURE.md#reproduction). The RCCL scan's NVTX kernel filter is `regex:^expand_(bounds|offsets|reduce)`; select the second warmed three-kernel group with `--launch-skip 3 --launch-count 3`. Xalan experimental occurrence is 2; scaled packed occurrence is 3. Keep default-build timing separate from NVTX-enabled counter runs.
