# Barrier-free packed merge writer

## Change 1: independent payload words

[INFERENCE, recorded before editing] Move insertion-code translation into ranking and output-word starting ranks plus metadata into a barrier-free preparation pass. The writer then owns disjoint output words, reads their precomputed starting insertion indices, and copies owner bit intervals or encoded insertion fragments without a shared tile, block barriers, or binary searches. Rare widened owner frames use code-space fragment translation, not decoded rows. Use 128-thread writer blocks.

Predicted cold state-90, 2^26 rows, 1/16 delta counters: writer barrier samples 41% → below 1%; lanes/instruction 14.9 → 28–31; registers 72 → 40–64; achieved occupancy 41% → 60–85%; writer DRAM throughput 97 → 350–550 GB/s. These are predictions, not measurements. The extra word-rank scratch costs eight bytes per output payload word and a preparation launch; end-to-end speedup is not guaranteed. Ranking also writes encoded insertion codes, with fixed schema-width scratch slots, so its DRAM traffic will increase. Launch count is initially six instead of five. Exact allocation/readback remains until independently evaluated.

Baseline and per-change full Nsight captures, process checks and clean timing outputs are kept outside git in `/data/design-notes/doop-memory/merge-writer/`. Supplied historical measurements are not claimed as runs made for this experiment.

Measured, fresh `c1-full.ncu-rep` (cold caches, state 90, 2^26 rows, 1:16). The original selector missed **the merge's sizing scan**. Including `offsets_TID` reveals three scans: the first two belong to sealing the inputs and are excluded; the third is inside the merge and must be counted. The previously reported 4.60 ms is not the full packed sum.

|Launch|ms|DRAM MB (read + write)|GB/s|Registers|Active warps %|Issue %|Lanes/inst|
|---|---:|---:|---:|---:|---:|---:|---:|
|E partition|0.077|16.06|208|19|16|2|31.73|
|E merge|1.249|1025.55|821|38|94|21|31.47|
|P plain-storage merge|2.737|1059.10|387|33|92|21|30.97|
|C partition|0.072|29.72|413|36|65|21|31.03|
|C plan|0.827|47.71|58|80|49|72|18.86|
|C offsets|3.168|9.92|3|40|17|12|31.82|
|C rank insertions|0.622|229.95|370|64|56|59|29.11|
|C prepare write|1.646|183.78|112|48|76|68|27.89|
|C write|1.434|489.29|341|80|41|48|16.35|

Six C launches, **7.769 ms**, versus E's two launches, **1.326 ms**. The new writer met the barrier prediction (less than 1% of stall samples), but missed lanes, registers, occupancy and throughput; long scoreboard accounts for 47% of its stall samples, wait 24%. The path choice (untouched / fit / widen) is uniform within a CTA: each CTA works on just one leaf. The fragment loop, however, is per *word*; only words intersecting insertions or a reframed interval take the expensive fragment paths. Its live state includes two views, absolute 64-bit row and bit cursors, a running word and a fall-through call to the code translator. `offsets` runs one CTA for all 246,724 owner leaves, serially revisiting CTA barriers and global memory. Raising occupancy of the writer alone cannot approach E's time.

## Change 2: one insertion mask per 32 rows

[INFERENCE, before implementation] Rank a whole owner interval per warp rather than search the owner directory per arriving row. The same warp publishes headers/fences and one `(owner rows before group, insertion bitmap)` pair per 32 output rows; remove the prepare launch, owner map and 64-bit rank per payload word. A warp per owner writes contiguous output words using population counts, local 32-bit bit cursors and the next set bit to find contiguous owner spans. Expect five launches, rank+metadata below 1.6 ms (previously 2.27), 300–390 MB writer traffic, 20–28 lanes/inst, 56–72 registers and 330–520 GB/s. Variable fragment counts and the rare frame-translation path may still limit lane use. Exact sizing remains an independent bottleneck.

Measured (`c2.ncu-rep`): rank + metadata **1.214 ms**, 265.8 MB, 80 registers, 44.0% occupancy, 15.76 lanes; prepare eliminated. Writer **0.981 ms**, 323.1 MB, 329 GB/s, **64 registers**, 52.5% occupancy, 59.1% issue, **14.48 lanes**. Five-launch sum **6.261 ms** including offsets; scratch for group starts and masks is 18.8 MB here. Register, traffic and fused-pass predictions held; utilization did not. SASS instruction-weighted lane counts in `[0,8), [8,16), [16,24), [24,32]` are 13.8%, 64.6%, 7.7%, 14.0%. The fragment-loop backward branch alone diverged 1.326M times in 2.057M warp executions, averaging only 13.26 lanes. The corresponding change-1 backward branch diverged 1.170M times in 1.944M executions, averaging 13.73 lanes. Moving to a single warp per leaf did not solve divergent sub-word iteration.

