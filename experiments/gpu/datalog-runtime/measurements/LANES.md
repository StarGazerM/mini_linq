# Packed FOR tiles: layout and decoder measurements

## Why the first interleaved attempt lost

FOR values are independent (`base + residual`); they do not require a cooperative prefix operation. GPU lanes already have independent registers and addresses: different shift operands do not themselves add warp instructions. Interleaving can change coalescing and specialization can provide immediates, but neither guarantees throughput. FastLanes' CPU SIMD constraints and the full-column, one-width generated-code setting of [FastLanes-GPU](https://ir.cwi.nl/pub/34260/34260.pdf) differ from these per-block widths, multiple columns and skewed intervals. The paper's compute-bound speedups are not a prediction for this workload. The primary next candidate is coalesced tile fetch plus **row-major** shared-memory extraction; intratile interleaving is a control.

Task-only counters at state 112, 2^28 rows, make the two losses explicit:

| Metric | Original row-major | Fully unrolled 8-lane |
|---|---:|---:|
| Device ms | 2.040960 | 16.496864 |
| Warp instructions | 1,342,872,639 | 1,611,926,691 |
| Warp instructions/row | 5.0026 | 6.0049 |
| Active lanes/instruction | 21.50 | 11.53 |
| Active thread instructions/row | 107.56 | 69.24 |
| Registers / occupancy % | 64 / 58.59 | 80 / 45.75 |
| No-instructions PC share % | 1.12 (supplemental control) | 83.70 |
| DRAM percent of peak | 34.75 | 10.57 |

The issue unit is the warp instruction, not an isolated active thread instruction. Different 8-lane groups execute different block/column/width bodies under masks: warp work increases even as active-thread work falls. The 600+ KB specialized bodies additionally starve instruction delivery. In contrast, the 1024-row/32-lane state-90 reduction takes 3.334496 ms against 3.258048 ms original, at 48.03% DRAM peak; its whole pipeline still loses because bounds become more expensive.

Original task load requests/row are **0.27070 (90)** and **0.18330 (112)**. Requested global-load sectors are **132,760,865 / 88,062,729**, respectively: **6.735 / 4.380×** retained representation bytes. This is L1-sector demand, **not DRAM traffic**. New physical-traffic profiles measure original task DRAM reads/writes of **840.258/287.283 MB (90)** and **636.252/10.081 MB (112)**. State-112 unrolled writes grow to **949.880 MB**, so reads alone cannot explain its memory traffic. No unmeasured per-source decomposition is asserted here.

Ncu reports 96 bytes/cycle sustained peak and approximately 9.49139 GHz DRAM cycles, or **911.17 GB/s**. The proposed **700 GB/s** is an aspirational operating point (~77% of that peak), not a measurement obtained by dividing logical plain bytes by DRAM utilization. Reading 631–643 MB once at that assumed rate costs 0.90–0.92 ms; projection, repeated tile visits, output/temporary traffic and bounds work must be accounted for separately. State-90 bounds alone already exceed that complete-scan reference.

## Predictions recorded before self-contained-tile runs

These are **[PREDICTION]**, not measurements. The planned tile has a compact local header, contiguous bases and width/offset descriptors, then payload; a separate u64 directory and fences locate it. One warp loads a tile coalescently into private shared scratch, then all 32 lanes decode the same block. Two work partitions preserve row-striped scheduling for short intervals and tile-aligned scheduling for long intervals. The initial per-interval crossover is 128 rows; 32/64/256 are comparison points, not assumed improvements.

The input distribution was counted on the CPU before these runs (`tile-prediction-inputs.json`). Without a short-range path, the 2^28-row scans visit 256-row tiles **3,947,540 / 1,184,479** times (states 90/112), **3.7647 / 1.1296×** the unique tile count. Their row-extraction steps average **23.95 / 31.50** valid lanes. In source 90, 90.56% of prefixes have fewer than 128 rows but contain only 3.69% of rows; source 112 is 88.36% of prefixes and 0.29% of rows. Padding every short query to a warp-owned tile would therefore manufacture idle work, not eliminate it.

