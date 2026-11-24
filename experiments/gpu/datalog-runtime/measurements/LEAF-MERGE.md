# Owner-leaf packed merge

**Outcome: exact experimental implementation, not a performance-accepted replacement.** Packed merge rises from 35.856 to 44.962 ms on the real admission history. Retained state falls from 122.556 to 115.726 MB. The matched-history real-read total rises 1.93%; scaled scans regress 9.91% and 10.68%, failing the two-percent reader gate. No cold packed/engine crossover was observed through 2^28 rows.

## Predictions recorded before measurement

The implementation under test retains owner frames, grows leaves and splits above 512 rows without tightening frames. The owner is the larger input. A sealed fixed run keeps its arithmetic row-address fast path; a merged run uses cumulative row positions in headers. Key search still selects a fence and searches within that leaf.

[INFERENCE] The supplied reconstruction (not a run made here) predicts 62.4% owner rows copied without field translation, 26.3% other rows encoded, and 11.3% owner rows reframed. Repeated absorption and conservative full-block envelopes can increase the reframed share. An aligned widening experiment should reduce subsequent widening at some payload cost; it is not yet an aligned-sealing comparison.

[INFERENCE] Before execution, predicted synchronized packed merge totals by admission-size class relative to the same-process baseline are: 1–256: 1.00x; 257–4,096: 1.00x; 4,097–65,536: 0.95x; 65,537–1,048,576: 0.80x; above 1,048,576: 0.80x. These are hypotheses, not measured results. The implementation initially uses three launches (partition/frame planning, prefix sizing, write), not the two-launch target. It reads back two sizing words and allocates the exact payload and header counts rather than reserving schema-width storage. Small plain merges retain one existing launch. First transitions from two plain runs retain the existing four-launch sealing merge.

The first implementation uses a single-thread prefix sizing kernel. [INFERENCE] This may erase the packing benefit at high leaf counts and must be measured rather than described as a speedup. Retained-frame owner rows are copied as bit fragments into a shared code tile; unchanged whole leaves are copied directly. No row key is reconstructed except through the existing fence maintenance and explicit oracle boundaries.

## Measurement record

Raw data is under `/data/design-notes/doop-memory/leaf-merge/`; compact evidence, executable hashes, launch counts and counters are in `LEAF-MERGE.json` under `/data/design-notes/doop-memory/`. The following chronology preserves the predictions and rejected trials. Final tables are below.

### First exact implementation (rejected performance)

Five newly run clean baseline processes measured E/P/C admission at 182.960 / 140.051 / 155.527 ms. A clean first leaf process measured 189.388 / 140.711 / 224.097 ms; packed merge was 102.595 ms, with planning / prefix-sizing / writing intervals 34.451 / 2.917 / 50.393 ms. Final packed state was 119,000,456 bytes. This is a regression, not the retained performance result.

The first measured implementation already replaced the proposed serial prefix with a 256-thread scan. It also removed the old fixed-block packed merge entirely. Plain-to-packed transitions seal the larger plain input, then use the same leaf insertion path. Both the initial seal and leaf insertion currently have size readbacks. The aligned experiment now aligns initial sealing as well as later widening.

The representative write kernel used 128 registers and 758,944 ns; planning used 64 registers and 239,552 ns (kernel-filtered state-90 launch 54; profiler results, not clean timing samples). Active-thread instructions were approximately 1.729 billion for writing and 193.756 million for planning. The largest write specializations spilled 132 store / 144 load bytes per thread at compilation. These costs explain the >20% prediction miss: changing the geometry alone did not remove per-row traversal, serial boundary reductions or excessive live state.

The next trial uses a 32-thread planner, metadata envelope checks before payload work, known-owner block addressing, and outlined translation/co-rank helpers. It does not claim those changes help until measured.

### Test changes

`packed_code_space::disjoint_aligned_blocks_copy_payload_bits` no longer pins four output headers or old `reserved` tags. It checks exact output, total row count, the cap, unchanged owner-leaf payload/frame bytes and every fence. `repeated_leaf_absorption_plain_batches_and_fence_changes` adds repeated plain absorption, frame widening and fence changes, checking every key and bound after every admission. All 31 existing packed-run tests and all seven code-space tests passed in the first implementation. The formal decode registry checker also passed; existing fence and oracle boundaries are unchanged.

### Beyond-cache protocol and predictions