Independent baseline control (`baseline-full.ncu-rep`), using the archived `leaf-v8` binary whose SHA-256 matches the leaf study's final admission run: writer 3.491 ms, 336.8 MB, 96 GB/s, 72 registers, 41.3% occupancy, 36.9% issue, 14.90 lanes; 36.0% of stall samples were barrier stalls. Full five-launch packed total **8.053 ms**, of which 3.169 ms was offsets. The alternative four-kernel total without sizing, 4.884 ms, is not the merge's bandwidth comparison.

## Change 3: tile the complete layout plan

[INFERENCE, before implementation] Give each lane one owner interval, share the next fence rank with its neighbor, and compute the two exclusive output prefixes within each warp while frames are hot. Scan only one pair of totals per 32 leaves, adding the tile base at consumption. Expect partition to disappear, plan 0.20–0.65 ms (above 23 active lanes), sizing below 0.20 ms instead of 3.17, four packed launches and a full sum below 3.0 ms. No output geometry or frame-selection change. The metadata-fit fast path should benefit; long conflicting intervals may penalize the per-lane planner.

Measured (`c3.ncu-rep`): fused plan **0.150 ms**, 38.6 MB, 80 registers, 24.87 lanes; `tile_offsets` **0.047 ms** (7,711 pairs), 40 registers, 31.69 lanes. Rank + metadata 1.168 ms, 72 registers, 16.07 lanes; writer 0.983 ms, 64 registers, 14.64 lanes, 329 GB/s. Four packed launches, **2.348 ms** including sizing. The plan beats the predicted range; 39% of its stall samples are long scoreboard rather than the old column-reduction wait. One 16-byte sizing readback remains. The store instruction itself executes with 26.82 lanes in changes 1 and 2: the larger loss comes from the variable fragment loop, not from store tails alone.

## Change 4: merge compressed codes directly in warp-local tiles

[INFERENCE, before implementation] Do not stage global positions, insertion codes or masks. For a compact common output frame, merge the two sorted streams of residual codes in shared memory, using one merge-path cut per lane, even contiguous work per lane, then pack contiguous words. Use an independent wide kernel (merge row references in code space, translate only requested fragments); the two paths never share register allocation. Pure owner copies bypass merging. Expect compact writer 0.65–1.05 ms, 300–430 GB/s, 23–29 lanes, 48–72 registers, no separate rank pass; wide kernel under 0.2 ms on the narrow state-90 case. Predicted full sum 0.95–1.45 ms versus E at 1.33. Competing costs are shared-bank conflicts, packing variable-width words and writer metadata publication.

Measured (`c4.ncu-rep`): compact writer **1.685 ms**, 298.4 MB, **177 GB/s**, **96 registers**, 38.0% occupancy, 43.5% issue, **24.30 lanes**; wide writer **0.404 ms**, 12.5 MB, 128 registers, 16.4% occupancy, 21.85 lanes. Plan and tile scan 0.148 and 0.047 ms; four-launch sum **2.284 ms**. The separate rank launch and **all** per-incoming/per-word global scratch disappeared. Lane use improved, but the compact kernel must keep arbitrary split cursors, two staged code streams, fence generation and its serial lane merge live. Long scoreboard is still 40.4% of its stall samples. Moving that much control flow into the writer missed the throughput and register predictions.

## Change 5: splice a warp's insertions into the original bitstream

[INFERENCE, before implementation] An unsplit, fitting compact leaf with at most one insertion per lane needs neither a full shared merge nor the general writer's split/fence control. Hold sorted insertion bit positions and codes in lane registers; stage the packed owner words once. For each output word, find its first overlapping insertion by warp exchange, read one contiguous owner window and splice any overlaps. Whole warps issue each exchange; unsplit first fences already exist in the inputs. Use layout-produced per-tile exception masks for the remaining leaves. Expect common writer 0.45–0.75 ms, 400–600 GB/s, 26–31 lanes and 48–64 registers; expensive/general writer 0.1–0.4 ms. Expected full sum 0.8–1.35 ms. This trades shared-code and metadata work for warp shuffles; dense or widened tiles still use the general merge.

