# Mid-size packed merge

Baseline `2498e967`; raw artifacts and per-run GPU tenant checks: `/data/design-notes/doop-memory/merge-midsize`. E/P/C denote engine, new plain, new packed. The cold benchmark seals both inputs even at 32,768 rows; ordinary admission can retain plain storage through that size.

## Prediction before editing

[INFERENCE] The discontinuity is the choice of *owner block count*, not output row count. Through 16,384 owner blocks the old code launches one 128-thread CTA per owner to plan, then one CTA per output leaf for the combined compact/wide writer. Beyond 16,384 blocks it plans 128 owners per CTA, classifies each, and dispatches sparse splices and compact/wide exceptions separately. Both paths use exact sizing. On the small path the CTA repeats both fence searches, reduces each column with two barriers, and may run a retirement atomic and one-CTA scan (output ≤131,072 and pool lane identical). The output writer searches the entire prefix table before handling even a single unchanged leaf. The large sparse writer requires retained encoding/frame, packed width ≤32 bits, ≤512 output rows and ≤16 incoming rows; a plain bootstrap cannot splice packed words.

[INFERENCE] Compute each fence cut once; summarize aligned incoming groups once and reduce each owner's two partial groups plus complete groups in a warp. This bounds the skewed planner tail without cooperative CTA barriers or retirement, and keeps exact inherited-frame widening. Dispatch writers by actual plan class on both schedules. Expect largest gains at 2²⁰–2²² for sparse deltas, and shorter in-sequence planners for skewed inputs. Check both scan/launch overhead and register spilling against the eight large endpoints; do not infer elapsed publication from kernel sums.

## First candidate: tiled extrema

Three clean xalan processes: E/P/C publication **17.661 / 16.213 / 29.742 ms**, packed-only **17.885 ms**, versus my baseline **17.732 / 16.422 / 31.252 ms** and **19.061 ms** packed-only. Both publish 115,726,164 bytes. Baseline → candidate in-sequence median spans **10.144/6.656/57.120 → 10.081/6.689/62.977 µs**: host publication improved, median device span did not. Planner kernel sums **4.834 → 1.070 + 1.169 ms**; scans **.324 → .554 ms**; writers **5.542 → 5.348 + 1.050 ms**. In the 101,500-row scope, partition/plan/scan were **5.54/8.48/1.50 µs** vs **40.58 µs** previously, but separate sparse/compact writers took **9.73/33.95 µs**. In the 1,780,906-row scope those five launches took **7.30/5.92/9.47/10.27/65.34 µs**. The old 40 µs is not an intrinsic CUDA launch floor; the new implementation spends it differently.

Reject this candidate: at 2²², 90, 1/16 the single-CTA scan alone takes **100.86 µs** under full NCU, vs 15.36/35.26 µs for the two planning kernels. The first 2²⁶, 90, 1/16 endpoint also regressed. The real admission schedules often have only a few spliceable owners. Replaying actual keys and frames on CPU gave median **0.87%** sparse; only **5/243** merges have at least half sparse owners. Maximum single-owner incoming interval ranged up to **45,129** rows (median maximum **1,593**); switching all owners to a serial loop is unsafe for latency.

[INFERENCE, next prediction] Group owner prefixes within a CTA before the global scan, and send infrequent sparse leaves through the compact launch instead of scheduling a second mostly empty grid. Search each interior split with parallel comparisons, not a serial dependent binary search replicated in every lane. Specialize the known-unsplit compact writer at large sizes: equal owner/output block counts prove no interior split exists. This should lower small scan and writer time and undo the large endpoint regression without relaxing the register limit.

## What changes at the step

For a 256-row fixed owner, 16,384 owner blocks means **4,194,304 owner rows**. In this replay `owner = output - floor(output/(denominator+1))`; the first sparse-schedule output is **4,456,449** for 1/16 or **6,291,457** for 1/2. For a variable owner the condition instead counts actual headers; a plain owner uses `ceil(rows/block_rows)`. On the old small schedule a plain owner just bootstraps metadata, but the large sparse splicer requires encoded words and seals a plain owner. Neither owner representation nor the 32,768-row plain-admission cutoff itself is the source of the cold step. All 2¹⁹–2²⁴ survey inputs already have **two sealed fixed runs**.

