# Coalesced row-major tile fetch study

## Register pressure and occupancy — priority study

The user reprioritized this work ahead of further tile analysis. Tile WIP is preserved on `codex/packed-run-tile-wip` (`f2fad327bf`) and RCCL `mini-linq/packed-primitives-tile-wip` (`11b1cacbf0`); this branch uses the original format and decoder.

**[PREDICTION, before occupancy-control implementation]** Reserve 0, 32 KiB and 48 KiB of unused dynamic shared memory per CTA without touching kernel instructions, register allocation or input. For 256-thread CTAs, 32 KiB permits at most three CTAs/SM (24 warps), 48 KiB at most two (16 warps), versus original 32 packed/48 plain register-limited warps. With long-scoreboard latency dominant, throughput should scale no faster than resident warps: packed 32→24 predicts up to 1.33× slowdown, 32→16 up to 2×; plain 48→24 up to 2× and 48→16 up to 3×. Conservative time forecasts are +20%/+60% packed and +50%/+120% plain, because cache reuse, issue saturation and tails weaken linear scaling. Measure state-137 shuffled lookup/membership and both 2^28 scans for plain and packed. A shared-memory reservation can also change L1 carveout, so L1-miss traffic is a mandatory confound check.

Register budget: 65,536 32-bit registers **per SM**, shared across resident warps; 48 warps/SM and 12/scheduler maximum. Ignoring allocation rounding, 256-thread CTAs suggest 51/42-register targets for five/six CTAs. The actual controls allocate **48/40** registers to achieve those steps; observed rounding includes 28→32, 54→56, 78→80 and 102→104. **64 is the design ceiling.** u64 bases/checksums/pointers/bit cursors each take two registers. Do not infer bank/read-port conflicts: no direct counter exists here. All stall categories, issue/pipe utilization and local load/store sectors are exported.

Register-allocation controls will use `launch_bounds(256,5)` then `(256,6)` on packed lookup/membership and the balanced reduction only, leaving storage and decoded values identical. These ask the assembler to fit five/six CTAs; report actual used/rounded registers rather than assuming an exact 51/42 cap. [PREDICTION] With no added spills, 32→40/48 resident warps offers at most 1.25×/1.5× throughput; forecast 10%/15% improvements for latency-bound packed lookup and 5%/10% for scans. Additional local-memory traffic can reverse those gains.

`nvdisasm --life-range-mode count` on this study's original embedded cubin finds peak live GPR counts **62** (state-137 packed lookup), **48** (membership), **51/54** (state-90/112 reductions), against allocated kernel register counts **64/48/64/64**. The original membership specialization is **48**, not 64 registers. Lookup already contains three static LDL/STL instructions and membership nine; both have u64 divide calls. The scan task retains u64 scheduler positions/bounds/checksum state across runtime divide/remainder calls. Peak live counts are static compiler/dataflow evidence, not dynamic register-file bank-conflict counters. Raw liveness output: `original-live-registers.sass`.

The five-CTA assembler control emits **48/48/48/48** registers for those same four kernels: savings **16/0/16/16** without changing expressions or layout. Text sizes change **23,808→24,320**, **16,000→16,000**, **26,368→27,136**, and **31,616→32,768** bytes. Runtime spill cost and occupancy must still decide whether that allocation is useful.

### Same-code occupancy controls: default-build timings

Unused dynamic shared reservation is the only varying input to the same binary. State-137 entries are microseconds/call; scaled entries are milliseconds/call. The state-90 plain scan uses `storage_plain_scan`, not the balanced reduction; its launch contract and reservation were included in `occupancy2`, and the initially untargeted plain scan profiles are excluded.

| Workload/storage | 0 KiB | 32 KiB | 48 KiB |
|---|---:|---:|---:|
| 137 shuffled / packed, us | 369.209 | 595.373 | 565.316 |
| 137 shuffled / plain, us | 290.387 | 330.354 | 337.356 |
| 137 membership / packed, us | 35.436 | 58.141 | 50.104 |
| 137 membership / plain, us | 24.964 | 33.524 | 29.389 |
| 90 scaled scan / packed, ms | 5.35131 | 6.52680 | 7.08142 |
| 90 scaled scan / plain, ms | 5.90992 | 10.41788 | 14.70375 |
| 112 scaled scan / packed, ms | 2.1976 | 2.8056 | 3.3174 |
| 112 scaled scan / plain, ms | 5.1591 | 5.1392 | 5.1125 |

The completed zero-reservation profiles measure packed shuffled lookup at **64 used/64 allocated registers**, **32 theoretical/30.518 achieved warps**, and **90.83% long-scoreboard samples**. Plain lookup is **28/32 registers**, **48/47.073 warps**, **95.75% long scoreboard**. Membership is **48/48 registers, 40/26.821 warps** packed and **23/24 registers, 48/27.267 warps** plain: achieved residency is already similar despite different theoretical limits. The nonmonotonic 32/48-KiB lookup/member timings must be explained with the pending residency/cache counters before inferring an occupancy-only benefit.

The counters expose the shared-memory control's confound: reservation changes the SM shared-memory carveout **16→100 KiB**, reducing L1 capacity. Packed lookup's L1-miss bytes rise **921.4→1458.2 MB** at 32 KiB reservation. Thus the 0→32-KiB slowdown is not an occupancy-only effect. The 32/48-KiB controls have the same 100-KiB carveout: lookup **24→16 warps actually improves 582.752→549.568 us**, while packed scan tasks slow **4.156→4.834 ms (90)** and **2.669→3.206 ms (112)**. Plain state-112 scan remains **4.960→4.959 ms**, despite 24→16 warps and 95.15→93.66% long-scoreboard samples. Long scoreboard alone does not establish a useful occupancy optimization.