Measured (`c5.ncu-rep`): sparse writer **0.644 ms**, 285.4 MB, **444 GB/s**, **48 registers**, 72.3% occupancy, 71.9% issue, **22.41 lanes**. General writer **0.844 ms**, 11.2 MB, **152 registers**, 8.8% occupancy, 23.1% issue, 21.80 lanes. Plan 0.152 ms; sizing 0.047 ms; sum **1.686 ms**. The sparse splice puts occupancy and traffic in range but each owner typically has only 16 incoming rows: the hottest wasted-lane search instructions execute at exactly 16 lanes, about 30.3M idle lane-instructions apiece. General writing, not bulk payload movement, is now the largest interval; its wait and long-scoreboard stalls are 34.7% and 28.3%.

## Change 6: give each exceptional leaf a full CTA

[INFERENCE, before implementation] The general warp is spending 0.844 ms on 11 MB. Use compact 64-bit (rather than only 32-bit) common-frame codes for exceptional packed leaves; retain source-reference comparisons for identity or larger frames. A 128-thread CTA shares one tile per leaf, and the two specializations no longer share a 152-register allocation. Reduce class presence in the existing tile scan and submit only nonempty streams. Expect exceptional writing 0.12–0.35 ms, active occupancy above 25%, compact general registers at most 85, sparse writer unchanged; full kernel sum 1.0–1.30 ms.

Measured (`c6.ncu-rep`): exceptional compact writer **0.221 ms**, 11.3 MB, 80 registers, 45.9% occupancy, 28.74 lanes. No identity/wider-than-64 class needed a launch. Sparse writer **0.643 ms**, 285.4 MB, 444 GB/s, 48 registers, 22.41 lanes. Plan 0.148 ms, class-aware tile scan 0.061 ms. **Four launches total 1.074 ms, below E's 1.324 ms**, including the sizing kernel on both sides of the comparison where applicable. The turnover came from eliminating repeated full source comparisons on the minority of leaves; the common writer itself has not yet reached 500 GB/s.

## Change 7: two independent sparse leaves per warp

[INFERENCE, before implementation] Use 16-lane shuffle masks so that the predominant 16 arriving rows occupy both halves of a warp on independent leaves. More than 16 insertions go to the general stream. Size each team's shared packed source from the known fixed-owner row cap (512 for variable owners), with two guarded zero words for window reads. Expect 27–31 lanes, 48–60 registers, 0.45–0.58 ms and 490–630 GB/s for sparse writing; total 0.9–1.10 ms if exceptions stay sparse. Separately measure the exact sizing round trip, not just the GPU scan: opt-in CUDA markers bracket the device scan and the post-scan readback/host resubmission gap, a host clock brackets the actual synchronous download. Diagnostic runs are excluded from clean E/P/C timings.

Measured (`c7.ncu-rep`): common writer **0.511 ms, 559 GB/s, 48 registers, 26.02 lanes**. Two half-warps increased useful work: the share of warp instructions at PCs averaging at least 24 active lanes rose from 46.9% (change 5) to 85.0%. The remaining hole is inside the insertion splice itself: seven of the top eight idle-lane PCs are shift instructions averaging **8.90 lanes**. Words without overlapping insertions skip them. Issuing those instructions for unrelated words just to raise the lane metric would not move more useful bytes. On this input 6,828 of 246,724 owner intervals widened; these use `write_compact`, not the common writer.

### Change 7 cold hardware counters

RTX 6000 Ada, `ncu --set full`, state 90, 67,108,864 rows, 1:16; decimal DRAM MB and GB/s. These are individual cold-cache replay intervals, not the same as the later unprofiled host-submission timing. `--kernel-name` included `tile_offsets`, `offsets_TID`, `write_compact` and `write_wide` in addition to the original selector. Two input-sealing scans precede the measured merge and are excluded.