My NCU state 90, 1/16: doubling 2²²→2²³ reduces old planning from **523.33 to 41.86 µs**. Grid drops from **15,421 to 241** 128-thread CTAs; registers fall **96→80**, active warps **40.2→13.6%**, lanes/instruction **23.5→25.1**. High occupancy by itself was not useful work: the old CTA duplicates both dependent searches and performs per-column reductions even when only 16 incoming rows need them. The combined writer was **386.75 µs, 124 regs** (29.5% long-scoreboard, 26.7% barrier samples); its replacements were **76.32 µs, 48 regs** (sparse) and **62.02 µs, 80 regs** (compact exceptions). In the 1/2 survey only the compact writer is needed on the large schedule. The retirement atomic **does not run at either side of this step**. The serial one-CTA scan does run on both, but its input shrinks from one record per owner to one per warp of owners.

The four real scopes are not four samples of a universal 40-µs floor. Measured baseline planner median across all 243 was **12.90 µs**. Matching the actual interval lengths to the captured planner times:

|Largest incoming interval per owner in one merge|Merges|Median output rows|Median owner blocks|Median old plan µs|Median tiled partition + plan µs|
|---|---:|---:|---:|---:|---:|
|1–512|28|83,066|171.5|8.29|6.82|
|513–2,048|118|84,973|177|10.74|7.98|
|2,049–8,192|84|144,499|283|18.46|9.98|
|>8,192|13|249,139|545|47.36|13.92|

The 101,500-row example has only 175 owners but **7,544 incoming rows for one owner**; at 1,780,906 rows there are 3,801 owners and the longest interval is 4,927. Both have packed incoming runs. A warp-independent serial walk of source frames inside one owner, including fallback to exact residual extrema on widening, lies on the critical path. This association does not isolate the cost of each instruction; staged extrema change the work decomposition, not just the launch shape.

At 2²⁶, 90, 1/16 my (plan / offsets / sparse / compact) full-counter device times were **148.96/60.16/511.14/223.52 µs** baseline, **140.51/62.43/511.65/243.10 µs** under the tiled register cap. The compact writer's 32-byte local load/store sectors rose from **4,955,794/5,311,984** to **8,374,350/8,430,384**. The sparse writer had zero local sectors both times. Simply forcing 64 registers without removing dead split logic is not a fix for the large endpoint.

### Counterfactual scheduling experiment

[INFERENCE, before measurement] The fixed-block interleaved cold inputs are deliberately low-skew. A short lane-per-owner grid should win earlier for the 1/16 delta than for 1/2, whereas the long real intervals need incoming-group parallelism. Build a second executable with **only the schedule predicate inverted**, run the same full-output cold oracle at 2¹⁹–2²⁴, both states/deltas, and compare with the normal predicate. This is not a production environment override.

## Full-counter baseline sweep

Each entry is `kernel device µs/registers per thread/grid CTAs/block threads/active warps %/lanes per instruction`. The two successive `part, merge` pairs are E and P; `tile_offsets` belongs to C (captured separately from the templated kernels). The order shown is report order, not execution order. Full Nsight Compute replay controls cache/clock; use the unprofiled event map, not the sum of these times, for crossover.