**[PREDICTION, before corrective controls]** To hold carveout at 100 KiB while restoring the original register-limited warps, reserve 23 KiB per CTA for 64-register packed kernels (`(23552+1024)*4=98304` bytes), 18 KiB for 48-register kernels (`(18432+1024)*5=97280`), and 15 KiB for six-CTA plain kernels (`(15360+1024)*6=98304`). The driver overhead is measured at 1024 bytes/CTA. These cases should retain 32/40/48 warps respectively without changing SASS or L1 capacity relative to the 24/16-warp controls. Verify the profiler's actual carveout before interpreting throughput.

### Fixed-cache occupancy-to-throughput result

All following profiles confirm **100 KiB shared carveout** and unchanged registers/instructions. Each cell is **theoretical/achieved warps; task us; billion items/s** (probes for lookup/membership, rows for scans).

| Kernel/storage | Original register-limited residency | 24-warps cap | 16-warps cap |
|---|---:|---:|---:|
| 137 shuffled packed | 32/30.74; 641.824; 1.908 | 24/23.42; 582.752; 2.102 | 16/15.61; 549.568; 2.229 |
| 137 membership packed | 40/28.02; 46.624; 2.765 | 24/23.14; 56.512; 2.282 | 16/15.38; 47.424; 2.719 |
| 90 scan packed | 32/27.27; 3374.432; 79.550 | 24/20.40; 4155.584; 64.596 | 16/13.99; 4834.400; 55.526 |
| 112 scan packed | 32/27.90; 2155.360; 124.543 | 24/20.85; 2669.216; 100.567 | 16/14.58; 3206.080; 83.727 |
| 137 shuffled plain | 48/44.67; 441.504; 2.774 | 24/23.79; 335.072; 3.656 | 16/15.79; 342.464; 3.577 |
| 137 membership plain | 48/28.13; 29.376; 4.389 | 24/23.30; 36.000; 3.582 | 16/15.38; 30.624; 4.210 |
| 90 scan plain | 48/35.08; 6555.232; 40.950 | 24/18.59; 10414.176; 25.776 | 16/12.52; 14677.216; 18.289 |
| 112 scan plain | 40/33.18; 5013.600; 53.541 | 24/21.36; 4960.256; 54.117 | 16/14.64; 4958.720; 54.134 |

**Decision:** reject a uniform occupancy-only speedup model. At constant cache capacity, more resident lookup warps increase traffic: packed 16→32 raises L1 misses **1394.0→1556.1 MB**, plain 24→48 **921.6→1088.8 MB**, while throughput falls. [INFERENCE] Concurrent working-set/cache contention and latency, rather than an insufficient number of resident warps alone, matter here. This is not a register-bank conflict claim. The original 0-KiB control's larger L1 is materially faster than all these fixed-small-L1 lookups, so real register caps must be tested at that original configuration.

Membership's grid has **504 CTAs on 142 SMs**, only **28.39 warps/SM averaged over the whole grid**, regardless of a theoretical 40/48-warp ceiling. A three-CTA cap admits 426 CTAs then a sparse 78-CTA tail; a two-CTA cap admits 284 then 220. [INFERENCE] This wave/tail effect explains why the 16-warp case can outperform the 24-warp case; raising the theoretical ceiling cannot manufacture more work.

Packed scan throughput rises **1.43×/1.49×** when theoretical warps double 16→32, below the ideal 2×. Plain state 112 instead measures **861.39 GB/s physical DRAM traffic** at its highest residency and essentially unchanged time down to 16 warps. Thus high long-scoreboard share does not imply that occupancy explains the scan bandwidth gap. All sixteen PC-sampling stall categories, ALU/FMA/LSU and issue utilization, register allocation/limiter metrics, and local-memory sectors are retained in the `occ*.csv` exports.

### Live-state elimination experiments

Each source control starts from the original decoder, not from another control. These are **compiled register counts**, not additive attribution of individual source variables: the allocator can spend a shorter lifetime on fewer spills or a different schedule. The corresponding runtime-counter controls are collected separately.

| Group/control | Lookup registers | Membership registers | Scan 90 / 112 registers | Saved versus original |
|---|---:|---:|---:|---|
| Original | 64 | 48 | 64 / 64 | — |
| Remove impossible >64-bit residual-key path when `W::BITS<=64` | 64 | 48 | 64 / 64 | 0 everywhere |
| Specialize a single run, eliminating multi-run loop state | 64 | 54 | unchanged | 0 / **−6** |
| Make bound search non-inlined, separating descriptor/search live ranges | 78 | 58 | unchanged | **−14 / −10** |
| Narrow block-local bit cursor and row count to u32 under proven `rows<=65536`, `bits<=512`, `stride<=256` guards; keep exact wide fallback | unchanged | unchanged | 76 / 75 | **−12 / −11** |
| Remove per-column checksum/frame hoisting; decode a row then checksum it | unchanged | unchanged | 64 / 64 | 0 |
| Five-CTA allocation target | 48 | 48 | 48 / 48 | 16 / 0 / 16 / 16 |
| Six-CTA allocation target | 40 | 40 | 40 / 40 | 24 / 8 / 24 / 24 |