|Launch|ms|DRAM MB|GB/s|Registers|Occupancy %|Issue %|Lanes/inst|
|---|---:|---:|---:|---:|---:|---:|---:|
|E partition|0.077|16.1|209|19|16.0|2.1|31.73|
|E merge|1.252|1025.6|819|38|93.5|20.9|31.47|
|P plain-storage merge|2.734|1059.9|388|33|92.1|21.2|30.97|
|C plan (incl. fence partition)|0.149|38.2|256|80|40.8|46.6|24.88|
|C tile offsets + class flags|0.062|0.2|3|36|16.5|16.0|31.74|
|C sparse writer|0.511|285.6|559|48|71.2|67.1|26.02|
|C exceptional compact writer|0.222|11.3|51|80|46.0|39.1|28.74|
|C wide reference-index writer|not launched|—|—|—|—|—|—|
|Shifted-owner copy control (separate capture)|0.330|246.1|745|27|90.4|29.9|27.28|

**C 0.943 ms vs E 1.329 ms.** The >500 GB/s and ≤64-register targets hold for the common writer, but **>28 lanes does not**. Exceptional writing has the opposite tradeoff; its largest stall categories are long scoreboard (32.4%) and barrier (25.7%). The four packed kernels also perform layout, comparisons, translation and publication that the shifted copy does not. Four launches here, three to five in general (plan + scan + populated output classes). No start ranks, per-incoming staging or decoded-row scratch in global memory. Sizing, payload, directories, plans, exception masks and frames all use the pool.

### Exactness and allocation

The all-code and all-packed-run suites, two synthetic storage replays, full admission oracle, matched-history real reads, and scaled reads and merges at both 2^28 states passed for each retained implementation. Final test count is 31 packed-run + 12 code-space. The repository-root formal inventory checker reported 523 rules and 522 registered decode sites; it is an inventory check, not a proof. Export/decode is still limited to existing oracle/public boundaries and the registered output fence; no whole-key decode was added to the operation path.

**No-readback goal missed deliberately.** Before seeing frame widths, `rows × sizeof(W)` bounds payload even with per-leaf padding (`sizeof(W)` is a multiple of eight). For 2^28, state 90/112 this reserves 2.147/4.295 GB of payload alone; measured *entire* exact outputs at 1:16 are 0.643/0.654 GB. The bounded leaf count, `owner_blocks + floor((rows - owner_blocks)/512)`, is not the exact number of valid headers/fences required by readers. Retain exact allocation, not a pessimistic capacity charged as compressed storage. Download 24 bytes once per packed merge (word count, leaf count, present writer classes); there is no sizing round trip on the small plain path.

#### Change 7: what the sizing round trip costs

Five fresh processes with `PACKED_LEAF_SIZING=1`, clean before/after process checks. Device scan = CUDA markers around `tile_offsets`. Post-scan round trip = scan-complete marker to a marker enqueued immediately after the blocking 24-byte download returned; includes DtoH, synchronization and host resubmission. Inclusive host wait = host clock around that download; it **also waits for previously queued work**, principally the leaf plan. The last two columns overlap: they must not be added. These instrumented runs do not supply the E/P/C table.

|Workload (individual packed-merge output rows)|Merges|Device scan p50 µs / sum ms|Post-scan round trip p50 µs / sum ms|Inclusive host wait p50 µs / sum ms|
|---|---:|---:|---:|---:|
|Cold 2^26, state 90, 1:16|1|23.23 / 0.023|8.06 / 0.008|128.61 / 0.129|
|xalan 32,768–65,536|75|3.33 / 0.265|11.58 / 0.884|260.73 / 29.771|
|xalan 65,537–1,048,576|156|3.30 / 0.553|11.63 / 1.826|392.03 / 111.235|
|xalan >1,048,576|12|3.57 / 0.043|11.49 / 0.134|543.72 / 8.051|
|**xalan, all packed merges**|**243**|**3.30 / 0.861**|**11.62 / 2.839**|**350.57 / 149.076**|

Five-process medians of per-process sums and percentiles. xalan p95 per packed merge: 4.10 µs device scan, 15.71 µs post-scan round trip, 1,998 µs inclusive host wait. The cold scan's 23.23 µs unprofiled is not the 61.66 µs cold-cache NCU interval. It is the synchronization dependency on the preceding work, not copying 24 bytes, that is material to host availability. The packed kernel count in this history is **927** (45 merges × 3 and 198 × 4), excluding plain-owner sealing. Other small plain merges have no sizing pass. Size classes here describe one merge; the admission-class table below classifies a whole admission by its aggregate merged rows.

### xalan gate failure and attribution