|log₂ rows|State|Delta/owner|Kernel measurements|
|---:|---:|---:|---|
|19|90|1/16|plan_cooperative 74.3/96/1928/128/36.8/23.1; write_balanced 60.35/124/1928/128/30.6/28.1; part 12.1/19/1/256/10.7/28.2; merge 15.33/38/147/512/33.4/31.5; part 12.06/19/1/256/10.8/28.2; merge 15.65/38/147/512/33.4/31.5; tile_offsets 15.46/40/1/256/16.5/31.6|
|19|90|1/2|plan_cooperative 56.9/96/1366/128/36.9/31.4; write_balanced 49.57/124/1366/128/28.8/30.1; part 13.6/19/1/256/10.5/27.9; merge 16.13/38/147/512/33.0/31.2; part 13.73/19/1/256/10.3/27.9; merge 16.1/38/147/512/33.6/31.2; tile_offsets 12.13/40/1/256/16.6/31.6|
|19|112|1/16|plan_cooperative 90.5/94/1928/128/36.9/23.2; write_balanced 64.99/128/1928/128/30.9/27.9; part 13.41/22/2/256/11.6/29.8; merge 22.37/32/342/512/70.2/30.6; part 13.38/22/2/256/11.8/29.8; merge 22.34/32/342/512/69.9/30.6; tile_offsets 15.33/40/1/256/16.4/31.6|
|19|112|1/2|plan_cooperative 68.64/94/1366/128/37.1/31.4; write_balanced 52.48/128/1366/128/29.4/30.1; part 15.68/22/2/256/11.4/29.8; merge 23.78/32/342/512/69.8/31.2; part 15.68/22/2/256/11.6/29.8; merge 23.68/32/342/512/70.0/31.2; tile_offsets 12.61/40/1/256/16.5/31.6|
|20|90|1/16|plan_cooperative 137.98/96/3856/128/38.5/23.3; write_balanced 103.9/124/3856/128/31.8/28.2; part 13.5/19/2/256/10.5/28.9; merge 22.34/38/293/512/59.6/31.5; part 13.22/19/2/256/10.8/28.9; merge 22.02/38/293/512/59.6/31.5; tile_offsets 27.81/40/1/256/16.5/31.7|
|20|90|1/2|plan_cooperative 105.98/96/2731/128/38.1/31.4; write_balanced 81.41/124/2731/128/31.4/30.1; part 15.39/19/2/256/10.8/28.9; merge 24.16/38/293/512/60.3/31.2; part 15.46/19/2/256/10.8/28.9; merge 23.65/38/293/512/60.4/31.2; tile_offsets 20.35/40/1/256/16.4/31.7|
|20|112|1/16|plan_cooperative 168.26/94/3856/128/38.7/23.4; write_balanced 110.98/128/3856/128/31.8/28.0; part 14.62/22/3/256/14.6/30.4; merge 33.41/32/683/512/83.9/30.6; part 14.53/22/3/256/14.8/30.4; merge 33.54/32/683/512/82.9/30.6; tile_offsets 28.29/40/1/256/16.5/31.7|
|20|112|1/2|plan_cooperative 129.76/94/2731/128/38.1/31.4; write_balanced 86.18/128/2731/128/31.4/30.2; part 16.51/22/3/256/14.5/30.4; merge 34.24/32/683/512/85.2/31.2; part 16.64/22/3/256/14.4/30.4; merge 34.43/32/683/512/85.3/31.2; tile_offsets 20.42/40/1/256/16.5/31.7|
|21|90|1/16|plan_cooperative 265.82/96/7711/128/39.6/23.4; write_balanced 198.78/124/7711/128/32.2/28.3; part 14.34/19/3/256/13.2/30.2; merge 32.8/38/586/512/86.3/31.5; part 14.24/19/3/256/13.4/30.2; merge 33.5/38/586/512/85.1/31.5; tile_offsets 51.07/40/1/256/16.6/31.7|
|21|90|1/2|plan_cooperative 199.14/96/5462/128/39.2/31.4; write_balanced 154.02/124/5462/128/31.9/30.2; part 16.29/19/3/256/13.4/29.9; merge 34.3/38/586/512/85.9/31.2; part 16.83/19/3/256/13.2/29.9; merge 34.62/38/586/512/85.5/31.2; tile_offsets 38.46/40/1/256/16.6/31.7|
|21|112|1/16|plan_cooperative 325.82/94/7711/128/39.9/23.5; write_balanced 212.32/128/7711/128/32.2/28.1; part 15.68/22/6/256/14.8/31.1; merge 53.34/32/1366/512/89.0/30.6; part 15.84/22/6/256/14.7/31.1; merge 54.14/32/1366/512/88.4/30.6; tile_offsets 51.26/40/1/256/16.6/31.7|
|21|112|1/2|plan_cooperative 245.25/94/5462/128/39.4/31.4; write_balanced 162.78/128/5462/128/31.9/30.2; part 18.46/22/6/256/14.7/30.9; merge 54.98/32/1366/512/89.2/31.2; part 18.37/22/6/256/14.9/30.9; merge 54.75/32/1366/512/90.5/31.2; tile_offsets 38.98/40/1/256/16.6/31.7|
|22|90|1/16|plan_cooperative 523.33/96/15421/128/40.2/23.5; write_balanced 386.75/124/15421/128/32.5/28.4; part 15.62/19/5/256/15.3/30.9; merge 52.54/38/1171/512/92.3/31.5; part 16.03/19/5/256/15.1/30.9; merge 53.22/38/1171/512/87.6/31.5; tile_offsets 98.21/40/1/256/16.7/31.8|
|22|90|1/2|plan_cooperative 388.19/96/10923/128/40.0/31.4; write_balanced 294.24/124/10923/128/32.4/30.2; part 18.05/19/5/256/15.0/31.0; merge 53.28/38/1171/512/90.5/31.2; part 18.18/19/5/256/14.8/31.0; merge 55.01/38/1171/512/90.1/31.2; tile_offsets 70.11/40/1/256/16.7/31.7|
|22|112|1/16|plan_cooperative 644.7/94/15421/128/40.4/23.6; write_balanced 409.66/128/15421/128/32.5/28.2; part 18.27/22/11/256/15.7/31.1; merge 104.03/32/2731/512/87.2/30.6; part 18.34/22/11/256/15.6/31.1; merge 104.77/32/2731/512/87.3/30.6; tile_offsets 98.02/40/1/256/16.7/31.8|
|22|112|1/2|plan_cooperative 480.64/94/10923/128/40.2/31.4; write_balanced 314.59/128/10923/128/32.5/30.2; part 21.41/22/11/256/15.8/31.2; merge 105.66/32/2731/512/87.7/31.2; part 21.92/22/11/256/15.5/31.2; merge 106.98/32/2731/512/87.3/31.2; tile_offsets 70.98/40/1/256/16.7/31.7|
|23|90|1/16|plan 41.86/80/241/128/13.6/25.1; write 76.32/48/3856/128/65.0/26.0; write_compact 62.02/80/3856/128/36.0/28.8; part 17.98/19/10/256/15.1/30.9; merge 103.1/38/2341/512/90.6/31.5; part 18.08/19/10/256/15.1/30.9; merge 102.91/38/2341/512/88.4/31.5; tile_offsets 10.34/40/1/256/16.4/31.5|
|23|90|1/2|plan 189.76/80/171/128/9.6/20.6; write_compact 390.4/80/2731/128/44.6/29.7; part 20.8/19/10/256/15.2/31.0; merge 105.54/38/2341/512/88.2/31.2; part 21.25/19/10/256/15.2/31.0; merge 104.67/38/2341/512/88.0/31.2; tile_offsets 8.42/40/1/256/16.2/31.4|
|23|112|1/16|plan 54.08/80/241/128/12.9/24.6; write 76.1/48/3856/128/66.1/26.4; write_compact 96.03/80/3856/128/34.8/28.4; part 25.54/22/22/256/15.6/31.2; merge 258.78/32/5462/512/89.3/30.6; part 25.09/22/22/256/15.7/31.2; merge 255.94/32/5462/512/90.3/30.6; tile_offsets 10.27/40/1/256/16.6/31.5|
|23|112|1/2|plan 267.26/80/171/128/9.0/19.5; write_compact 435.65/80/2731/128/44.5/29.8; part 32.86/22/22/256/15.5/31.4; merge 260.38/32/5462/512/90.6/31.2; part 32.51/22/22/256/15.4/31.4; merge 259.78/32/5462/512/91.5/31.2; tile_offsets 8.48/40/1/256/16.3/31.4|
|24|90|1/16|plan 51.81/80/482/128/26.0/25.0; write 136.83/48/7711/128/68.0/26.0; write_compact 76.42/80/7711/128/41.6/28.8; part 23.65/19/19/256/15.6/31.0; merge 256.58/38/4682/512/91.7/31.5; part 24.06/19/19/256/15.6/31.0; merge 256.99/38/4682/512/90.7/31.5; tile_offsets 17.66/40/1/256/16.4/31.6|
|24|90|1/2|plan 222.59/80/342/128/18.7/20.6; write_compact 689.12/80/5462/128/47.2/29.7; part 29.98/19/19/256/15.7/31.3; merge 261.18/38/4682/512/91.2/31.2; part 29.86/19/19/256/15.6/31.3; merge 257.06/38/4682/512/92.2/31.2; tile_offsets 13.98/40/1/256/16.2/31.6|
|24|112|1/16|plan 69.86/80/482/128/24.6/24.5; write 136.22/48/7711/128/69.2/26.4; write_compact 131.23/80/7711/128/41.1/28.4; part 43.71/22/43/256/16.0/31.1; merge 586.4/32/10923/512/91.5/30.6; part 43.52/22/43/256/15.9/31.1; merge 590.91/32/10923/512/90.7/30.6; tile_offsets 17.63/40/1/256/16.4/31.6|
|24|112|1/2|plan 293.41/80/342/128/16.4/19.5; write_compact 792.29/80/5462/128/47.2/29.8; part 58.78/22/43/256/15.9/31.4; merge 594.72/32/10923/512/90.8/31.2; part 58.72/22/43/256/15.8/31.4; merge 595.49/32/10923/512/91.4/31.2; tile_offsets 13.89/40/1/256/16.6/31.6|