The impossible-wide-path control shrinks lookup/membership text **23,808→15,232 / 16,000→11,648 bytes**, but does not cross a register-residency threshold. The narrower cursor adds a guarded path and changes allocator choices, increasing rather than decreasing registers. Moving search out of line similarly introduces call-boundary live state instead of making it free. The per-row checksum control removes loop-invariant bases/widths/masks/weights across iterations but still uses 64 registers, and sacrifices metadata amortization. No source control is retained merely because its local variables look smaller.

Runtime profiles confirm the allocator tradeoff (task time, not full replay):

| Control/kernel | us | Registers used/allocated | Warps theoretical/achieved | Local ld/st sectors | Long scoreboard / no-instruction % |
|---|---:|---:|---:|---:|---:|
| no-wide64 / lookup137 | 379.200 | 64/64 | 32/29.30 | 0/0 | 91.33 / .26 |
| no-wide64 / membership137 | 32.512 | 48/48 | 40/26.81 | 64,468/64,468 | 89.55 / .21 |
| single-run / lookup137 | 342.944 | 64/64 | 32/30.74 | 0/0 | 90.88 / .21 |
| single-run / membership137 | 30.336 | 54/56 | 32/26.74 | 0/0 | 90.37 / .27 |
| bound-call / lookup137 | 318.336 | 78/80 | 24/20.88 | 0/0 | 85.07 / .40 |
| bound-call / membership137 | 30.240 | 58/64 | 32/26.76 | 0/0 | 89.67 / .68 |
| cursor32 / scan90 | 3687.744 | 76/80 | 24/20.41 | 0/0 | 67.11 / 1.16 |
| cursor32 / scan112 | 2463.232 | 75/80 | 24/20.71 | 0/0 | 64.09 / 1.42 |
| row-checksum / scan90 | 3645.952 | 64/64 | 32/27.48 | 0/0 | 53.57 / 1.39 |
| row-checksum / scan112 | 3504.864 | 64/64 | 32/27.02 | 0/0 | 36.78 / 1.77 |

Original local sectors are **306,216/306,216** for lookup, **96,641/96,641** for membership, and **4,194,304/2,162,688** for each scaled task. Removing these accesses is not sufficient: cursor32 still loses residency and time; row-checksum doubles state-112 warp instructions to **2,665,409,566**. Conversely bound-call improves this cached lookup despite *more* registers and fewer warps, demonstrating why register count is a budget, not a standalone objective. These source-isolation controls are not retained as optimizations; the register-cap candidates are evaluated end to end next.

### Register-cap results and decision

Every row below is **E / P / C milliseconds**, default builds, all five read oracles passing. Xalan is the ten-call/all-state sum; scaled entries are per call.

| Case/workload | Original E/P/C | 48-register target E/P/C | 40-register target E/P/C |
|---|---:|---:|---:|
| Xalan sorted | 17.5065/16.0903/22.0830 | 17.8597/16.4400/25.4323 | 17.8080/16.2777/29.0226 |
| Xalan shuffled | 39.7753/37.1903/45.2128 | 40.1874/37.6339/53.2387 | 40.1015/37.5257/58.6219 |
| Xalan mixed | 47.7810/45.0994/54.9707 | 48.2199/45.5852/65.3350 | 48.1267/45.3668/72.8697 |
| Xalan scan | 59.6971/57.0809/82.3382 | 60.0285/57.4286/82.4517 | 59.9391/57.2995/82.6534 |
| Xalan membership | 20.3669/19.5123/28.1637 | 20.7111/19.9813/29.3439 | 20.6044/19.8272/30.9133 |
| 90 sorted | .5473/.5491/.3400 | .5478/.5493/.3371 | .5492/.5493/.3938 |
| 90 shuffled | 3.8016/3.7500/2.1112 | 3.8110/3.7564/2.3423 | 3.8579/3.7508/2.5314 |
| 90 mixed | 6.5288/6.4362/3.6057 | 6.5326/6.4432/3.9763 | 6.5481/6.4391/4.3216 |
| 90 scan | 6.6311/5.8876/5.3268 | 6.6585/5.9166/5.6480 | 6.6595/5.9064/6.1766 |
| 90 membership | .1036/.1068/.0539 | .1036/.1067/.0546 | .1036/.1070/.0548 |
| 112 sorted | .0218/.0199/.0278 | .0220/.0200/.0346 | .0220/.0214/.0399 |
| 112 shuffled | .1655/.1614/.1604 | .1650/.1597/.1403 | .1662/.1608/.1467 |
| 112 mixed | .3739/.4089/.2382 | .3622/.4091/.2843 | .3610/.4084/.2690 |
| 112 scan | 8.5075/5.1594/2.1927 | 8.5191/5.1619/2.3698 | 8.5323/5.1132/2.5887 |
| 112 membership | .1526/.1512/.0615 | .1529/.1519/.0662 | .1539/.1525/.0710 |

| Target/kernel | Task us | Registers used/allocated | Warps theoretical/achieved | Local ld/st sectors, millions | Long scoreboard / no-instruction % |
|---|---:|---:|---:|---:|---:|
| 48 / lookup137 | 408.000 | 48/48 | 40/38.09 | 7.037/5.961 | 91.89/.16 |
| 48 / membership137 | 31.616 | 48/48 | 40/26.74 | .097/.097 | 90.81/.22 |
| 48 / scan90 | 3369.728 | 48/48 | 40/32.26 | 159.048/56.031 | 72.57/.97 |
| 48 / scan112 | 2222.432 | 48/48 | 40/31.21 | 106.410/39.190 | 65.36/1.09 |
| 40 / lookup137 | 455.616 | 40/40 | 48/44.49 | 21.101/8.875 | 92.71/.15 |
| 40 / membership137 | 31.456 | 40/40 | 48/26.63 | .258/.193 | 90.25/.40 |
| 40 / scan90 | 3933.184 | 40/40 | 48/41.35 | 254.190/100.315 | 76.66/1.04 |
| 40 / scan112 | 2437.216 | 40/40 | 48/37.16 | 205.004/68.485 | 64.58/1.16 |