The earlier cold improvements are **not acceptable on their own**. Default replay results below (ms, one directly verified admission process at each prior commit) expose two independent regressions. The first small increase already occurs with change 1 against the nominal 45.0 ms gate. Change 2's owner-assigned writer introduces the first large jump; change 3's one-lane-per-owner planner introduces another. Later writer changes did not fix skewed real admissions. In change 7 the default run took 211.76 ms, not 45 ms; diagnostic runs took 218.57 ms median. The 149 ms diagnostic host blockage is real waiting, not an instrumented run turned 5× slower.

|Commit/change|Whole C admission|C merge publication (all 1,314 merges)|GPU analyze|GPU pack|
|---|---:|---:|---:|---:|
|28735929 / 1|165.15|45.64|11.14|15.93|
|9a433e6d / 2|268.29|145.20|11.12|115.12|
|c93e3809 / 3|404.06|283.88|150.51|116.42|
|da922fb7 / 4|363.81|243.43|150.37|76.55|
|b93408ab / 5|451.74|332.71|150.15|166.04|
|d42c07e0 / 6|329.99|211.27|150.04|44.61|
|42483a3e / 7|330.47|211.76|150.27|44.95|

Accounting-only replay, five clean default processes: 42483a3e C publication **211.885 ms**, of which **194.387 ms** publishes packed leaves (243 of 1,314 merges), whole C admission **330.827 ms**. Fresh five-process 0fa2fccc medians: C publication **44.756 ms**, analyze 11.089 ms, pack 15.577 ms, whole admission 163.620 ms. This comparison times the same trace and storage policy, not a synthetic extrapolation.

Full NCU on *actual* xalan plan invocations (0-based packed-merge ordinals, same trace order): #110, **101,500 output rows**, **2 × 128 threads**, **3.848 ms**, 3.3% occupancy, 15.3% issue, **2.39 lanes**, 51.3% wait; #165, **1,085,127 rows**, **18 × 128**, **2.719 ms**, 3.7% occupancy, 12.8% issue, **5.34 lanes**, 40.5% wait. Both use 80 registers. The smaller merge is slower: not fixed launch cost or constant cost per output row. A lane scans a potentially huge incoming interval for each column; sibling lanes can finish much earlier. Only two SMs can even host the smaller launch.

## Change 8: parallelize skewed intervals, preserve cold throughput

[INFERENCE, before implementation] Plain incoming or previously merged variable leaves do not justify the balanced fixed/fixed assumption. Give every owner a CTA to reduce per-column extrema across its interval, and distribute skewed writing by *output* leaf, not by owner. Keep the tiled scan and fast sparse fixed/fixed path. Expect representative planner intervals below 0.15/0.25 ms, ≥20 active lanes on long intervals, default xalan merge ≤45.0 ms; cold 67M retains <1.33 ms full-sum and >500 GB/s common writer. Profile and gate both before commit.

Measured: the dispatch criterion is *grid size*, not the replay ID or input representation. With ≤16,384 nonempty owner blocks, the previous 128-thread lane-per-owner schedule has at most 128 CTAs. Give each owner a 128-thread CTA instead. Lanes read plain rows at unit stride across the warp; packed intervals use contiguous subintervals and reduce extrema. Exact output-leaf offsets then let each writer CTA find its owner and both co-ranks independently. Long splits no longer serialize on an owner CTA. Above this threshold, retain the sparse lane schedule. No change to leaf format, exact sizing, tie order, readers or storage policy.

|Actual xalan planner|Before → after CTAs × 128|Planner ms|Registers|Occupancy %|Issue %|Lanes/instruction|
|---|---:|---:|---:|---:|---:|---:|
|101,500 output rows|2 → 175|3.848 → **0.115**|80 → 94|3.3 → 10.1|15.3 → 16.2|2.39 → 26.98|
|1,085,127 output rows|18 → 2,223|2.719 → **0.141**|80 → 94|3.7 → 36.6|12.8 → 53.1|5.34 → 27.82|

One planning launch per merge before and after. Each small packed publication is now **three launches** (cooperative plan, exact offsets, independently assigned output leaves), not three or four. Across xalan, 243 × 3 = **729** instead of 927, excluding adoption/sealing and plain merges. In five clean default runs the C merge gate is **37.974 ms** (0fa2fccc: 44.756 ms, cap: 45.0 ms); **20.490 ms** of that is packed-only publication. C analyze/pack = **7.716/13.747 ms**, whole C admission = **157.236 ms**. GPU analyze/pack as recorded by admission events include some plain publications as well; they do not partition the packed-only host interval.