## Grouped prefixes and split scheduling

Three clean processes: xalan E/P/C publication **18.089/16.491/28.448 ms**, C packed-only **16.325 ms**, unchanged **115,726,164 published bytes**. The 243 in-sequence medians are **10.144/6.688/54.880 µs**; C wins none. Bounding the search and folding infrequent splices reduced compact writer time from **5.348 to 4.299 ms** and sparse writer time from **1.050 to .037 ms** (138 launches → 5). Counts: 243 partition/summaries, 243 team plans, 243 offset scans, 243 compact writers, 5 sparse writers, 68 bootstrap analyzes. At 2²⁶, 90, initial sparse/dense endpoints **.01065/.02108 ns/row** vs baseline **.01070/.02496**.

[INFERENCE, next prediction] Small grouped merges still retire separate scratch allocations; the four measured host scopes each record eleven CUDA events. The exact D2H transferred 24 bytes in .96–.99 µs, yet the next writer submission began 17.0–27.1 µs after the scan launch API returned. Use one aligned, call-owned pool workspace for temporaries and bootstrap, and fuse the scan only when the grouped tile counts fit one 128-thread scan iteration. Count cache eviction, allocation and readback pauses in the comparison; no oversized published allocation or alternate reader. Expect fewer retirement events, fewer launches on the 32K–128K outputs, unchanged bytes, and lower real and small-cold wall times.