| Planned variant | Predicted task requests/row 90 / 112 | Sectors/row 90 / 112 | Warp inst/row 90 / 112 | Active lanes 90 / 112 | Predicted full scan ms 90 / 112 | Xalan scan ms |
|---|---:|---:|---:|---:|---:|---:|
| Row-major tile, 32-byte alignment | .12 / .05 | .18 / .09 | 3.0 / 2.2 | 26 / 30 | 4.0 / 1.15 | 76.6 |
| Row-major tile, 128-byte alignment | .12 / .05 | .18 / .09 | 3.0 / 2.2 | 26 / 30 | 4.1 / 1.20 | 79.8 |
| Interleaved 32-lane tile, 32-byte alignment | .13 / .06 | .21 / .10 | 2.8 / 2.1 | 26 / 30 | 8.3 / 1.45 | 167.5 |
| Interleaved 32-lane tile, 128-byte alignment | .13 / .06 | .21 / .10 | 2.8 / 2.1 | 26 / 30 | 8.5 / 1.50 | 171.0 |

Arithmetic: a typical ~560–580-byte tile needs about five 32-lane u32 load instructions and 18 sectors for 256 rows; directory, work-prefix and short-query loads supply the remaining request/sector allowance. Filtering the many tiny intervals is essential to approach these estimates. At an assumed 6.0–6.6e11 issued warp instructions/s, `268435456 × 2.2–3.0 / issue_rate` is ~0.9–1.34 ms of issue work; ~0.65–0.8 GB of physical traffic at the aspirational 700 GB/s is ~0.93–1.14 ms. Row-major state 90 additionally budgets ~2.5 ms for bounds/preparation and ~0.2 ms for short-query work, yielding ~4 ms. State 112 budgets ~0.15 ms beyond the approximately 1-ms task. Issue rate and bandwidth jointly limit the task; instruction delivery must remain below 5% no-instructions samples. Interleaved bounds retain extra per-column dependencies: the forecast does not pretend tile staging removes their measured ~6–7 ms state-90 cost.

Xalan's original 81.490-ms scan total decomposes by the actual dispatch into **45.777 ms serial, 19.428 ms cooperative-short, and 16.284 ms balanced**. Halving only balanced work gives 73.348 ms; a 5% allowance on the other paths gives ~76.6 ms. Interleaved's measured nonbalanced work is 121.838 + 51.972 ms; a 10% metadata-locality improvement plus half of its 22.147-ms balanced work gives ~167.5 ms. Thus xalan is not predicted to inherit the largest-state bandwidth gain. Lookup forecasts are original row-major time ×1.05 (32-byte tiles) or ×1.10 (128-byte tiles), and existing interleaved time ×0.90; these are explicit locality/extra-address-work assumptions to test, not claims that colocation removes a dependency.

The 128-byte alignment is not predicted to reduce 32-byte-sector count: a 128-byte warp load starting at any 32-byte-aligned address already covers four sectors. It may change cache-line placement, while necessarily retaining more padding; unused end padding will not be fetched by the row-major reader. Any >20% prediction error will be explained using measured issue/pipe utilization, active lanes, cache hits, traffic, stalls and fixed-work costs.

DELTA is deliberately not implemented. It would need a cooperative prefix scan per tile or transposed independent lane chains; O(1) FOR lookup would become reconstruction from an anchor (or require additional checkpoints). The user-provided earlier size study reports 79.3 MB DELTA versus 89.1 MB FOR; those figures were not remeasured here and are not evidence for free random-access decoding.

### Compact-loop control prediction, recorded before running

`compact32` changes only the 256-row/32-lane decoder to a single runtime step loop with a branch-free u64 funnel and a clamped second-word address. It is a control for code expansion, **not** the self-contained shared-memory tile design above. Predicted task counters (90/112): **3.8/3.5 warp instructions per row**, **24/30 active lanes**, **.24/.18 load requests per row**, **.50/.50 sectors per row**; code below 48 KB and no-instructions share below 5%. Runtime addressing and repeated global lane-word loads add work, so instruction issue and dependent cached loads should replace instruction delivery as the limit. At 5.0/6.6e11 warp instructions/s, issue work is `268435456 × 3.8/5.0e11 = 2.04 ms` and `268435456 × 3.5/6.6e11 = 1.42 ms`; retaining measured interleaved bounds and a latency allowance predicts **9.3/2.3 ms full scans**. Xalan prediction is **168 ms** (121.8-ms serial path unchanged, cooperative-short reduced to ~37 ms, balanced to ~9 ms). Lookup time, bytes and admission should remain within 3%, identical, and within 5%, respectively, of `vector32`: neither random access nor packing changes. A metadata-only width histogram is collected after each state's measured reads.