`--state 90|112 --scale-rows N --merge-delta 16|2` builds two disjoint runs by taking every 17th or third sorted real-distribution key for the smaller side. Real keys are replicated using the existing scaled replay's disjoint leading-field offsets. E/P/C execute in the same process, with 256 MiB of nonconstant device writes before each timed merge. Full sorted outputs are compared with the generated host oracle. Reported resident GB/s uses input plus output allocation bytes, not a hardware DRAM-byte counter.

[INFERENCE] The supplied standalone bandwidth results (not run here) put a streaming 8-byte merge near 0.021 ns/row and a 16-byte merge near 0.041 ns/row beyond cache, with 754–785 GB/s reference bandwidth. Given the first leaf counters, the unoptimized implementation is predicted not to cross the plain curve even at 2^28 rows. Untouched-leaf copying should approach streaming speed only when enough contiguous payload words are assigned per CTA; tiny leaves and the fused writer's register allocation can prevent that.

`PACKED_READ_LEAVES=1` reconstructs captured/scaled read runs via a two-thirds owner plus one-third insertion, rather than sealing fixed blocks. It isolates the cost of actual variable leaves on identical keys; it is not a reconstruction of each snapshot's original admission history.

### Reader gate and cap experiment

The 512-row-cap variable read trial measured C193.658 ms versus baseline179.850 ms (+7.68%). At recorded site2625, baseline/variable counters were 68/70 registers, 3.040/3.158 billion active-thread instructions and 4.496/5.204 million global-load requests. The counter kernel interval was 161.632/161.664 us: that one site is not an attribution of the full replay regression.

A four-byte directory entry per 256 rows bounds variable row-address searches. The next clean trial reduced the variable read total to185.275 ms (E153.429, P146.415), still above the two-percent gate. It added137,084 resident bytes over the otherwise identical admission state. This 512-cap trial is not accepted as meeting the reader requirement.

Before the next trial, the cap is reduced to256 and oversized owner intervals are split into balanced equal-size leaves, still retaining their common frame. [INFERENCE] Keeping the maximum in-block search depth at the former256-row level should offset the extra fence and row-directory work; balancing avoids the one-row tails caused by cutting257 rows into256+1. More headers and independently scheduled output CTAs may cost merge time. The cap decision awaits measurement.

The earlier slow writer serialized all splits of a large absorbed interval on one CTA. Scheduling each output split independently reduced merge time to49.743 ms (min/max) /47.791 ms (aligned) in the next single-process trials; min/max state bytes were117,651,384 versus aligned122,886,408. These are intermediate trials, not final medians.

## Design as built

There is one packed merge, including manually sealed small inputs; the legacy re-cutting merge is deleted. The unchanged 32,768-row cutoff keeps normal small admissions plain. Owner selection uses row counts. Initial plain owners are sealed once; a fresh plain smaller input is encoded directly into the owner's frames.

Leaves keep frames and split evenly above 512 rows. The 256-cap experiment was not retained. Headers carry cumulative row positions but stay 32 bytes: the redundant payload-word count is derived from row count and stored row width instead of pushing the encoding tag into a second 32-byte sector. Variable runs additionally retain a four-byte row-directory entry per 256 rows. Fixed sealed runs retain arithmetic row addressing. Strided scans retain a block cursor rather than repeat global row-to-leaf searches.

The packed path has five launches:

1. One other-run fence search per owner leaf, producing insertion intervals.
2. Warp-parallel frame planning. Metadata bounds prove fits when possible; uncertain packed intervals reduce residual extrema. Unchanged owner frames are retained.
3. A parallel prefix scan produces payload offsets and balanced-split counts. Two sizing words are read back for exact allocation. Reserving schema-width payload storage was not chosen; the implementation does not meet the two-launch/no-readback objective.
4. Incoming rows compute their merged positions in parallel. Fitting narrow owner frames compare concatenated codes; wider or widening cases compare source codes in a common frame. Scratch is eight bytes per incoming row, not a decoded row buffer.
5. A 64-thread CTA per output leaf writes packed words. A shared interval descriptor identifies a contiguous owner bit range or an encoded insertion. Kept owner rows are never extracted field by field. Only incoming rows and owner rows in changed frames use code translation. Each output word has one writer; fences use the existing registered boundary.