### Retired tiled experiment: all cold endpoints

E/P/C event ns per output row; each entry is a median of three clean processes, with the sorted full-output oracle. These are measurements from this investigation, not numbers copied from the previous report.

|log₂ rows|State|Delta/owner|base|tiled|
|---:|---:|---:|---:|---:|
|26|90|1/16|0.02236/0.02164/0.01070|0.02240/0.02167/0.01158|
|26|90|1/2|0.02283/0.02222/0.02496|0.02289/0.02239/0.03157|
|26|112|1/16|0.04382/0.04309/0.01181|0.04387/0.04311/0.01303|
|26|112|1/2|0.04999/0.04831/0.02809|0.04997/0.04952/0.03554|
|28|90|1/16|0.02137/0.02117/0.00961|0.02139/0.02121/0.01009|
|28|90|1/2|0.02351/0.02314/0.02258|0.02334/0.02335/0.02832|
|28|112|1/16|0.04299/0.04283/0.01022|0.04289/0.04273/0.01098|
|28|112|1/2|0.04648/0.04628/0.02515|0.04639/0.04630/0.03241|

## First-use gap and sparse cutoff prediction

In clean cold Nsight Systems captures at 2¹⁵/2²⁰/2²¹, 90, 1/16, the single-workspace version spends **26.50/66.94/87.58 µs** executing C kernels. The first kernel starts **31.8/31.5/33.3 µs** into the host merge range. At 2¹⁵, kernels run for only 26.50 µs despite a 127.67-µs host range; this includes first-use host preparation, not 100 µs of fixed device work. The next launch API starts 17.7 µs after the first launch API ends at that size; there is no intervening GPU kernel. At 2²¹ the four stages after partition take **20.26/5.47/19.14/35.23 µs** (plan/scan/splice/compact).

[INFERENCE, predicted before dispatch change] In the forced comparison state 90, 1/16 at 2²⁰ and 2²¹ takes **.1162/.0610** on the lane schedule vs **.1328/.0650** on grouped; 1/2 at 2²⁰ goes the other way (**.1787 vs .1494**). Both states have short interleaved fixed owners on cold input, but no real merge has two fixed sealed operands. Allow sparse fixed/fixed inputs onto the existing lane path above 2,048 owners when the mean incoming interval fits its 16 insertion lanes; do not apply this heuristic to bootstrap or variable leaves. Expect to move the 1/16 crossover without changing real-history dispatch. The owner mean is a cost estimate, not a correctness assumption or a bound on the longest interval.

### Exactness and the real critical path

A `PACKED_LEAF_SIZING=1` admission run on both 2498e967 and the selected binary returned identical **(output rows, payload words, output leaves) for all 243 pairs**, not just equal final totals. Allocation remains exact after a 24-byte size/class readback. The bootstrapped source and temporary arrays are views of one stream-owned pool allocation, not a global buffer; the small-grid retirement flag is per invocation. The fused scan is restricted to at most 128 four-owner tiles (512 owners). Larger tile sets use the standalone offset kernel. Compact and wide output leaves use different kernels, so the compact writer never statically pays for the wide path.

When summing every packed merge in one Nsight Systems trace, baseline / grouped / single-workspace C spend **10.922/7.303/7.366 ms** actually executing kernels and **6.529/8.131/7.156 ms** in the gaps *between* those kernels. The first CUDA launch API starts a median **8.47/8.85/7.50 µs** after host merge entry. The median per-merge sum of kernel durations is **31.10/24.86/24.90 µs**. These quantities are different from both publication and the first-to-last device span. The workspace principally removes inter-kernel and host cost; attributing its gain to increased DRAM bandwidth would give the wrong explanation.

## Reproduction and units

All timings here are from the local GPU. Build default and NVTX binaries separately; the sorted-output oracle is enabled on every cold invocation. Use packed-code-space and packed-run tests with `--test-threads=1`, plus formal, synthetic, real read, scaled and admission oracles. Exact commands, stdout, per-process pre/post tenant checks and captures are retained under `/data/design-notes/doop-memory/merge-midsize/{build.sh,run.py,real-ncu.py,*-results}`. Test both fixed and variable layouts for scaled readers.