48 registers allocate `48*256=12288` registers/CTA, allowing five CTAs (61,440 registers); 40 registers allow six (also 61,440). The 48-register cases are register-limited; 40-register cases reach the register/warp limit together. Shared memory and the hardware 24-block limit do not bind. The register file is not replicated per scheduler. The 40-register entry texts are **25,088 / 16,128 / 27,776 / 33,024 bytes** for lookup, membership, scan90 and scan112, respectively; all no-instruction shares remain below 1.2%.

**Reject both caps.** The predicted 10–15% lookup improvement becomes **18–32% slower** in the profiled state-137 kernel. Local-load sectors rise **23×/69×**, despite achieved warps increasing **30.52→38.09/44.49**; the >20% forecast discrepancies are explained by measured spill traffic and continued >91% long-scoreboard share, not instruction delivery. Scan local loads rise **25–61×** and both scaled full scans regress. The isolated state-112 shuffled-lookup gain does not justify xalan's 15–33% read regressions. Retained bytes are unchanged. No register-source control or cap is kept; the original format/decoder/allocation policy is restored before returning to the tile experiment.

## Exact original requested-traffic attribution

Per-instruction `inst_executed`, `memory_l1_tag_requests_global` and `memory_l2_theoretical_sectors_global` were collected from this study's original binary. Categorizing all **47 executed global-load PCs** reproduces the hardware request/sector totals exactly: **72,664,629 / 132,760,865 (90)** and **49,205,465 / 88,062,729 (112)**. State 90's predicated schema load at PC `0x4700` executes with no participating lane on half its visits; counting issued instructions as load requests would overcount 418,275 requests. The PC-to-source-class map is preserved in `requested-attribution.json`.

| Reduction source | Requests 90 / 112 | Requested sectors 90 / 112 | Requested bytes / retained byte 90 / 112 |
|---|---:|---:|---:|
| Task/range-prefix metadata | 29,398,959 / 7,036,264 | 35,828,910 / 7,326,079 | 1.817728 / .364419 |
| Run descriptors | 5,737,068 / 1,840,842 | 5,737,068 / 1,840,842 | .291062 / .091568 |
| Block headers | 5,847,120 / 4,047,165 | 14,128,176 / 4,735,452 | .716773 / .235554 |
| Field schema | 7,377,885 / 8,038,986 | 7,377,885 / 8,038,986 | .374306 / .399881 |
| Frame widths | 3,898,080 / 4,047,165 | 9,418,784 / 4,735,452 | .477849 / .235554 |
| Frame bases | 2,367,315 / 2,725,782 | 7,738,648 / 3,303,319 | .392609 / .164316 |
| First payload word | 9,650,234 / 11,956,823 | 31,183,283 / 34,760,289 | 1.582039 / 1.729071 |
| Straddle second word | 8,387,968 / 9,512,438 | 21,348,111 / 23,322,310 | 1.083066 / 1.160115 |
| Fences | 0 / 0 | 0 / 0 | 0 / 0 |
| **Total** | **72,664,629 / 49,205,465** | **132,760,865 / 88,062,729** | **6.735 / 4.380** |

Fences and key-bound searches belong to the separate `expand_bounds` kernel, not the reduction. The bulk decoder loads frames once per block/column, not once per row; the scalar short-atom path loads them per row. Neighbouring lanes' overlapping words are already coalesced within a single instruction. Repeated first-word/sector requests across column passes, row steps, split tile visits and short ranges remain. Payload first+second loads account for only **2.665 / 2.889×** retained bytes; it would be wrong to attribute the full **6.735 / 4.380×** to payload overlap or promise a corresponding DRAM reduction. State 90's range scheduling alone costs **1.818×**.

The profiler does **not** expose L1-miss or DRAM bytes by instruction PC. Kernel-wide original L1-miss ratios are **1.690 / 1.038×**, far below requested-sector ratios. Physical cache lines can be reused by several source classes; assigning kernel DRAM bytes proportionally to requests would fabricate a source attribution. Counter-only straddle/base removal controls therefore measure signed *counterfactual deltas*, not additive ownership of physical traffic. They intentionally do not compute valid checksums: Nsight terminates the first packed reduction before its oracle readback; no correctness/performance acceptance claim is made for them. Bounds and task ownership remain unchanged, and ordinary replay oracles remain enabled in all real decoder runs.

Counterfactual removals use the same first packed scan launch (`packed_scan/`, occurrence 12, reduction only, `--kill yes`), with an original-binary control at that same launch. Positive values below mean bytes removed, normalized by retained representation bytes:

| Removed load class/state | Requested bytes delta | L1-miss bytes delta | DRAM read bytes delta | DRAM write bytes delta |
|---|---:|---:|---:|---:|
| Straddle / 90 | 1.043426 | −.004840 | .000318 | −.000395 |
| Straddle / 112 | 1.133521 | −.003762 | −.000112 | −.000564 |
| Frame base / 90 | .392609 | −.006220 | .001416 | .001542 |
| Frame base / 112 | .164316 | .001724 | .000137 | .000567 |