The rank-position pass replaces the generic serial co-rank and shared-row sorting that dominated the first implementation. Compiled writer registers fell from 128 to 72–80; insertion ranking uses 54–56. Frames, split metadata, positions, payload and reader directories all use the device pool.

Measured Nsight launch counts over all 1,314 merges are 2,628 for E, 1,314 for P, 2,043 for the old C and 2,490 for the new C. The 243 packed merges use five launches; 68 first transitions additionally seal a plain owner with three launches. The other 1,071 merges stay on the original one-launch plain path.

### Copy-only bound and attribution

The bound copies the owner's payload, shifted by 13 bits, into destinations derived from planned output spans (with padding where a shifted owner payload needs more room). It performs no absorption, key comparison or field access. A 256 MiB nonconstant write evicts L2 first; every copied word is checked against the original packed words afterward. On the first 2^26/state-90/1:16 trial it moved 267.158 MB in 417.632 us, **639.7 GB/s**, using 28 registers. This is within 1.23x of the supplied 754–785 GB/s reference [INFERENCE: reference run supplied by Main], so the copy organization is not intrinsically 14x from streaming.

The same trial's old fused writer spent 5.787 ms and approximately 16.79 billion active-thread instructions at 128 registers. Its fitting/absorb subset spent 5.531 ms for 65.252M output rows. The interleaved input has no wholly untouched leaves: the filtered untouched-path launch is skip overhead and must not be advertised as copy throughput. Path-filtered timings include skipped-CTA scheduling and are not additive components of the full launch.

### Matched reader construction and ablations

The acceptance comparison is committed `d8ad470a` fixed-block metadata merge versus this leaf merge, both constructed from identical two-thirds/one-third partitions of each captured/scaled run. This is a matched one-merge construction, not every snapshot's full admission history. Direct exact sealing is a reference, not the post-merge engine baseline.

`PACKED_READ_LEAVES=1` enables that construction. `PACKED_READ_ABLATION=geometry` keeps the leaf geometry but re-encodes original plain input into exact frames. `PACKED_READ_ABLATION=frames` uses fixed 256-row blocks with the inherited leaf-frame envelopes; crossing a leaf boundary unions those envelopes, and can additionally widen them. These are untimed construction-only ablations, never operation-path unpacking. `geometry.rs` reads the original replay input, not reconstructed packed rows.

Reproduce the old side by extracting `git archive d8ad470a` into an isolated directory, applying `leaf_merge_baseline.patch`, retaining the local RCCL manifest/lock override there, and building with the same toolchain. The patch changes only replay construction and row-count reporting. The baseline executable used here is archived outside git as `history-baseline`.

Normal timings never enable `PACKED_MERGE_FRAMES=paths`; that flag repeats the three writer paths separately and reports row populations. `PACKED_COPY_BOUND=1` additionally enables the cold copy bound. Full raw reports, profilers, executable hashes and process checks remain under `/data/design-notes/doop-memory/leaf-merge/`.

## Final measurements

All replay timing processes reported clean before/after GPU checks. Admission and synthetic numbers are five-process medians. Matched-history reads, ablations, aligned-frame comparison and cold scaled merges each use one fresh process; small differences are not confidence bounds. Scaled and synthetic reads contain ten warmed repetitions. Counter intervals are profiler observations, not substitutes for these timing samples.

### Real merge and admission

Classes use the **sum of rows merged in an admission**, including its cascade, not a single kernel's row count. Times are synchronized host milliseconds. E/P/C mean engine, plain-storage control, packed.

| Admission merged rows | E before | P before | C before | E after | P after | C after |
|---|---:|---:|---:|---:|---:|---:|
| 1–256 | 5.281 | 6.531 | 6.585 | 5.291 | 6.550 | 6.561 |
| 257–4,096 | 4.673 | 6.087 | 6.118 | 4.691 | 6.103 | 6.121 |
| 4,097–65,536 | 3.771 | 4.964 | 6.664 | 3.752 | 4.969 | 8.173 |
| 65,537–1,048,576 | 2.996 | 4.046 | 11.878 | 2.974 | 4.043 | 18.027 |
| Above 1,048,576 | 1.048 | 1.871 | 4.668 | 1.048 | 1.875 | 6.047 |