### Final cold hardware counters (change 8)

|Launch, state 90, 2^26, 1:16|ms|DRAM MB|GB/s|Registers/thread|Occupancy %|Issue %|Lanes/inst|
|---|---:|---:|---:|---:|---:|---:|---:|
|E partition|0.077|16.1|209|19|16.0|2.1|31.73|
|E merge|1.249|1,025.3|821|38|94.0|21.0|31.47|
|P plain-storage merge|2.738|1,062.1|388|33|92.0|21.1|30.97|
|C plan (partition fused)|0.151|40.5|268|80|40.9|46.7|24.88|
|C tile offsets|0.060|0.2|3|40|16.5|16.2|31.74|
|C sparse writer|0.511|285.5|559|48|71.2|67.0|26.02|
|C compact writer|0.224|11.4|51|80|46.1|38.3|28.76|
|C wide writer|not launched|—|—|—|—|—|—|

**Packed kernel sum 0.947 ms; E 1.326 ms.** Historical full packed sums from fresh captures, including the easily missed leaf `offsets` kernel: 8.053 ms baseline → 7.769 → 6.260 → 2.348 → 2.284 → 1.686 → 1.074 → 0.943 → 0.947 ms. Input sealing is outside all these sums. Common writer targets >500 GB/s and ≤64 registers pass; >28 lanes fails (26.02), for the shift-predication reason above. The compact kernel clears the lane target but not the bandwidth/register targets: 33% long-scoreboard and 27% barrier samples. The original writer was 3.491 ms, 336.8 MB, 96 GB/s, 72 registers, 41.3% occupancy and 14.90 lanes.

### Reader residency control (unmodified reader sources)

Full NCU, 65,536 four-word keys with one prefix per row to exercise the actual 256-thread low-fanout `packed_serial_scan` specialization: source registers **120 → 122**, `launch__registers_per_thread_allocated` **120 → 128**. That is 30,720 → 32,768 registers per CTA. `launch__occupancy_limit_registers` remains **2 CTAs/SM**; `sm__maximum_warps_avg_per_active_cycle` **16 → 16**, `sm__maximum_warps_per_active_cycle_pct` **33.33 → 33.33%**. Different register-allocation buckets, same residency band. `cuobjdump` on replay executables: packed membership registers unchanged [40,48,63,64], parallel scan [117,120,126,128] → [117,120,122,126], serial scan [101,108,114,120] → [101,108,114,122]. All plain scan and membership resource counts unchanged.

### Default xalan admission and merge publication

Five fresh clean processes (`c8-admission`); per-process class sums divided by merged rows. A class buckets the total merged rows at one admission; an admission may publish several runs. E/P/C have the same work counts.

|Merged rows per admission|Merges|Merged rows|E ns/row|P ns/row|C ns/row|
|---|---:|---:|---:|---:|---:|
|1–256|420|22,652|233.193|289.230|290.470|
|257–4,096|377|323,220|14.443|18.833|18.850|
|4,097–65,536|279|3,894,086|0.970|1.275|1.890|
|65,537–1,048,576|190|26,560,805|0.113|0.154|0.481|
|>1,048,576|48|27,314,294|0.039|0.069|0.191|

Xalan merge publication E/P/C **17.771 / 23.587 / 37.974 ms**. C loses to E in every merge size class. Whole admission E/P/C **182.698 / 139.085 / 157.236 ms**; C beats E by 25.462 ms over the full history, but not P. These are host publication/admission intervals, not the cold-kernel NCU sums.

### Final exact sizing round trip

Same diagnostic definition as change 7; five clean processes, absolute size (no reserved slack), one 24-byte transfer per packed merge, no transfer on 1,071 plain publications.

|Case (individual merge output)|Merges|Device scan p50 µs / sum ms|Post-scan round trip p50 µs / sum ms|Inclusive host wait p50 µs / sum ms|
|---|---:|---:|---:|---:|
|Cold 2^26, 90, 1:16|1|22.53 / 0.023|9.92 / 0.010|129.50 / 0.129|
|xalan 32,768–65,536|75|3.30 / 0.266|10.75 / 0.853|14.96 / 1.326|
|xalan 65,537–1,048,576|156|4.10 / 0.642|11.55 / 1.846|22.97 / 4.372|
|xalan >1,048,576|12|10.11 / 0.123|12.53 / 0.148|63.84 / 0.787|
|**xalan all packed**|**243**|**4.10 / 1.030**|**11.52 / 2.862**|**22.45 / 6.485**|