Frame-base request deltas exactly match the per-PC base class. Removing straddles also changes allocation/scheduling: local-load sectors rise from 4.194 M to **12.801/10.931 M**, and other global loads increase, so its net requested-byte delta is slightly below the original straddle class's 1.083/1.160×. Crucially, eliminating either class does **not** remove meaningful physical reads or L1 misses: the redundancy is overwhelmingly cached for these classes. Signed miss deltas near zero must not be converted to a fictional negative source footprint. Width/header/schema/task loads determine the address stream and cannot simply be removed while preserving it; their exact per-source miss/DRAM ownership remains unobservable with these counters. This is an explicit measurement limit, not an asserted decomposition.

## Revised whole-tile prediction after the register study

**[PREDICTION, before building the revised scheduler/decoder]** Separate the existing row-striped short-range reduction from a long-range pass whose work unit is one tile (32 atoms); use the same checked balanced-range machinery twice, recording original bounds once. This eliminates artificial splits of long tiles at 1024-row task boundaries and keeps short ranges on the original cache/register footprint. The extra preparation/reduction launches and 16 bytes of temporary bounds per interval are charged. Compare shared and prefetched-global decode with the same scheduling and 32-byte layout.

Use **2304 bytes per warp**, 18,432 dynamic bytes/CTA plus measured 1024-byte driver allocation. Five CTAs consume 97,280 bytes and fit the 100-KiB shared allocation; at the **64-register ceiling**, registers instead permit four CTAs = **32 warps**, versus the preliminary shared design's 24. Predict 56–64 used registers, rounded to at most 64, 27–30 achieved warps, and <5% no-instruction share. Keep block-local row/bit positions u32 only after the scratch-size check proves the payload occupies at most 288 u64 words; retain exact wide/oversized fallback. This reuses a necessary size guard rather than adding the failed cursor control's extra dual-path lifetime. Count any oversized fallback coverage explicitly.

For a typical 576-byte tile, three warp u64 fetches/18 sectors divided by 256 rows still give .01172/.07031 fetch requests/sectors per row. Short ranges and scheduling dominate state 90. Predict total reduction requests **.11/.03 per row**, sectors **.30/.10**, L1-miss bytes per stored byte **1.6/1.1**, warp instructions per row **4.0/2.7**, and active lanes **25/29** for states 90/112. At 5e11 warp instructions/s, task issue costs `2^28*(4.0,2.7)/5e11 = (2.147,1.450) ms`. Add approximately 2.2/.14 ms existing bounds and .6/.2 ms extra short/preparation work: predict **5.0/1.8 ms full scans**, **81 ms xalan**. Global decode predicts **5.2/2.2 ms** and **82 ms xalan**, with higher repeated global requests (.24/.17 per row) but no shared-carveout cost. The crossover comparison remains 32/64/128/256 rows, starting at 128.

**Stated physical reference:** the measured plain state-112 task transfers **4,303,713,000 read + 14,955,904 write bytes in 5.013600 ms = 861.391 GB/s** (`occ18432-plain-scan112.csv`). At that measured physical throughput, one pass over original retained state-90/112 bytes costs **.732/.747 ms**; the preliminary 32-byte tiled state-112 footprint costs **.766 ms**. This is an optimistic streaming reference, not a promise that a packed dependent-load kernel attains it. State-90 bounds alone already exceed it; output, temporary traffic and repeated visits are additional. No logical-plain-bytes/percentage-of-peak derivation is used.


## Pre-implementation predictions

These are **[INFERENCE / PREDICTION]**, recorded before changing either decoder or layout. Prior LANES.md measurements are context, not measurements made by this study. Production row-major residual ordering and separate fences remain unchanged. A tile consists of its existing 16-byte column frames immediately followed by its existing payload; tile starts are aligned to 32 bytes, with 128-byte alignment as a control. The existing block header remains the random-access directory. Frame storage moves, rather than being duplicated.

The scan control fetches the same contiguous region into L1 without shared staging. The shared variant uses 4096 bytes per warp (eight warps, 32768 bytes per CTA). Oversized tiles use the existing scalar decoder rather than overflowing scratch. At 100 KiB shared capacity/SM the shared-memory bound is three CTAs = 24 warps, or 50% of Ada's 48-warp maximum; at 64 registers/thread the register limit is four CTAs, so shared memory is the tighter limit. Predict 64 registers and 45–50% achieved occupancy for shared, 64 registers and 58–66% for global. Zero shared allocation is needed for the global control.

A 576-byte typical tile takes three warp-wide u64 load requests and eighteen 32-byte sectors. At 256 rows that is .01172 requests and .07031 sectors/row before scheduling, directory loads, partial tiles and revisits. Retaining the published distribution estimate (not remeasured yet), unfiltered visit multiplicities 3.7647/1.1296 imply .04412/.01324 fetch requests and .26471/.07943 fetch sectors/row for states 90/112. Short intervals must stay row-striped to avoid those extra visits. Neighbouring ranges can visit the same tile; the prediction does not count them as unique physical reads.

| Metric | Shared 90 / 112 | L1-global 90 / 112 |
|---|---:|---:|
| Total global-load requests/row | .12 / .05 | .25 / .18 |
| Requested sectors/row | .25 / .11 | .50 / .34 |
| L1-miss bytes/stored byte | 1.5 / 1.2 | 1.5 / 1.2 |
| Warp instructions/row | 3.3 / 2.6 | 3.5 / 2.8 |
| Active lanes/instruction | 26 / 29 | 25 / 28 |
| Registers/thread | 64 / 64 | 64 / 64 |
| Achieved occupancy | 45–50% / 45–50% | 58–66% / 58–66% |
| Kernel text | <48 KiB | <48 KiB |
| No-instruction stall sample share | <5% | <5% |
| Full scan at 2^28 rows, ms | 4.3 / 1.7 | 4.5 / 1.9 |
| Xalan ten-call scan total, ms | 79 | 81 |