### Uniform-straddle control prediction, recorded before running

`class32` (3c) retains compact code and the same 32-lane layout, but uses a u32 single-word extraction when `shift + width <= 32` and loads the next lane word only for a straddle. The branch is step-uniform, not value-dependent; width classes are zero, 1..32, and the explicit wide representation. The measured xalan compressed-frame histogram has 33,505 zero-width frames; dominant nonzero widths are 8 (30,838), 9 (26,913), 20 (18,700), 10 (18,083), and 18 (14,527). No widths are unrolled.

[PREDICTION] For full 256-row blocks, summing `8 + straddles(width)` over that histogram gives a payload-load instruction count of 0.6483 times the always-two-load loop. Total task requests/sectors are predicted to fall from .271/.568 to **.23/.43 per row (90)** and from .191/.448 to **.15/.32 (112)**; metadata/work scheduling is unchanged. Predict **4.6/3.5 warp instructions per row**, **20/23 active lanes**, code below 40 KB and no-instructions below 5%. At 5.0/5.5e11 warp instructions/s, task issue work is `268435456 × 4.6/5.0e11 = 2.47 ms` and `268435456 × 3.5/5.5e11 = 1.71 ms`; with unchanged bounds, predict **9.8/2.3 ms full scans** and **172 ms xalan**. Lookup time/bytes/admission remain unchanged within the same tolerances as 3b. Fewer global payload requests should reduce the measured long-scoreboard pressure; a uniform branch may offset some arithmetic saving.

## Protocol and interpretation

RTX 6000 Ada, sm_89, release/default-feature timing; NVTX-enabled binaries are used only for Nsight Compute counters. Every reported timing process has before/after compute-tenant queries. Raw binaries, source snapshots, build/test logs, JSON and profiles are in `/data/design-notes/doop-memory/packed-run/arch/lanes/`. All measurements in this document were collected for this experiment. Xalan timing sums ten warmed calls over 131 states; scaled timing is milliseconds per call at 268,435,456 rows. E/P/C denote engine / new set with plain storage / new set with packed storage. Xalan E is paired with C; P has its own same-process E in the raw JSON. No engine implementation changed.

The original scalar control (`original`) and lean scalar control (`lean`) use the same checked RCCL 1024-atom scheduler as the interleaved variants. `lean.patch` preserves the measured u32 row-major loop, including the wide fallback. Counters select the second warmed launch inside the existing workload/state NVTX scopes. Long scans include bounds, offsets and reduction; instruction counts below sum that pipeline. Registers, occupancy, sector ratio and branch samples describe its reduction kernel (or the sole serial kernel). Profiles cover all five workloads at xalan states 90/112/137 and both scaled states, not an invented whole-xalan hardware-counter aggregate.

## Layout and library boundary

For residual row r, lane = r & (L-1), step = r >> log2(L), bit = step*w, and u32 address = `(bit >> 5)*L + lane`. All lanes in a vector have the same shift, mask and straddle decision at each step. A column occupies `L*ceil(ceil(rows/L)*w/32)` u32 words; partial blocks include their padded lane tails. Width zero stores no residual words. Widths 33..64 use an explicit wide row-major column bitstream, not truncation.

RCCL supplies `InterleavedForRange`, `ForColumn`, `pack_for_into[_async]`, allocator-aware `pack_for_workspace_in`, and packing from any lowered u64 read range. Completions retain input/output/workspace borrows. Mini_linq retains its headers, fences, cutoff and set/ratio policy; its seal and shared-tile merge packers call RCCL's word packer. Run headers remain 32 bytes and column frames 16 bytes (u64 base, u32 width, u32 column-word offset). Mini_linq aligns each column to two u32 words. No whole-run decoded temporary is used outside oracles.

The cooperative-sum contract permits vector ownership without changing checked preparation or task ownership: four 8-lane groups own independent 256-row vectors, or one 32-lane group owns a 1024-row vector. Values are consumed immediately. Random access stays O(1), uses masks/shifts rather than division for lane addressing, and retains once-per-probe residual translation when the composite residual fits u64.

### Measured xalan geometry bytes