| Quantity | E before / after | P before / after | C before / after |
|---|---:|---:|---:|
| Whole admission ms | 182.960 / 182.533 | 140.051 / 139.150 | 155.527 / 164.197 |
| State bytes | 271,198,552 / 271,198,552 | 271,192,208 / 271,192,208 | 122,555,552 / 115,726,164 |
| Peak pool bytes | 293,549,568 / 293,549,568 | 293,502,208 / 293,502,208 | 131,314,176 / 127,778,816 |
| State bits/row | 86.339 / 86.339 | 86.337 / 86.337 | 39.017 / 36.843 |

The separately rerun old exact-frame policy retained 97,141,096 bytes (30.926 bits/row). The leaf result is 19.13% larger than that exact reference but 5.57% smaller than the old metadata-frame history. Its final payload/header-plus-schema-and-directory/fence bytes are **108,500,696 / 6,297,828 / 927,640**. These are finite full-history results, not an assertion that inherited frames remain as tight as freshly optimized blocks indefinitely.

CUDA-event C intervals, before/after ms:

| Class | Partition | Analysis | Write |
|---|---:|---:|---:|
| 1–256 | 0.630 / 0.630 | 0.645 / 0.646 | 2.360 / 2.382 |
| 257–4,096 | 0.564 / 0.565 | 0.573 / 0.571 | 2.338 / 2.357 |
| 4,097–65,536 | 0.743 / 0.664 | 1.238 / 1.155 | 2.143 / 3.016 |
| 65,537–1,048,576 | 2.376 / 1.305 | 2.639 / 6.586 | 2.931 / 5.774 |
| Above 1,048,576 | 0.693 / 0.365 | 0.712 / 2.136 | 1.850 / 2.199 |
| Independently aggregated total | 5.008 / 3.543 | 5.808 / 11.093 | 11.602 / 15.730 |

The new analysis interval includes prefix sizing; the old interval excludes it. The new write interval includes insertion ranking. Plain merges retain the old event-mark protocol, so their partition/analysis intervals are submission gaps, not partition kernels. Bootstrap sealing, sizing readback and allocation are not all covered by these three intervals.

Measured launch averages and profiler kernel-only milliseconds:

| Class | C launches/merge before / after | E kernel ms before / after | P kernel ms before / after | C kernel ms before / after |
|---|---:|---:|---:|---:|
| 1–256 | 1.000 / 1.000 | 1.332 / 1.332 | 0.960 / 0.960 | 0.960 / 0.958 |
| 257–4,096 | 1.000 / 1.000 | 1.364 / 1.364 | 1.161 / 1.160 | 1.158 / 1.157 |
| 4,097–65,536 | 1.441 / 1.964 | 1.509 / 1.508 | 1.263 / 1.260 | 2.280 / 3.135 |
| 65,537–1,048,576 | 3.495 / 4.753 | 1.391 / 1.389 | 1.386 / 1.380 | 6.670 / 12.027 |
| Above 1,048,576 | 3.750 / 5.042 | 0.554 / 0.555 | 0.999 / 0.996 | 3.173 / 4.319 |

E uses two launches and P one launch per merge in every class. Nsight launch/API correlation was matched to all replay admissions; counts are not estimates from source inspection.

### Rows handled and prediction gaps

These are **writer-phase** populations, in rows; plaintext-only merges are excluded:

| Admission class | Packed output rows | Owner bits copied | Incoming encoded/rebased | Owner reframed |
|---|---:|---:|---:|---:|
| 1–256 | 0 | 0 | 0 | 0 |
| 257–4,096 | 0 | 0 | 0 | 0 |
| 4,097–65,536 | 1,887,292 | 907,388 | 647,638 | 332,266 |
| 65,537–1,048,576 | 25,883,697 | 15,686,848 | 7,360,101 | 2,836,748 |
| Above 1,048,576 | 27,278,434 | 19,716,626 | 6,452,560 | 1,109,248 |
| Total | 55,049,423 | 36,310,862 | 14,460,299 | 4,278,262 |

The actual schedule additionally encodes **2,699,565 initially plain owner rows** during 68 bootstrap seals. Those rows can subsequently appear in the copied/reframed writer columns: do not mistake them for pre-existing packed input. Of the incoming rows, **5,070,907** originate in plain inputs, including retained small runs. Bootstrap counts were reconstructed from recorded admissions and agree with measured seal launches.

Writer shares are **65.960% copied / 26.268% incoming / 7.772% reframed**, versus the supplied fresh-256-block reconstruction's 62.4/26.3/11.3% prediction. Persistent balanced leaves reduce owner-block count to 115,948, retain broader capacity from earlier admissions, and therefore widen less often. The owner-reframe prediction is 31% too high for this geometry.