Arithmetic: at a predicted sustained 5e11 warp instructions/s, task issue times are `2^28 * (3.3,2.6)/5e11 = (1.772,1.396) ms`; allowing approximately 2.5/.2 ms for other pipeline stages gives 4.3/1.7 ms. Global's `(3.5,2.8)` gives 1.879/1.503 ms task time, with additional dependency allowance yielding 4.5/1.9 ms. Issue/dependent cached loads, not nominal peak DRAM bandwidth, are predicted to limit both. Xalan's prior dispatch breakdown was 45.777 ms serial + 19.428 ms cooperative-short + 16.284 ms balanced; reducing only balanced by 25% gives 77.418 ms, plus roughly 2% total locality overhead gives 79 ms. The global variant's smaller reduction gives 81 ms. Predictions for 128-byte alignment are the same traffic and +3% time: sector granularity gives no inherent benefit, while padding grows.

Acceptance tolerances fixed before measurement: both scaled scan times must improve by at least 5%, xalan scan and each non-scan workload may regress at most 3%, retained xalan pool bytes at most 1%. Every read/admission oracle must pass. More than 5% no-instruction samples requires a code-size fix, not acceptance. A physical bandwidth reference will use this study's measured DRAM read-plus-write bytes divided by task time, never percentage of peak or logical plain bytes.

## Initial aligned-tile controls, before whole-tile scheduling

These runs were queued before the register-first priority change. Default-build xalan times sum ten calls; scaled times are per call. All recorded before/after tenant queries are empty. The shared32 source passes all 31 packed-run tests in 5.35 s. These use the original row-task scheduler, not the later two-pass whole-tile implementation.

| Variant | Xalan scan ms | State 90, 2^28 ms | State 112, 2^28 ms | Retained pool bytes xalan / 90 / 112 |
|---|---:|---:|---:|---:|
| Original | 82.338242 | 5.3268 | 2.1927 | 96,710,144 / 630,746,112 / 643,310,336 |
| shared32 | 86.891266 | 5.920647 | 2.230122 | 97,058,304 / 630,746,112 / 660,087,552 |
| global32 | 83.331767 | 5.804867 | 2.876883 | 97,058,304 / 630,746,112 / 660,087,552 |
| shared128 | 87.510383 | 5.879113 | 2.206826 | 101,605,376 / 682,073,600 / 703,934,208 |

| Variant/state | Used / allocated registers | Theoretical / achieved warps per SM |
|---|---:|---:|
| Original/90 | 64 / 64 | 32 / 27.938 |
| Original/112 | 64 / 64 | 32 / 28.216 |
| shared32/90 | 80 / 80 | 24 / 19.576 |
| shared32/112 | 80 / 80 | 24 / 19.904 |
| global32/90 | 80 / 80 | 24 / 19.801 |
| shared128/90 | 80 / 80 | 24 / 19.480 |
| shared128/112 | 80 / 80 | 24 / 19.921 |

The queued global32 profiling batch reached its wall-clock deadline while waiting between GPU-lock windows, after state 90 completed; state 112 counters were not collected for that initial control. Raw binaries, source snapshots, full five-workload timings/admission JSON and profiles remain under `/data/design-notes/doop-memory/packed-run/arch/tile/`. The final comparisons and decision follow.

## Final whole-tile comparison

The implementation partitions short rows and long tiles using two calls to the existing checked balanced-range machinery. Each long tile contributes 32 scheduling atoms, so 1024-atom warp tasks contain whole tiles. First/last rows are masked. Overlapping intervals deliberately revisit their common tiles; this cost is included, not divided by unique tiles. Short ranges retain the original row decoder. Bounds are computed once and retained in pooled temporary storage; both preparation/reduction passes are charged.

The 2304-byte/warp version measured 2369 oversized xalan blocks (606,453 rows), so the final control uses **4096 bytes/warp**. Coverage diagnostics now report **zero oversized packed blocks** at xalan and both scaled states; maxima are **430 / 148 / 210 u64 words** respectively. The prediction for this coverage correction, recorded on the WIP branch before building, was time within 5% of the earlier whole-tile control; the final shared times meet that prediction. The generic oversized fallback remains exact but is not used by any packed block in these measurements.

### Global-cache control validation

Disassembly caught an important failed control: ptxas removed a discarded-result `ld.global.ca.u64`. Its archived `tile-global*` timings are **direct global without a surviving fetch**, not evidence for coalesced prefetch. The final **`tile-prefetch`** binary instead issues verified **`CCTL.E.PF1`** instructions from `prefetch.global.L1` over the same lane-strided contiguous tile addresses, then decodes global addresses without shared storage. Hardware prefetch works at cache-line granularity, rather than returning a register value per word. The global-load counters below do not purport to count CCTL instructions. This correction is preserved in WIP commit `5adee9433a`; `tile-prefetch.sass` is the emitted-code evidence.

### Measured crossover

Full shared-scan milliseconds; xalan is the ten-call sum. All four thresholds pass every replay oracle.

| Short-range cutoff, rows | Xalan | 90 at 2^28 | 112 at 2^28 |
|---|---:|---:|---:|
| 32 | 106.4698 | 6.1775 | 2.2656 |
| 64 | 106.0509 | 6.0493 | 2.2648 |
| 128 | 105.1542 | 6.1229 | 2.2393 |
| 256 | 102.5535 | 6.0857 | 2.2104 |