Payload/header/fence columns are allocation sizes reported by run owners. The retained pool total additionally includes reader/control allocations and allocator rounding; it is the footprint gate. Header bytes include frame/width/field metadata. All totals include partial-block tails; tail rows also include plain small runs.

| Rows × lanes | Payload | Header/frame/field | Fences | Tail rows | Retained pool bytes |
|---|---:|---:|---:|---:|---:|
| 256 × 8 (32 values/lane) | 88,702,464 | 6,867,700 | 1,036,208 | 22,303 | 96,714,240 |
| 256 × 32 (8 values/lane, padded) | 98,770,224 | 6,867,700 | 1,036,208 | 22,303 | 106,777,344 |
| 1024 × 32 (32 values/lane) | 102,077,760 | 1,722,644 | 259,568 | 72,991 | 104,165,888 |

Original retained xalan bytes: **96,710,144**. The 8-lane layout adds 4,096 bytes; the two other geometries are materially larger. The 1024-row format reduces metadata but widens column residuals enough to increase payload.

## 3a: full width × step specialization

RCCL dispatches widths 0..32 once per vector/column into 32 const-generic steps with immediate shifts/masks and statically resolved straddles. No 32-value per-thread buffer is used. `vector8`, `vector32`, and `vector1024` are the final strength-reduced, vector-owned variants; earlier `lanes*` and `group*` artifacts are not substituted for them.

### Mandatory three-decoder scan counters

Time is the full pipeline in ms; instructions are active predicated-on thread instructions per scanned row. B is branch-resolving PC samples as percent of all samples; U is uniform branch-target percent. S is global-load sectors/request. R/O are registers/thread and achieved occupancy percent. DRAM is percent of peak reported by ncu.

| Case | Decoder | ms | Inst/row | B % | U % | S | R/O % | DRAM % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| xalan90 | original row-major | 0.1535 | 206.9 | 4.67 | 87.67 | 1.851 | 64/28.56 | 0.03 |
| xalan90 | lean row-major | 0.1536 | 198.7 | 5.44 | 86.40 | 1.865 | 76/28.16 | 0.03 |
| xalan90 | 3a interleaved 8-lane | 0.1955 | 213.2 | 2.48 | 95.56 | 1.464 | 64/39.25 | 0.40 |
| xalan112 | original row-major | 0.0609 | 115.4 | 8.32 | 80.70 | 1.920 | 64/39.44 | 0.77 |
| xalan112 | lean row-major | 0.0601 | 101.1 | 7.98 | 79.14 | 1.966 | 76/38.84 | 0.67 |
| xalan112 | 3a interleaved 8-lane | 0.1411 | 79.6 | 3.36 | 93.11 | 1.272 | 80/42.17 | 1.30 |
| scale90 | original row-major | 5.4807 | 136.8 | 2.49 | 87.03 | 1.827 | 64/58.19 | 37.90 |
| scale90 | lean row-major | 6.0071 | 128.5 | 3.26 | 85.68 | 1.843 | 76/42.52 | 33.35 |
| scale90 | 3a interleaved 8-lane | 15.5346 | 129.5 | 1.24 | 95.97 | 1.437 | 64/58.93 | 20.62 |
| scale112 | original row-major | 2.1781 | 108.7 | 3.76 | 81.00 | 1.790 | 64/58.65 | 34.75 |
| scale112 | lean row-major | 2.5339 | 94.6 | 4.89 | 79.40 | 1.831 | 76/43.58 | 29.57 |
| scale112 | 3a interleaved 8-lane | 17.0496 | 71.2 | 0.91 | 94.58 | 1.282 | 80/45.75 | 10.57 |

At scaled state 112, active instructions fall **108.7 → 71.2**, but the reduction kernel's `no_instructions` PC share is **83.70%** (original supplemental control **1.12%**). Its text grows **31,616 → 666,496 bytes**; state 90 grows **26,368 → 615,808 bytes**, with **64.93%** `no_instructions`. Text sizes are measured from CUDA ELF function symbols embedded in the binaries (`kernel-symbol-sizes.json`); ordinary cuobjdump scanning of the Rust executable does not find those embedded standalone cubins. [INFERENCE] Instruction-delivery/cache pressure from the fully expanded width-by-step code, rather than residual bandwidth, dominates this loss. This rejects the fully unrolled implementation, not interleaving as a data decomposition.