The first two timing-class predictions were within one percent. Actual times in the three larger classes exceeded predicted times by **29.1%, 89.7% and 61.9%**. The anticipated two-launch benefit was not achieved: rank scratch, five launches, bootstrap sealing, exact sizing/readbacks and frame reduction remain. Faster word copying does not pay for those costs on this admission history.

### Matched real reads and reader gate

Times below are milliseconds; kind rows are separately replayed launches and must not be added to the mixed total.

| Workload | E before | P before | C fixed-history | E after | P after | C leaf-history |
|---|---:|---:|---:|---:|---:|---:|
| Mixed total | 152.245 | 145.716 | 179.116 | 151.916 | 145.117 | 182.567 |
| Membership misses | 19.317 | 18.241 | 19.522 | 19.230 | 18.055 | 19.436 |
| Range misses | 69.893 | 67.697 | 84.080 | 69.772 | 67.538 | 84.785 |
| One-row hits | 119.217 | 113.964 | 138.618 | 118.912 | 113.359 | 141.207 |
| Short ranges | 40.171 | 38.365 | 47.816 | 40.177 | 38.271 | 48.909 |
| Long ranges | 13.561 | 13.036 | 15.996 | 13.562 | 12.979 | 16.023 |

| Construction | C total ms | Snapshot-weighted state bits/row |
|---|---:|---:|
| Committed fixed-block merge, matched history | 179.116 | 27.391 |
| Leaf merge, matched history | 182.567 | 22.913 |
| Variable geometry, exact frames | 182.956 | 22.772 |
| Fixed geometry, inherited envelope union | 177.546 | 24.774 |
| Direct exact sealing, new reader code, reference only | 176.509 | 21.889 |

Bits/row here weight each unique captured snapshot once; they are not the final admission state's bits/row. All recorded bounds and child checksums are verified. The observed mixed-total difference is **+1.927%**; this single-process comparison is borderline, not proof of a universal two-percent bound.

### Synthetic and scaled reads

Synthetic results are E/P/C triples, before → after, in ms:

| Workload | Before E/P/C | After E/P/C |
|---|---:|---:|
| Admission | 4.383 / 3.933 / 4.038 | 4.307 / 3.955 / 3.954 |
| Sorted prefix | 0.463 / 0.484 / 0.486 | 0.468 / 0.485 / 0.488 |
| Shuffled prefix | 0.546 / 0.505 / 0.502 | 0.548 / 0.501 / 0.504 |
| Mixed prefix | 0.546 / 0.496 / 0.494 | 0.549 / 0.494 / 0.495 |
| Membership | 0.611 / 0.508 / 0.508 | 0.611 / 0.509 / 0.509 |
| Range scan | 0.677 / 0.783 / 0.787 | 0.682 / 0.784 / 0.784 |

The 10,740-row synthetic state stays plain. E/P/C state bytes are 368,640 / 215,760 / 215,760; P/C previously used 215,568 bytes. The extra 192 bytes are reader descriptors, not payload. Peaks remain 529,152 / 436,992 / 436,992 bytes.

Scaled read times cover ten repetitions at 268,435,456 rows:

| State / construction | Sorted | Shuffled | Mixed | Membership | Scan | State bits/row |
|---|---:|---:|---:|---:|---:|---:|
| 90 / E control | 5.475 | 38.190 | 65.592 | 1.029 | 66.466 | 64.000 |
| 90 / P control | 5.490 | 37.631 | 64.619 | 1.066 | 58.941 | 64.000 |
| 90 / committed fixed history | 3.494 | 22.778 | 39.113 | 0.450 | 54.642 | 22.217 |
| 90 / leaf history | 3.686 | 22.814 | 39.341 | 0.434 | 60.055 | 20.154 |
| 90 / variable exact | 3.678 | 22.751 | 39.229 | 0.429 | 59.877 | 20.024 |
| 90 / fixed inherited | 3.499 | 23.344 | 40.258 | 0.451 | 60.725 | 22.295 |
| 90 / old direct exact reference | 3.389 | 21.631 | 37.134 | 0.437 | 52.662 | 18.798 |
| 112 / E control | 0.215 | 1.639 | 3.540 | 1.528 | 85.250 | 128.000 |
| 112 / P control | 0.205 | 1.612 | 4.094 | 1.509 | 51.515 | 128.000 |
| 112 / committed fixed history | 0.291 | 1.656 | 2.416 | 0.502 | 21.514 | 22.344 |
| 112 / leaf history | 0.303 | 1.677 | 2.409 | 0.480 | 23.813 | 20.143 |
| 112 / variable exact | 0.300 | 1.667 | 2.417 | 0.479 | 23.765 | 19.991 |
| 112 / fixed inherited | 0.291 | 1.652 | 2.445 | 0.501 | 25.535 | 22.411 |
| 112 / old direct exact reference | 0.278 | 1.603 | 2.378 | 0.489 | 20.548 | 19.172 |