No measured cutoff clears the scan gates. The paired shared/prefetched-global comparison below retains the preregistered 128-row cutoff, not a different best case for each input.

### All five read workloads

Each entry is **E / P / C ms**, default builds. Xalan sums ten calls over all states; scaled entries are per call. E and C are paired; P is the same variant's plain-storage replay.

| Case/workload | Original E/P/C | Shared tile E/P/C | L1-prefetched global E/P/C |
|---|---:|---:|---:|
| Xalan sorted | 17.5065/16.0903/22.0830 | 17.4828/16.1384/21.7148 | 17.5210/16.0447/21.7931 |
| Xalan shuffled | 39.7753/37.1903/45.2128 | 39.7993/37.3528/45.8565 | 39.7996/37.2909/45.9263 |
| Xalan mixed | 47.7810/45.0994/54.9707 | 47.8313/45.2614/55.9420 | 47.8769/45.2093/56.1038 |
| Xalan scan | 59.6971/57.0809/82.3382 | 59.5650/57.1573/105.1542 | 59.6395/57.1723/105.5632 |
| Xalan membership | 20.3669/19.5123/28.1637 | 20.3126/19.6117/28.6265 | 20.3298/19.5576/28.5919 |
| 90 sorted | .5473/.5491/.3400 | .5479/.5490/.3563 | .5476/.5493/.3559 |
| 90 shuffled | 3.8016/3.7500/2.1112 | 3.8180/3.7645/2.2905 | 3.8085/3.7478/2.2816 |
| 90 mixed | 6.5288/6.4362/3.6057 | 6.5461/6.4552/3.8909 | 6.5324/6.4319/3.8956 |
| 90 scan | 6.6311/5.8876/5.3268 | 6.6734/5.9085/6.1229 | 6.6349/5.8924/5.9730 |
| 90 membership | .1036/.1068/.0539 | .1039/.1069/.0580 | .1032/.1060/.0587 |
| 112 sorted | .0218/.0199/.0278 | .0218/.0197/.0282 | .0216/.0197/.0292 |
| 112 shuffled | .1655/.1614/.1604 | .1658/.1615/.1642 | .1659/.1605/.1633 |
| 112 mixed | .3739/.4089/.2382 | .3680/.4075/.2444 | .3651/.4080/.2381 |
| 112 scan | 8.5075/5.1594/2.1927 | 8.5173/5.1598/2.2393 | 8.5560/5.1584/2.3362 |
| 112 membership | .1526/.1512/.0615 | .1526/.1515/.0662 | .1526/.1514/.0662 |

Random access remains O(1), but layout-induced regressions are real: state-90 shuffled/mixed/membership and state-112 membership exceed the 3% tolerance. This is not hidden by xalan's smaller non-scan changes.

### Predictions beside measured counters

Traffic/instructions sum the two reduction kernels and divide by all scanned rows; miss ratios use each variant's actual retained bytes. Active lanes are instruction-weighted across the reducers. Resource/stall entries describe the long-tile kernel.

| Metric, 90 / 112 | Original | Revised shared prediction | Shared measured | L1-prefetch measured |
|---|---:|---:|---:|---:|
| Global load requests/row | .27070/.18330 | .11/.03 | .23609/.06564 | .28824/.14933 |
| Global load sectors/row | .49457/.32806 | .30/.10 | .44535/.13931 | .51350/.29341 |
| L1-miss bytes/stored byte | 1.690/1.038 | 1.6/1.1 | 2.239/1.050 | 1.824/1.016 |
| Warp instructions/row | 5.911/5.003 | 4.0/2.7 | 5.922/3.985 | 6.041/4.120 |
| Active lanes/instruction | 19.50/21.50 | 25/29 | 21.59/24.06 | 21.63/24.18 |
| Registers used/allocated | 64/64;64/64 | ≤64/≤64 | 94/96;102/104 | 94/96;104/104 |
| Theoretical/achieved warps | 32/27.94;32/28.22 | 32/27–30 | 16/14.70;16/14.51 | 16/14.94;16/14.82 |
| Long kernel text bytes | 26,368/31,616 | <49,152 | 31,360/36,864 | 32,128/37,888 |
| No-instruction samples % | 1.03/1.03 | <5 | 1.64/1.51 | 1.90/1.72 |
| Full default scan ms | 5.3268/2.1927 | 5.0/1.8 | 6.1229/2.2393 | 5.9730/2.3362 |
| Xalan default scan ms | 82.3382 | 81 | 105.1542 | 105.5632 |

The shared register budget was **not achieved**. At 96/104 allocated registers, only two 256-thread CTAs fit the 65,536-register file. Shared storage permits three (`3*33792=101376 <=102400` bytes); block-count and warp limits permit 24 and six CTAs. Registers, not shared storage, bind at 16 theoretical warps. Prefetch uses no dynamic shared staging and is likewise register-limited. The long kernels have **zero local load/store sectors**: they avoid spills by holding more live state, which does not make their lower residency free.

| Long-kernel metric | Shared 90/112 | Prefetch 90/112 |
|---|---:|---:|
| Long scoreboard % | 51.96/36.89 | 49.70/43.38 |
| Short scoreboard % | 4.13/6.64 | 1.19/.20 |
| Wait % | 23.92/28.39 | 26.30/28.88 |
| Dispatch stall % | .74/.77 | .88/.78 |
| Active-cycle issue % | 32.94/42.28 | 36.50/42.32 |
| ALU % | 33.72/38.48 | 37.39/38.49 |
| FMA % | 9.02/14.10 | 9.94/14.00 |
| LSU % | 15.33/15.13 | 16.24/13.83 |