```sh
source /dev/shm/gpu-env.sh
R=/data/design-notes/doop-memory/merge-midsize
# From experiments/gpu:
flock -s -w 900 /tmp/mini_linq-timing.lock cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release -p gpu-datalog-runtime --bin packed-replay
# Copy the executable to $R/selected. The runner takes the exclusive GPU lock
# and rejects occupied pre/post lists, including for nsys and ncu captures.
cd "$R"
python3 run.py selected sweep 3       # 2^15..2^26, states 90/112, both deltas
python3 run.py selected endpoints 3   # includes 2^28
python3 run.py selected ncu           # ncu --set full, every C stage, 2^19..2^24
python3 report.py matrix selected     # medians, monotonic absolute time, sustained wins
```

Device-event ns/row × output rows / 1,000 gives absolute µs. A crossover below means the first exponent that beats E **and all larger measured exponents through 26 also beat E**. Publication classes aggregate host merge timers by admission `merged_rows`; unlike individual merge spans an admission can publish multiple merged runs. For replicated Nsight Systems spans, join kernels through launch correlation IDs, match (sequence, merge index, state, left, right) across all three traces, then median the three samples *per merge* before taking size-class medians. This avoids pooling unrelated merges or comparing isolated cold events to cache-resident in-sequence spans.

## Measured comparison at handoff

Each cold cell below is E/P/C device-event ns/output row; parentheses give **completed clean processes**, not inner-loop repetitions. A dash is unmeasured, not zero. Only cells with (3) are acceptance medians. All four baseline sustained-crossing decisions use three samples at exponents 23, 24 and 26; some adjacent exponents have two and are provisional.

### Baseline cold map

|log₂ rows|90 1/16|90 1/2|112 1/16|112 1/2|
|---:|---:|---:|---:|---:|
|15|3.0937/1.0938/3.2812 (3)|2.9688/1.1562/3.4688 (3)|2.9688/1.3467/3.5596 (3)|2.8750/1.2812/3.6611 (3)|
|16|1.4546/0.6243/1.6887 (2)|1.4936/0.6267/1.7263 (2)|1.4602/0.6416/1.7583 (2)|1.5937/0.6233/1.7900 (2)|
|17|0.7148/0.3130/0.8794 (2)|0.7188/0.3164/0.8789 (2)|0.7174/0.3779/0.9014 (2)|0.7500/0.3619/0.9337 (2)|
|18|0.3809/0.2264/0.4767 (2)|0.3671/0.2306/0.4409 (2)|0.3787/0.2263/0.4921 (2)|0.3785/0.2246/0.4517 (2)|
|19|0.2051/0.1230/0.2710 (3)|0.1890/0.1225/0.2578 (3)|0.2090/0.1328/0.2908 (3)|0.2070/0.1361/0.2867 (3)|
|20|0.1084/0.0752/0.1865 (3)|0.1064/0.0723/0.1608 (3)|0.1307/0.0936/0.2090 (3)|0.1394/0.0956/0.1787 (3)|
|21|0.0688/0.0508/0.1429 (3)|0.0670/0.0497/0.1196 (3)|0.0848/0.0668/0.1558 (3)|0.0869/0.0674/0.1294 (3)|
|22|0.0439/0.0356/0.1174 (3)|0.0438/0.0348/0.0959 (3)|0.0647/0.0527/0.1312 (3)|0.0657/0.0557/0.1071 (3)|
|23|0.0322/0.0270/0.0233 (3)|0.0337/0.0271/0.0438 (3)|0.0547/0.0488/0.0258 (3)|0.0559/0.0493/0.0515 (3)|
|24|0.0274/0.0236/0.0157 (3)|0.0278/0.0252/0.0313 (3)|0.0626/0.0837/0.0179 (3)|0.0640/0.0817/0.0363 (3)|
|25|0.0315/0.0421/0.0125 (2)|0.0318/0.0395/0.0266 (2)|0.0449/0.0437/0.0155 (2)|0.0468/0.0451/0.0305 (2)|
|26|0.0224/0.0216/0.0107 (3)|0.0228/0.0222/0.0250 (3)|0.0438/0.0431/0.0118 (3)|0.0500/0.0483/0.0281 (3)|
|28|0.0214/0.0212/0.0096 (3)|0.0235/0.0231/0.0226 (3)|0.0430/0.0428/0.0102 (3)|0.0465/0.0463/0.0252 (3)|

The observed baseline sustained crossover exponents are **23 / no win through 26 / 23 / 23** in that order.

### Selected candidate: available cold map