Leaf-state bytes/peak pool bytes are **676,255,744 / 2,636,523,520** for state 90 and **675,871,380 / 3,368,064,768** for state 112. Complete E/P/C bytes and peaks are in the JSON. Construction and reader scratch contribute to these peaks.

**The reader gate fails:** scans regress 9.91% and 10.68% against matched fixed histories; sorted probes regress about 5.50% and 4.08%. Exact-frame variable leaves retain nearly the same cost, so frame inheritance is not the primary cause. The fixed-inherited scan also regresses, showing that the modified reader path matters in addition to geometry.

Matched scan counters identify register pressure:

| State | Registers before / after | Occupancy % before / after | Active instructions/row before / after | Load requests/row before / after | Kernel ms before / after |
|---|---:|---:|---:|---:|---:|
| 90 | 64 / 78 | 58.26 / 42.66 | 105.74 / 108.53 | 0.26990 / 0.28758 | 3.184 / 3.678 |
| 112 | 64 / 78 | 59.12 / 44.01 | 76.59 / 73.93 | 0.19598 / 0.18779 | 2.130 / 2.254 |

The state-112 kernel slows despite fewer instructions and load requests. Added row-location/control live state reduces occupancy. At real site 2625, registers increase 68→72 and load requests 4.557M→5.219M; its counter interval increases 156.928→161.760 us.

### Frame variant decision

| Variant | State bytes | Peak pool bytes | State bits/row | Admission ms | Merge ms | Matched real-read ms | Touched leaves fitting | Owner rows reframed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Min/max, default | 115,726,164 | 127,778,816 | 36.843 | 164.197 | 44.962 | 182.567 | 86.804% | 4,278,262 |
| Aligned, diagnostic | 120,988,492 | 133,765,376 | 38.518 | 160.824 | 41.611 | 185.117 | 97.313% | 763,415 |

Min/max is kept as the default: aligned saves merge work but increases retained bytes 4.55% and makes the matched read total worse. Its owner-reframe share is 1.387%, versus 7.772% for min/max. Both variants preserve exactness; neither makes the implementation pass all performance gates.

### Beyond-cache merge

N is total output rows; delta is other/owner. E/P/C execute in the same process after L2 eviction. Full sorted outputs match the host oracle. C logical GB/s uses `2*N*sizeof(W)`; resident GB/s uses input-plus-output allocations, not hardware DRAM traffic or scratch traffic. Event intervals include submission/allocation gaps. Cold benchmark pool peaks include eviction and oracle scratch, unlike the admission peak table.

| State | N | Delta | E ns/row | P ns/row | C ns/row | C logical GB/s | C resident GB/s |
|---|---:|---:|---:|---:|---:|---:|---:|
| 90 | 2^26 | 1/16 | 0.02224 | 0.02643 | 0.05247 | 304.92 | 92.97 |
| 90 | 2^26 | 1/2 | 0.02267 | 0.03354 | 0.07333 | 218.18 | 71.87 |
| 90 | 2^28 | 1/16 | 0.02142 | 0.02959 | 0.05165 | 309.77 | 94.45 |
| 90 | 2^28 | 1/2 | 0.02346 | 0.04587 | 0.06434 | 248.68 | 81.93 |
| 112 | 2^26 | 1/16 | 0.04369 | 0.04726 | 0.06288 | 508.89 | 78.87 |
| 112 | 2^26 | 1/2 | 0.04919 | 0.05562 | 0.07761 | 412.34 | 68.03 |
| 112 | 2^28 | 1/16 | 0.04291 | 0.05777 | 0.06465 | 494.94 | 76.72 |
| 112 | 2^28 | 1/2 | 0.04645 | 0.07812 | 0.07824 | 409.00 | 67.50 |