Every sampled stall category remains in the raw CSVs, including zero/near-zero categories. Short-scoreboard share is material in shared state 112; it is not a direct register-bank/read-port measurement. Long-kernel no-instruction shares across xalan and scale are at most **2.724%**, below the 5% failure threshold.

### Explaining the forecast misses and reference gap

- Shared full times miss the revised forecast by **22%/24% at scale and 30% at xalan**. Warp instructions miss by approximately **48%/48%**, while allocated registers exceed the 64 ceiling and cut theoretical residency in half. [INFERENCE] Scheduler state, original bounds, generic fallback paths, column invariants and decode state remain live together; narrowing local cursors did not remove that whole-kernel allocation pressure.
- Requests/sectors were underpredicted because short-range scheduling and metadata work remain substantial, especially state 90 (whose original scheduling demand alone was 1.818× retained bytes). Shared state-90 L1 misses are **2.239×**, not 1.6×; splitting passes and staging whole fringe tiles does not preserve the original cache reuse. These counters reject the assumption that all removed requested sectors become saved misses.
- NVTX pipeline sums expose the xalan fixed-work loss: state 90 **.152704→.372032/.368192 ms**, state 112 **.061472→.259872/.265888 ms** (original→shared/prefetch). At scale, shared short/long reduction times are **.936800/2.823904 ms (90)** and **.213216/1.919616 ms (112)**; bounds/preparation are additional.
- Against the measured **861.391-GB/s** streaming reference, shared full scans are **8.36× / 2.92×** the one-representation-pass times (.732/.766 ms). This intentionally optimistic reference excludes necessary bounds, output/temporary traffic and revisits. State-90 original bounds/offsets already cost about 2.21 ms. The measured gap is not an instruction-fetch failure and is not closed by more occupancy alone.
- Shared is only **0.4% faster at xalan**, **2.5% slower at state 90**, and **4.1% faster at state 112** than the verified prefetched-global path: no consistent shared-memory advantage meets the 5% scan-improvement gate.

### Bytes and admission

Retained packed pool bytes are **96,710,144→97,058,304** at xalan (+0.36%), **630,746,112→630,746,112** at state 90, and **643,310,336→660,087,552** at state 112 (+2.61%). Odd-column frame prefixes cause 16 bytes of extra padding per full state-112 tile. The 128-byte-aligned initial variant retains **101,605,376 xalan bytes** (+5.06%), failing the preregistered 1% footprint tolerance. Neither alignment wins the scan gate.

Xalan admission milliseconds, E/P/C:

| Variant/phase | E | P | C |
|---|---:|---:|---:|
| Original sort/subtract | 164.942 | 112.819 | 128.393 |
| Original seal/adopt | .518 | 2.637 | 8.645 |
| Original merge/publication | 17.837 | 23.405 | 40.021 |
| Shared sort/subtract | 172.687 | 113.324 | 113.777 |
| Shared seal/adopt | .501 | 2.654 | 7.705 |
| Shared merge/publication | 18.092 | 23.504 | 38.050 |
| Prefetch sort/subtract | 164.801 | 113.088 | 113.469 |
| Prefetch seal/adopt | .524 | 2.608 | 7.655 |
| Prefetch merge/publication | 18.091 | 23.494 | 37.896 |

### Final decision, exactness and reproduction

**Reject the tile path and both register caps.** No measured shared cutoff improves both scaled scans by 5%; xalan regresses well beyond 3%, and several non-scan workloads also exceed 3%. Keep original row-major storage, original readers and original RCCL allocation policy. No new general tile primitive is shipped into RCCL; its experimental scratch contract remains only on the WIP branch because the integrated mechanism failed the gates.

The full-coverage shared implementation passes **31 packed-run tests in 5.11 s**, including the expanded 31/32/33, 63/64, 127/128/129 and 255/256/257 interval boundaries and overlapping ranges. Every reported full replay passes its CPU oracle at xalan and both scaled states. After restoring production, the seven runtime integration binaries pass **105 tests, zero failures, one pre-existing ignored test**; RCCL's two codec/cooperative CPU tests pass in **6.13 s**, and its checked-expansion/fused-sum and FOR pack/range/search GPU examples both pass.

All accepted timing/profile/test before/after tenant observations are empty; only the local GPU and the prescribed locks were used. Default timing binaries and NVTX binaries remain separate. `timing.sh`, `counters.sh` (six-stage tile pipeline: `PIPE_SKIP=6 PIPE_COUNT=6`), `occupancy-counters.sh`, `traffic.sh`, JSON, counter exports and disassembly are under the raw artifact directory. RCCL example builds require `--features cuda-oxide-example`; GPU executables run under the GPU lock with 100-second test limits.

Production branches are restored to original code with documentation-only study commits. Reproducible tile WIP is on `codex/packed-run-tile-wip` (`f2fad327bf`, `ecee97fdcb`, `5adee9433a`) and RCCL `mini-linq/packed-primitives-tile-wip` (`11b1cacbf0`). The original RCCL branch remains at `d05b07b7c9`; Cargo's local RCCL override and lockfile source changes remain intentionally uncommitted. Exact requested-sector attribution is complete; exact additive per-source L1-miss/DRAM ownership is **not observable** in these exports, and the counterfactual table is explicitly not a substitute for that unavailable measurement.