The transfer/host resubmission interval is material at small sizes: 2.862 ms across the history, not 149 ms. Fixing plan imbalance reduces inclusive host blockage from 149.076 to **6.485 ms** without eliminating the download. Xalan p95 scan/post-scan/inclusive wait are 7.17/15.65/63.71 µs. Five-process default C merge/packed-only publication were 37.974/20.490 ms; with sizing markers they were 44.724/27.328 ms: the diagnostic itself is not free. The cold device scan was 0.060 ms under full cold-cache NCU, 0.023 ms in the marker run.

Exactness after the scheduling change: 43/43 GPU tests (including a 21,209-row case crossing the scheduling threshold), both synthetic storage oracles, all xalan admissions, matched-history real reads, scaled reads and full-output 2^28 1:16 merges on both states passed. Repository-root formal inventory passed (523 rules, 522 registered decode-site comments). No operation-path whole-key reconstruction was added.

### Scaled reader gate against 0fa2fccc

Five fresh clean default-build processes per cell; C GPU ns/probe, baseline → final, 268,435,456 rows. The first two columns are the ordinary one-run scaled workload. The last two set `PACKED_READ_LEAVES=1` to build and read merged leaves. A 2% slowdown in any workload would fail. All twenty comparisons passed; the largest slowdown is 1.39%. Published leaf sizes are identical before and after: state 90 676,255,744 bytes, state 112 675,871,380 bytes.

|Workload|Fixed 90|Fixed 112|Merged leaves 90|Merged leaves 112|
|---|---:|---:|---:|---:|
|Sorted prefix|0.11661 → 0.11659|0.20863 → 0.20480|0.12687 → 0.12711|0.22114 → 0.22093|
|Shuffled prefix|0.76126 → 0.76145|1.17922 → 1.16502|0.78728 → 0.78383|1.23026 → 1.22886|
|Mixed prefix|0.76091 → 0.75819|0.99633 → 0.98855|0.79497 → 0.78884|1.00688 → 1.01327|
|Range scan|2.01570 → 2.01467|17.98565 → 17.97086|2.06443 → 2.06081|17.45287 → 17.45221|
|Membership|0.47076 → 0.47327|0.50987 → 0.51180|0.46178 → 0.46238|0.49982 → 0.50679|

### Cold default-build E/P/C

GPU event ns per **output row**, from `merge_scale`; each cell is a median of five fresh processes with clean before/after GPU checks. All three are checked against the exact sorted output. E = engine, P = new set with plain storage, C = packed. These are not sums of the much slower per-kernel cold-cache NCU replays.

|Rows|State|Incoming fraction|E|P|C|C vs E|
|---:|---:|---:|---:|---:|---:|---|
|2^26|90|1/16|0.02246|0.02658|0.01091|faster|
|2^26|90|1/2|0.02287|0.03377|0.02515|slower|
|2^26|112|1/16|0.04375|0.04744|0.01184|faster|
|2^26|112|1/2|0.04910|0.05605|0.02815|faster|
|2^28|90|1/16|0.02133|0.02950|0.00964|faster|
|2^28|90|1/2|0.02319|0.04586|0.02262|faster|
|2^28|112|1/16|0.04296|0.05759|0.01038|faster|
|2^28|112|1/2|0.04662|0.07815|0.02526|faster|

C beats E in **seven of eight** cases. An independent second five-process admission set (`gate8-admission`) also passes: C merge 37.865 ms, including 20.497 ms packed-only, whole C admission 156.947 ms.

### Matched-history real reads

Five fresh clean processes on each executable, actual merged leaves, 222,416,739 probes and 69,170,748 enumerated rows. Summed site GPU times E/P/C: **152.423 / 145.479 / 183.198 ms** at 0fa2fccc → **152.260 / 145.401 / 183.031 ms** at bb177e7596. C changed **−0.09%**, passing the 2% gate. C stratified diagnostic changes: long range +0.15%, membership miss −0.13%, one-row hit +0.01%, range miss −0.05%, short range +0.13%; these diagnostic intervals are not additive. Matched-snapshot footprint remains 22.913 bits/row.