|log₂ rows|90 1/16|90 1/2|112 1/16|112 1/2|
|---:|---:|---:|---:|---:|
|15|—|—|—|—|
|16|—|—|—|—|
|17|—|—|—|—|
|18|—|—|—|—|
|19|0.1859/0.1191/0.2535 (1)|0.2011/0.1387/0.2700 (1)|0.2012/0.1404/0.2700 (1)|0.2016/0.1329/0.3008 (1)|
|20|—|—|—|—|
|21|—|—|—|—|
|22|—|—|—|—|
|23|—|—|—|—|
|24|—|—|—|—|
|25|—|—|—|—|
|26|0.0223/0.0215/0.0106 (1)|0.0225/0.0221/0.0209 (1)|—|—|
|28|—|—|—|—|

The selected sparse lane path changes at >2,048 fixed owners with mean incoming ≤16; otherwise the grouped/tiled limit remains 16,384 owners. Only the forced-schedule comparison, not a complete three-sample selected map, supports the lowered threshold. Final crossovers and monotonicity are **not established**.

### Xalan publication and whole admission

|binary|samples|merge ms E/P/C|packed merges only ms|whole admission ms E/P/C|C published bytes|
|---|---:|---:|---:|---:|---:|
|base|3|17.732/16.422/31.252|19.061|182.143/132.043/151.018|115,726,164|
|tiled|3|17.661/16.213/29.742|17.885|182.316/131.290/148.511|115,726,164|
|grouped|3|18.089/16.491/28.448|16.325|184.525/132.385/148.436|115,726,164|
|arena|3|18.153/16.482/27.757|15.649|183.996/132.254/153.421|115,726,164|

At the last complete three-sample checkpoint C still misses both publication targets: **27.757 vs 24 ms** total; **15.649 vs 10 ms** packed only. Whole C admission did not improve over baseline.

|Published rows by admission|merge operations|baseline ms E/P/C|workspace ms E/P/C|
|---|---:|---:|---:|
|1–256|420|5.239/4.567/4.562|5.349/4.587/4.611|
|257–4,096|377|4.655/4.165/4.101|4.756/4.136/4.147|
|4,097–65,536|279|3.767/3.454/5.230|3.854/3.478/5.251|
|65,537–1,048,576|190|2.997/3.069/12.030|3.113/3.122/9.892|
|1,048,577–1,000,000,000,000|48|1.056/1.161/5.116|1.079/1.139/3.860|

Baseline matched three-trace medians for the **243 packed merges**: E/P/C **10.112/6.656/56.801 µs**. One selected trace: **10.144/6.688/52.513 µs**, **0/243 wins**. The 64-register grouped compact writer in the 101,500-row scope still issued **82,688 local load requests and 60,072 stores**, ran at **13.2% active warps**, **24.3 lanes/instruction**, and had **34.0% long-scoreboard and 20.0% barrier** PC samples. The measured evidence does not support calling this workload near parity with E.

### Acceptance status

Selected: **31 packed-run and 14 code-space tests passed**, formal checker exited successfully, all physical sizes matched, and three clean matched-history real reads took **152.582/145.747/183.163 ms E/P/C**, vs **152.534/145.692/183.316 ms** baseline. The sorted/cold and scaled-reader invocations that completed passed their oracles. Missing all-cell three-sample endpoint, scaled-reader, and cold-map coverage prevents marking the production code accepted or committing it. Reader implementations and engine files were not changed.

## Final selected publication checkpoint

Three additional clean, unprofiled selected replays: **E/P/C publication 17.740/16.263/27.310 ms; packed merges 15.317 ms; whole admission 182.615/131.764/146.435 ms**. Final bytes **115,726,164**. Total publication misses the 24-ms target by **3.310 ms**, and packed publication misses 10 ms by **5.317 ms**. One clean trace: 243 bounds (**1.103 ms**), 243 team planners (**1.582 ms**), 57 separate scans (**.115 ms**), 243 compact writers (**4.317 ms**), 5 splices (**.037 ms**), 68 bootstrap analyzes (**.223 ms**). Two planning kernels have a real median of 3.97 and 6.21 µs each: much smaller than the old outliers, not an aggregate ≤5-µs floor.

|Individual packed rows|merges|baseline span µs E/P/C|selected span µs E/P/C|C beats E|
|---|---:|---:|---:|---:|
|32,769–65,536|75|9.984/5.152/52.353|10.016/5.152/51.808|0|
|65,537–262,144|116|10.000/6.848/55.233|10.080/6.896/49.297|0|
|262,145–1,048,576|40|11.088/11.040/82.689|11.088/11.136/64.161|0|
|1,048,577–1,000,000,000,000|12|18.512/21.248/169.858|18.336/20.128/104.737|0|
|32,769–1,000,000,000,000|243|10.112/6.656/56.801|10.113/6.656/52.320|0|