Xalan state-137 shuffled lookup changes **349.792 → 1110.816 us**, **1555.4 → 2592.5 active instructions/probe**, **17.854 → 22.909 sectors/request**, and **61.41 → 55.29% occupancy**, at 64 registers. Its new long-scoreboard share is **94.65%**, versus only **0.24% no-instructions**: lookup loss is a different problem, extra per-column dependent accesses and extraction, not the scan's instruction-delivery bottleneck.

### Default-build three-decoder times

E/P are measured with 3a; scalar C controls are separate fresh processes. Xalan entries are ten-call totals; scale entries are per-call ms. Full paired data and tenant observations remain in the raw JSON.

| Case/workload | E | P | C original | C lean | C 3a |
|---|---:|---:|---:|---:|---:|
| xalan/prefix_sorted | 17.5563 | 16.1420 | 21.5565 | 21.6877 | 35.0041 |
| xalan/prefix_shuffled | 39.7371 | 37.2428 | 44.3079 | 44.4727 | 122.6813 |
| xalan/prefix_mixed | 47.6800 | 44.9238 | 53.6599 | 53.7949 | 155.7291 |
| xalan/range_scan | 59.6221 | 57.5407 | 81.4896 | 82.3355 | 192.3196 |
| xalan/membership | 20.4204 | 19.7673 | 27.1698 | 27.5287 | 53.1940 |
| scale90/prefix_sorted | 0.5473 | 0.5506 | 0.3389 | 0.3390 | 0.5180 |
| scale90/prefix_shuffled | 3.8053 | 3.7711 | 2.1042 | 2.1046 | 7.5074 |
| scale90/prefix_mixed | 6.5310 | 6.4594 | 3.5776 | 3.5887 | 12.6368 |
| scale90/range_scan | 6.6318 | 5.9068 | 5.3394 | 5.8837 | 15.4479 |
| scale90/membership | 0.1047 | 0.1061 | 0.0570 | 0.0549 | 0.0933 |
| scale112/prefix_sorted | 0.0217 | 0.0195 | 0.0309 | 0.0281 | 0.0534 |
| scale112/prefix_shuffled | 0.1661 | 0.1604 | 0.1648 | 0.1596 | 0.6857 |
| scale112/prefix_mixed | 0.3725 | 0.4141 | 0.2495 | 0.2414 | 1.0346 |
| scale112/range_scan | 8.5448 | 5.1560 | 2.4801 | 2.6136 | 16.8022 |
| scale112/membership | 0.1541 | 0.1517 | 0.0642 | 0.0615 | 0.1203 |

### Admission, three columns (default build, xalan ms)

Seal and merge produce the measured interleaved layout; admission is not a reinterpretation of previously packed bytes.

| Phase | E | P | C 3a |
|---|---:|---:|---:|
| Sort/subtract (including fused) | 165.456 | 124.096 | 119.178 |
| Seal/adopt | 0.531 | 3.081 | 8.285 |
| Merge/publication | 18.341 | 25.109 | 39.881 |
| Total | 184.328 | 152.286 | 167.345 |

## Exactness and reproduction

The final 3a variant passes **31 packed-run tests in 5.25 s**, including every residual width 0..64, all four key widths, copy-through merges, tails, bounds and balanced scan boundaries. All full xalan and both largest-scale read/admission oracles pass for each geometry. RCCL's exhaustive CPU bitstream oracle covers every width 0..64 and every row count through each block size; the GPU example checks packing, lazy projections, random access, completion drop and sorted search. The existing GPU expansion oracle and all 11 Thrust doctests also pass. The original three-row physical-layout test now checks its plain fallback and exact decoded bounds as well; its original row-major bit-order assertion is retained for row-major storage.