There is **no C/E crossover at the tested points**. The state-90 1/16 case improves from the rejected leaf trial's 0.31873 to 0.05247 ns/row, but remains slower than E. The final cold copy-only bound reaches **649.0 GB/s** (411.615 us for 267.158 MB), so field encoding, ranking, metadata/sizing and writer control—not the primitive word shift—remain decisive.

Final cold 2^26/state-90/1:16 counter intervals are partition 57.024 us, frame planning 329.536 us, insertion ranking 293.696 us and full writing 1,638.944 us. Registers are 36 / 80 / 56 / 72; the isolated copy bound uses 27. Compiled specializations use partition 36–46, planning 80–85, prefix sizing 40, rank 54–56 and write 72–80 registers; the ordinary sealing offset scan uses 28. All specialization resources are retained in the JSON.

For the real 2,301,108-row merge, counter-based writer-path costs are:

| Filtered writer path | Output rows in path | Kernel ns/row | Active instructions/row, approximate |
|---|---:|---:|---:|
| Untouched bit copy | 577,409 | 0.02438 | 100.58 |
| Fit/absorb, including copied owner intervals | 1,714,811 | 0.03282 | 286.04 |
| Widen/re-frame, including incoming rows | 8,888 | 2.96670 | 3,723.67 |
| Full writer | 2,301,108 | 0.02573 | 228.60 |

The rare widening row is dominated by skipped-CTA scheduling and cannot be interpreted as an isolated field-rebase latency. Fit/absorb combines encoding and owner copying; it is not a pure cost per inserted row. These non-additive measurements use the same writer binary and register allocation. Untouched-copy payload traffic is about 360.5 GB/s in this filtered real sample; the cold copy-only bound isolates the lower-cost primitive.

## Verification and remaining work

All **31 unchanged packed-run tests and nine code-space tests passed**, including new absorption/split/widening/repeated-merge/ablation checks. Formal checking passed with the same 522 registered decode markers. Full admission, synthetic, matched real-read and ablation oracles passed; both 2^28 scaled states and all eight cold merge cases passed. No engine files, RCCL source, proc macro implementation, IR or plain cutoff changed. The RCCL manifest/lock override remains outside the commit.

The implementation and measurements are complete as an experiment, but **performance acceptance is not achieved**. The scaled reader regressions, five-launch path, bootstrap encoding, exact-sizing readback and remaining compute/control cost prevent calling this a replacement win. No regression is hidden by reverting to the old merge, tightening frames in normal operations, changing the read oracle, or substituting directly sealed exact runs for the matched-history baseline.

### Reproduction

From `experiments/gpu`, after restoring/sourcing the documented local GPU environment:

```sh
source /dev/shm/gpu-env.sh
flock -s /tmp/mini_linq-timing.lock timeout 900 \
  cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- \
  --release -p gpu-datalog-runtime --features cq-gpu-runtime/nvtx --bin packed-replay
R=/data/design-notes/doop-memory/leaf-merge
T=/data/design-notes/doop-memory/real-replay
M=/data/design-notes/doop-memory/wire-in/cursor/census-tree/experiments/gpu/datalog-runtime/measurements
python3 "$M"/code_space_measure.py --processes 5 \
  target/release/packed-replay "$R/reproduce-admission" -- "$T/admission-v2" --admission-only
PACKED_READ_LEAVES=1 python3 "$M"/code_space_measure.py --processes 1 \
  target/release/packed-replay "$R/reproduce-history-reads" -- "$T/admission-v2" \
  --real-trace "$T/joint-v2" --real-reads-only
python3 "$M"/code_space_measure.py --processes 1 \
  target/release/packed-replay "$R/reproduce-cold" -- "$T/admission-v2" \
  --state 90 --scale-rows 67108864 --merge-delta 16
```

The runner owns the exclusive GPU lock and preserves process checks. Add `PACKED_READ_ABLATION=geometry|frames` to the history command for ablations, or `PACKED_LEAF_ALIGNED=1` for the alternative frames. `leaf_merge_summary.py DIR... --snapshot-events "$T/joint-v2/events.jsonl" --output SUMMARY.json` aggregates the timing reports. Counter captures and failed selector attempts are retained outside git; only successful captures are included in the compact evidence.