Three-trace medians of three matched samples per merge. Selected publication by the five admission size classes above, ms E/P/C: 5.261/4.526/4.542; 4.670/4.102/4.117; 3.776/3.434/5.118; 2.991/3.048/9.737; 1.068/1.150/3.776. The production candidate remains uncommitted because the remaining cold and scaled-reader gates were not completed.

## Completed acceptance gates for the selected candidate

Run after handoff with the same resumable runner (`run.py selected endpoints|scaled|sweep 3`) on the `selected` executable (built 05:19:37 after the last source edit at 05:19:05; SHA-256 `ed80de42540ce0320ba2e5a4f497484ca1817e9e94dc627609c7b0af3bfa2648`), local GPU, every process with empty compute-process lists before and after. All cells now have three clean samples for the candidate; baseline cells have three, except exponents 16–18 and 25, which have two.

Cold merge, packed (C) device-event ns per output row, baseline → selected, and selected C/E:

|log₂ rows|90, 1/16|90, 1/2|112, 1/16|112, 1/2|
|---:|---|---|---|---|
|15|3.2812 → 3.7187 (1.28)|3.4688 → 3.3066 (1.05)|3.5596 → 3.8125 (1.31)|3.6611 → 3.3760 (1.23)|
|17|0.8794 → 0.9609 (1.38)|0.8789 → 0.8828 (1.22)|0.9014 → 0.9297 (1.28)|0.9337 → 0.9062 (1.21)|
|19|0.2710 → 0.2561 (1.36)|0.2578 → 0.2700 (1.34)|0.2908 → 0.2656 (1.32)|0.2867 → 0.2930 (1.47)|
|20|0.1865 → 0.1152 (1.12)|0.1608 → 0.1553 (1.43)|0.2090 → 0.1356 (1.02)|0.1787 → 0.1592 (1.25)|
|21|0.1429 → 0.0638 (0.97)|0.1196 → 0.0913 (1.46)|0.1558 → 0.0698 (0.88)|0.1294 → 0.0996 (1.19)|
|22|0.1174 → 0.0359 (0.85)|0.0959 → 0.0608 (1.44)|0.1312 → 0.0410 (0.63)|0.1071 → 0.0649 (1.00)|
|23|0.0233 → 0.0233 (0.73)|0.0438 → 0.0375 (1.13)|0.0258 → 0.0255 (0.46)|0.0515 → 0.0450 (0.82)|
|24|0.0157 → 0.0152 (0.56)|0.0313 → 0.0267 (0.95)|0.0179 → 0.0170 (0.27)|0.0363 → 0.0302 (0.48)|
|26|0.0107 → 0.0106 (0.47)|0.0250 → 0.0208 (0.93)|0.0118 → 0.0115 (0.26)|0.0281 → 0.0229 (0.46)|
|28|0.0096 → 0.0094 (0.44)|0.0226 → 0.0181 (0.78)|0.0102 → 0.0100 (0.23)|0.0252 → 0.0201 (0.43)|

- Sustained crossover (first size at which C beats E and every larger measured size does too), baseline → selected: 90 at 1/16: 2^23 → 2^21; 112 at 1/16: 2^23 → 2^21; 112 at 1/2: 2^23 → 2^23 (parity at 2^22); 90 at 1/2: none through 2^26 → 2^24. C now beats E at all eight 2^26 and 2^28 endpoints.
- The 2.7x drop in absolute time between 2^22 and 2^23 is gone (90 at 1/16: 150.5 µs at 2^22, 195.5 µs at 2^23). A smaller step remains where the sparse path engages: 134.3 µs at 2^19 against 120.8 µs at 2^20 for 90 at 1/16.
- Regression outside the gates: the 1/16 cases at 2^15–2^17 rows are 3–13% slower than baseline (for example 3.28 → 3.72 ns/row at 2^15). The xalan in-sequence history, which is what those sizes serve, is faster overall (publication 31.25 → 27.31 ms).
- Endpoint gate (no cold endpoint more than 2% slower): passes; all eight are faster, by 0.8–2.7% at 1/16 and 16.5–19.9% at 1/2.
- Scaled reader gate (20 cells, fixed and leaf layouts, states 90 and 112, +2% limit): passes; worst cell +0.15%.
- Matched-history real reads E/P/C: 152.582 / 145.747 / 183.163 ms against 152.534 / 145.692 / 183.316 ms baseline.

With these gates complete the candidate is committed. The xalan targets remain missed as stated above (27.31 ms against 24 ms total; 15.32 ms against 10 ms for packed merges; C wins 0 of 243 in-sequence merges).