Build/run locks and NVTX scopes follow [ARCHITECTURE.md](ARCHITECTURE.md#reproduction). This experiment's raw `timing.sh` and `counters.sh` preserve exact commands. Counter selection uses `--cache-control none --clock-control none`, workload/state NVTX inclusion and occurrence 2 (xalan) or 3 (scaled packed), with skip/count 3/3 for the checked scan pipeline and 1/1 for single-kernel reads. The 1024-row geometry adds `--block 1024`; the padded 256-row build sets `FOR_LANES_256 = 32`. Cargo's local RCCL path override and lockfile source changes remain intentionally uncommitted.

## Compact controls 3b/3c: measured outcome

The runtime-loop controls fix instruction delivery but do not satisfy the production storage gate. `compact32` is the branch-free funnel (3b); `class32` is the uniform-straddle u32 control (3c). Both retain 256-row/32-lane padded storage and the same random-access path. **RCCL keeps 3b as its default**, favoring its lower time on both largest states; 3c improves xalan but is not a consistent win. This is not a claim that 3b is optimal for every range length.

### Task instruction and delivery counters

Pipeline time includes bounds/offsets; task time and all other counters describe the selected reduction kernel. Code sizes are CUDA ELF function-symbol sizes. Active lanes are the measured predicated-on thread/warp-instruction ratio, not the nominal group size.

| Case | Decoder | Pipeline / task ms | Warp inst/row | Active lanes | Thread inst/row | Code bytes | No-inst % | Registers / occupancy % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| xalan90 | 3a, 32 lanes | 0.1937/0.1461 | 7.153 | 23.52 | 168.24 | 615,808 | 43.10 | 64/38.12 |
| xalan90 | 3b | 0.1929/0.1445 | 5.987 | 20.86 | 124.89 | 32,640 | 2.34 | 96/18.23 |
| xalan90 | 3c | 0.1923/0.1444 | 5.911 | 20.57 | 121.59 | 32,384 | 2.03 | 92/18.39 |
| xalan112 | 3b | 0.0719/0.0440 | 4.060 | 24.26 | 98.50 | 36,352 | 2.52 | 105/25.87 |
| xalan112 | 3c | 0.0722/0.0444 | 3.966 | 23.59 | 93.55 | 36,224 | 2.30 | 104/25.95 |
| scale90 | 3a, 32 lanes | 13.5176/6.2951 | 6.545 | 23.33 | 152.71 | 615,808 | 60.74 | 64/58.11 |
| scale90 | 3b | 11.0641/3.8286 | 5.440 | 20.47 | 111.37 | 32,640 | 1.57 | 96/30.20 |
| scale90 | 3c | 11.1841/3.9367 | 5.364 | 20.12 | 107.92 | 32,384 | 1.42 | 92/30.23 |
| scale112 | 3a, 32 lanes | 12.8089/12.2718 | 4.884 | 27.11 | 132.42 | 666,496 | 83.28 | 80/45.93 |
| scale112 | 3b | 2.6484/2.1129 | 4.006 | 23.44 | 93.89 | 36,352 | 1.69 | 105/29.98 |
| scale112 | 3c | 2.9010/2.3621 | 3.911 | 22.74 | 88.93 | 36,224 | 1.65 | 104/30.17 |

The supplemental 3a/32-lane xalan-112 counter case was not collected; the earlier mandatory table contains its measured 8-lane control.

### Task traffic, issue and pipeline utilization

Redundancy is requested global-load sector bytes divided by that variant's retained representation, not DRAM bytes. Issue A/E is active-cycle / elapsed-cycle issue utilization; ALU and LSU are ncu's active-cycle pipe utilizations. B/U are branch-resolving sample percent / uniform branch-target percent. The padded representation's larger denominator is included.

| Case/decoder | Requests/row | Sectors/row | Redundancy | L1 hit % | Long scoreboard % | B/U % | Issue A/E % | ALU/LSU % | DRAM % |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| scale90/3b | 0.2712 | 0.5685 | 6.800× | 72.48 | 59.28 | 2.80/97.11 | 30.10/27.01 | 30.13/13.10 | 38.02 |
| scale90/3c | 0.2509 | 0.4907 | 5.870× | 68.93 | 60.42 | 2.70/97.11 | 29.12/25.82 | 29.49/12.73 | 36.02 |
| scale112/3b | 0.1913 | 0.4478 | 5.207× | 79.47 | 53.70 | 3.52/98.15 | 36.28/33.39 | 33.31/14.73 | 38.86 |
| scale112/3c | 0.1609 | 0.3290 | 3.826× | 72.32 | 57.43 | 3.29/98.15 | 33.94/31.62 | 32.86/14.28 | 34.70 |

### Predictions versus measurements

- **3b:** full default-build scans are **10.942/2.746 ms**, against **9.3/2.3 ms** predicted; xalan is **176.589 ms**, against 168 predicted. The larger task-level miss is measurable: warp instructions/row are **5.440/4.006**, not 3.8/3.5, and sustained issue is **3.814/5.089e11 warp instructions/s**, not 5.0/6.6e11. Those factors reproduce the **3.829/2.113-ms** tasks. Registers are **96/105**, occupancy only **30.20/29.98%**, and long-scoreboard share **59.28/53.70%**. Removing the large code body did not make runtime addressing cheap or hide dependent loads. The state-112 active-lane forecast (30) was data-step-only and overoptimistic: the whole task measures **23.44**, including scheduler/control/reduction work and partial spans. Requested-traffic forecasts are within 20%; the instruction/issue assumptions were not.
- **3c:** full scans are **11.021/2.886 ms**, against **9.8/2.3 ms** predicted, and xalan is **166.794 ms**, against 172 predicted. The state-112 >20% full-time miss is explained by **3.911 warp instructions/row** and **4.444e11 issued warp instructions/s**, giving **2.362 ms** of task time, plus **0.539 ms** bounds/offsets. State 90 likewise issues only **3.657e11/s**, giving a **3.937-ms** task rather than 2.47 ms predicted. Registers remain **92/104** with ~30% occupancy, and long-scoreboard share rises to **60.42/57.43%**. All request/sector and active-lane forecasts are within 20%.
- Removing unused high-word loads in 3c reduces state-112 requested sectors/row **.44778 → .32899**, but L1-miss sectors/row (sectors × miss rate) change only **.09194 → .09105**. Eliminated requests were predominantly hits. [INFERENCE] Removing speculative next-word loads also removes lookahead/independent load opportunity; the higher long-scoreboard share and lower issue rate explain why fewer requests do not produce a faster largest-state scan. This is a controlled removal experiment for that compact-reader load class, **not** a decomposition of the original decoder's frames/straddles/task loads.
- At the aspirational 700-GB/s reference, the padded representations require **1.026/1.055 ms** for one complete representation pass. 3b tasks remain **3.73/2.00×** that reference; 3c remains **3.84/2.24×**. These gaps correspond to dependent-load stalls, low occupancy and repeated sector demand, not instruction delivery (now <2% at scale). Bounds are additional and dominate state 90.

### Default-build lookup and scan times

All five workloads were timed and checked against the oracle for each compact control. Xalan is the ten-call/all-state total; scale is per-call ms. These are C times, not ncu timings.

| Case/workload | 3b C ms | 3c C ms |
|---|---:|---:|
| xalan/prefix_sorted | 34.5605 | 34.4972 |
| xalan/prefix_shuffled | 122.9805 | 123.3606 |
| xalan/prefix_mixed | 156.7379 | 157.0420 |
| xalan/range_scan | 176.5885 | 166.7936 |
| xalan/membership | 53.6875 | 53.6164 |
| scale90/prefix_sorted | 0.5567 | 0.5562 |
| scale90/prefix_shuffled | 7.2671 | 7.2658 |
| scale90/prefix_mixed | 12.4778 | 12.4780 |
| scale90/range_scan | 10.9423 | 11.0205 |
| scale90/membership | 0.1013 | 0.1008 |
| scale112/prefix_sorted | 0.0543 | 0.0528 |
| scale112/prefix_shuffled | 0.6645 | 0.6589 |
| scale112/prefix_mixed | 0.9892 | 0.9876 |
| scale112/range_scan | 2.7465 | 2.8856 |
| scale112/membership | 0.1338 | 0.1329 |

Both compact forms retain **106,777,344 xalan bytes**, versus original **96,710,144**. Random access was not changed: xalan shuffled lookup stays about **123 ms**, against **44.308 ms** original. Fixing scan code footprint therefore does not repair lookup or the padded layout's footprint.

### Compact admission (xalan default-build ms)

| Decoder/phase | E | P | C |
|---|---:|---:|---:|
| 3b/Sort/subtract | 166.533 | 113.244 | 117.381 |
| 3b/Seal/adopt | 0.498 | 2.581 | 7.725 |
| 3b/Merge/publication | 17.997 | 23.748 | 38.617 |
| 3c/Sort/subtract | 162.489 | 113.270 | 117.230 |
| 3c/Seal/adopt | 0.478 | 2.647 | 7.491 |
| 3c/Merge/publication | 17.586 | 23.643 | 38.146 |

The compact controls each pass **31 packed-run tests** (4.94/4.95 s), full xalan replay and both 2^28-row replay oracles. RCCL's exhaustive two-test CPU oracle and GPU pack/range/search example pass for both forms. All timing/profile tenant files collected for these controls are empty. The full compressed-frame histogram is:

```
0:33505, 1:3559, 2:2303, 3:1776, 4:1413, 5:2348, 6:3560, 7:2896, 8:30838, 9:26913, 10:18083, 11:9131, 12:5190, 13:4681, 14:4631, 15:6092, 16:6871, 17:13510, 18:14527, 19:9321, 20:18700, 21:12276, 22:4546
```

## Handoff boundary

The tested interleaved storage integrations fail the unchanged scan/lookup/bytes gate. Production is returned to the original row-major representation; this does **not** close the newly proposed shared-memory tile design. The compact RCCL primitive remains available, with its tested branch-free loop as default.

The self-contained aligned tile comparison, fused tile-aligned/short-range work scheduling, and exact original-load-source attribution are **not implemented or measured**. Their pre-run predictions above remain predictions. Partial tile-directory and partitioned-workspace edits were archived outside git (`tile-wip-mini.tar`, `tile-wip-rccl.tar`) and removed from the worktrees rather than leaving a broken migration. NVBit 1.8.1 was downloaded and hash-checked for possible post-SASS load attribution, but no instrumentation tool was built or used; no source-count result is claimed. RCCL search migration/cooperative pivots and admission-mechanics migration (original rows 4/5) remain unstarted. There is no measured claim for them.

## Delivered row-major checkpoint

A fresh default-build replay after restoring row-major storage passes every oracle, with no foreign tenant observed. Final packed-run tests: **31 passed in 5.07 s**. Delivered RCCL CPU codec/cooperative-sum tests: **2 passed in 6.14 s**; the GPU pack/range/search oracle passes. The 11 borrow/doctests and the existing GPU expansion oracle passed earlier in this experiment. These tables are fresh measurements of the delivered source, not copied baseline values.

| Case/workload | E ms | P ms | C ms |
|---|---:|---:|---:|
| xalan/prefix_sorted | 17.9900 | 16.0976 | 21.5952 |
| xalan/prefix_shuffled | 40.5258 | 37.3240 | 44.6693 |
| xalan/prefix_mixed | 48.7214 | 45.2490 | 54.6941 |
| xalan/range_scan | 60.5026 | 57.3441 | 81.9269 |
| xalan/membership | 21.0991 | 19.6689 | 27.4403 |
| scale90/prefix_sorted | 0.5490 | 0.5503 | 0.3402 |
| scale90/prefix_shuffled | 3.8183 | 3.8017 | 2.1082 |
| scale90/prefix_mixed | 6.5521 | 6.4534 | 3.6007 |
| scale90/range_scan | 6.6319 | 5.8971 | 5.3260 |
| scale90/membership | 0.1031 | 0.1060 | 0.0541 |
| scale112/prefix_sorted | 0.0218 | 0.0215 | 0.0286 |
| scale112/prefix_shuffled | 0.1645 | 0.1605 | 0.1617 |
| scale112/prefix_mixed | 0.3647 | 0.4085 | 0.2437 |
| scale112/range_scan | 8.5277 | 5.1639 | 2.1890 |
| scale112/membership | 0.1533 | 0.1515 | 0.0616 |

Retained xalan E/P/C pool bytes: **269,729,792 / 269,691,648 / 96,710,144**. The local Cargo path override and lockfile changes remain uncommitted.

| Delivered xalan admission phase | E ms | P ms | C ms |
|---|---:|---:|---:|
| Sort/subtract | 186.036 | 112.765 | 120.418 |
| Seal/adopt | 0.606 | 2.606 | 8.086 |
| Merge/publication | 20.898 | 23.461 | 38.920 |
| Total | 207.539 | 138.832 | 167.424 |

RCCL commits: `934ddfe73c` (range/packer), `8508343539` (cooperative partition contract), `e75f52f9cd` (lane address strength reduction), and `d05b07b7c9` (compact default). Mini_linq's fully unrolled experiment is preserved in `3dccae61f3`; this document's restoration commit retains the new oracle and metadata histogram, not the failed interleaved production integration. Raw `compact32-source/`, `class32-source/`, `lean.patch`, binaries and counter drivers preserve the other controls without adding bulky profiles to git.
