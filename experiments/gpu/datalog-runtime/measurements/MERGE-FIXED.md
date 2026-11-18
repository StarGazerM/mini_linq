# Fixed costs in packed merge publication

Raw evidence: `/data/design-notes/doop-memory/merge-fixed/`. All numbers below are from this investigation. E = engine, P = plain storage, C = packed with default 32,768-row plain cutoff. Times include publication unless explicitly identified as kernel times. All GPU executions use the exclusive GPU lock; per-process `.pre`/`.post` files record compute tenants.

## Baseline: 0b2bb9ff

Five fresh processes: median xalan E/P/C **18.143 / 23.519 / 38.031 ms**. Of C, 243 packed publications took **20.557 ms (84.60 µs/merge)**. This baseline itself differs slightly from the 37.97 ms acceptance ceiling; the ceiling is not relaxed. The real admission trace tops out at 2,301,108 rows; there are no observed, in-sequence 2²²–2²⁶-row xalan merges. Synthetic cold results must not be called measured real-history results.

Default binary SHA-256 `5406f36efebb6733c17b965b76e6ddea1b3b2d2749ea744fb14009bc5f069a48`. Nsight Systems `--trace=cuda,nvtx --sample=none --cpuctxsw=none`, separate NVTX build. Joined launch correlation IDs to the host CUDA API inside `new_packed/admission/packed_merge` and `engine/admission/engine_merge_publication`. For E the publication range can cover a chain; these are the two launches for the *same individual merge*, selected by position in the admission. For C these four examples have an already-packed owner. The C scope returns after submission; it excludes the profiling caller's final stream drain. API and device intervals overlap; don't sum them to estimate elapsed time.

|Output rows (admission seq; position, zero-based)|E partition / merge API → device µs|E gap µs|C plan / offsets / writer API → device µs|C gaps µs (end of launch API to next start)|C 24-byte D2H API / device µs|C field H2D bytes / API / device µs|
|---:|---|---:|---|---|---|---|
|101,500 (869; 1)|2.59→3.97 / 6.12→3.52|3.12|3.32→39.49 / 2.74→1.50 / 2.85→32.03|2.67 / 54.51|42.12 / 0.96|36 / 2.52 / 0.38|
|520,202 (1120; 1)|2.63→4.03 / 2.68→5.15|3.15|8.88→43.01 / 2.84→3.62 / 2.82→40.19|2.75 / 55.33|41.61 / 0.99|24 / 2.62 / 0.35|
|1,085,127 (1034; 2)|2.45→7.62 / 2.50→12.19|3.07|2.78→52.06 / 2.45→5.86 / 3.17→80.99|2.49 / 76.40|58.31 / 0.99|36 / 2.43 / 0.38|
|1,780,906 (999; 1)|2.67→4.77 / 2.52→10.18|3.20|3.36→49.31 / 2.56→9.22 / 2.76→95.87|2.61 / 81.00|67.04 / 0.99|24 / 3.08 / 0.38|

All four C ranges have three launches (not four), one stream sync immediately following the pageable D2H (0.49–0.56 µs after the D2H API has already waited), ten `cuEventRecord` calls (10.34–11.53 µs summed), and 14.95–20.11 µs host time outside traced CUDA APIs. Five event records are explicit merge diagnostics; pool/staging operations also record events. Five real output data allocations (payload, headers, directory, frames, fences), one field descriptor allocation and upload, and four nonempty temporaries (plan, frames, tile totals, three scalar totals); exception scratch is empty on this path. No driver allocation calls appear in those ranges: these are pool suballocations, not `cuMemAlloc` latency. No separate fence or directory upload: writer emits one fence/header per output leaf and one 32-bit directory slot per 256 output rows. E has output plus partition storage per merge, one workspace memset, three event records, two launches, no transfers. Launch submission, event bookkeeping and transfer latency are fixed; planner/writer/scan and waiting for them are not. The observed device work is substantial on the million-row merges.

## Plain-storage merge

[INFERENCE, before change] Reuse the engine's `SetOps::merge` for bandwidth-bound plain output instead of eight-row serial tiles. Keep the low-work single-launch path for tiny runs. Retain the sorted immutable batch directly rather than constructing another shared prefix for every transient merge. Do not record artificial partition and layout phases for plain merges. Expect a clear cold P improvement, reduced small-admission publication time and no change in packed format or reader work.

Five fresh default processes: E/P/C **18.123 / 19.288 / 34.348 ms**, of which C packed-only is **20.724 ms**. P now wins the two smallest size classes but misses E on the three larger ones. The admission timer synchronizes *each individual P/C merge* whereas E synchronizes only the published admission, even when it contains a chain. This is not reflected in device bandwidth. No reader or packed device kernel was intentionally changed.

|Cold rows|State|Delta/owner|Baseline E/P/C ns per output row|Plain-path change E/P/C|
|---:|---:|---:|---|---|
|2²⁶|90|1/16|.02245 / .02666 / .01092|.02251 / .02151 / .01090|
|2²⁶|90|1/2|.02295 / .03365 / .02495|.02290 / .02202 / .02509|
|2²⁶|112|1/16|.04388 / .04758 / .01236|.04388 / .04291 / .01190|
|2²⁶|112|1/2|.04897 / .05620 / .02870|.04915 / .04791 / .02865|
|2²⁸|90|1/16|.02132 / .02952 / .00961|.02137 / .02124 / .00969|
|2²⁸|90|1/2|.02352 / .04592 / .02252|.02329 / .02316 / .02281|
|2²⁸|112|1/16|.04295 / .05766 / .01033|.04287 / .04282 / .01031|
|2²⁸|112|1/2|.04676 / .07821 / .02528|.04670 / .04619 / .02528|

Three clean processes per cold cell, sorted full output checked after eviction. P matches or beats E in each cold case; no C median regressed >2% against the newly measured baseline. Both synthetic storage oracles, 31 packed-run and 12 code-space tests, and the formal inventory passed.

### Full cold-cache NCU checkpoints on the real inputs

`ncu --set full --nvtx --nvtx-include ... --kernel-name regex:... --launch-skip ... --launch-count ...`. Eight independent replays of the same xalan trace. Values are device µs, in E partition / merge and C plan / scan / writer order. The default clock/cache controls and kernel replay perturb these times: they are not in-sequence nsys times.

|Rows|E device µs|C device µs|C plan/writer CTAs|C plan/writer registers per thread|C plan/writer active warps %|
|---:|---|---|---|---|---|
|101,500|12.67 / 10.53|115.04 / 4.42 / 90.02|175 / 279|94 / 128|10.14 / 12.88|
|520,202|13.12 / 15.81|123.62 / 10.59 / 111.71|1,072 / 1,412|72 / 128|43.83 / 26.98|
|1,085,127|15.94 / 33.82|139.65 / 16.93 / 219.46|2,223 / 2,939|94 / 128|36.64 / 28.54|
|1,780,906|15.90 / 30.98|130.34 / 26.40 / 263.97|3,801 / 4,846|72 / 128|52.20 / 30.98|

Across all 243 packed publications, nsys counted **68** with a plain owner: each executes an additional `analyze/offsets/pack` bootstrap. The other 175 have just the three merge kernels. Total **933** kernels including bootstrap, **729** excluding it. The exact layout diagnostic (separate clean default process) gave p50 scan / post-scan / inclusive blocked **4.095 / 11.648 / 22.413 µs**, sums **1.025 / 2.848 / 6.440 ms**. This counts the merge readback, not the 68 bootstrap readbacks.

### Exact sizing versus a bound

Offline the schedule and exact per-field owner frames were replayed from actual xalan keys, including rounding, widening, uneven splits, zero-width frames and the bootstrap selection. **All 243** modeled `(rows, output blocks, payload words)` tuples matched the freshly captured device sizing log in merge order. The hypothetical estimator is deliberately advantaged: it knows, for every owner block, the exact number of incoming rows that land in it. The starting estimate is the owner's real bytes plus `8*ceil(incoming*owner_bits/64)` per block. Those counts alone require a device partition; they are not currently known by the allocating host.

|Payload estimate|Median extra bytes / %|p95 extra bytes / %|Worst extra bytes / %|Aggregate extra %|
|---|---:|---:|---:|---:|
|No slack (not a bound)|−3,192 / −0.65|2,174 / 0.09|9,904 / 0.28|−0.53|
|Plus worst observed byte shortfall|105,472 / 23.17|110,838 / 71.24|118,568 / 94.38|10.36|
|Times worst observed widening ratio|103,312 / 26.25|1,257,806 / 27.21|2,853,136 / 27.44|26.41|
|Hard safe bound: touched blocks at full word width; split padding exact|588,976 / 121.92|4,787,810 / 292.43|11,280,272 / 493.13|127.52|

Columns are marginal quantiles, not necessarily the same merge. The base estimate underallocated **207/243** times, by up to **108,664 bytes**, or **21.31% of exact size**. The empirical slacks only cover this history, not future inputs. A hard-safe fallback must either reserve materially more memory or detect overflow before the writer and fall back, which again requires synchronization. **Keep exact sizing.** Even correctly reserving payload bytes would not give the host the exact published block count needed by the unchanged reader. See `bound.py`, `bounds.csv` in the raw evidence directory.

## Cooperative exact sizing

[INFERENCE, before change] On the cooperative schedule each owner CTA already knows its final geometry. Use the last CTA to scan all published tile sizes; a stream-resident retirement counter avoids replacing the saved launch with a per-merge memset. This should remove one launch from each of 243 packed merges, preserve every offset and published byte count, and leave the large-grid sparse schedule alone. The actual D2H dependency remains. Expect a modest small-merge gain, not the full 2.85 ms sizing round trip.

Five-process E/P/C **17.868 / 19.122 / 34.483 ms**, packed-only **20.875 ms**. Two merge launches rather than three; total steady publication did **not** improve over the preceding step. In the same in-sequence ranges at 101,500 / 520,202 / 1,085,127 / 1,780,906 rows, fused plan+scan device time was 41.22 / 50.91 / 65.98 / 79.17 µs; at baseline separate plan+scan had summed to 40.99 / 46.63 / 57.92 / 58.53 µs. The retirement atomic and lower parallelism of the final scan trade device time for a saved host submission. For the first 142 outputs (32–128K), the in-sequence device span summed 0.155 ms lower; above 128K the savings did not generalize.

## Reuse immutable schema storage

[INFERENCE, before change] `fields` never changes during a merge or geometry diagnostic. Retain the sealed owner's immutable field buffer instead of allocating, collecting a CPU vector, staging a 24–48-byte H2D, and retiring its staging slot on each packed publication. Plain runs need no field allocation. Expect to eliminate 243 uploads and associated allocator/event work without changing any reader pointer layout, offsets, payload or logical size.

Five-process E/P/C **17.979 / 19.179 / 33.706 ms**, packed-only **20.059 ms**. Median packed publication fell by **3.36 µs/merge** versus the fused change. One clean process measured **41.15 ms** for C; the other four were 33.56–33.87 ms. The five-process median passes the 37.97 ms ceiling, but this is not an upper bound on individual runs. In the subsequent nsys capture, non-bootstrap packed merges have no H2D copy; bootstrapping a previously plain owner still has to establish a schema.

## Amortize RCCL setup

[INFERENCE, before change] In the in-sequence control the original one-launch plain merge still takes only 4.8–7.6 µs at 32K–128K versus roughly 9–11 µs for E's two launches. Use the single launch through 128K rows. Above it, reuse the loaded RCCL program and a partition buffer for the stream rather than clearing, allocating and retiring fresh workspace on every merge. No tiling or ordering changes to RCCL. Expect improved P in the three larger xalan classes with cold bandwidth unchanged.

Measured **18.162 / 17.030 / 32.331 ms**, packed-only **20.009 ms**. P beats E overall but not yet in the two largest classes (.1242 vs .1162 and .0459 vs .0386 ns/row). The first cache prototype failed the xalan oracle with `LengthMismatch { role: "merge partitions", expected: 129, actual: 296 }`. The fix passes a writable prefix with the exact logical partition count; it does not clear or allocate storage that the partition kernel initializes. The cache guard covers both kernel submissions; cached scratch is only used on the module's owning pool stream. Other allocator-sharing lanes use the engine's ordinary local workspace.

## Baseline crossover survey

Cold `--merge-delta` as implemented in `merge_scale`: the delta is relative to the owner, not relative to output. Each cell is **E/P/C GPU-event ns per output row**, median of three clean fresh processes; same-process configurations, 256 MiB eviction between samples, complete sorted-output oracle. Small cold samples include RCCL setup on the first merge of a new set and should not be mistaken for steady-state launch floors. Scaled C explicitly seals both inputs even at 2¹⁵; the ordinary C admission policy at/below 2¹⁵ is P instead. The original mode already accepts every integer row count; no powers-of-two parser change is necessary.

|log₂ rows|90 1/16|90 1/2|112 1/16|112 1/2|
|---:|---|---|---|---|
|15|2.9375/1.3125/3.3437|3.0664/1.2500/3.3437|2.9678/1.4062/3.4375|3.0947/1.4092/3.5938|
|16|1.4531/.6098/1.7002|1.4844/.6538/1.7319|1.4844/.7651/1.7495|1.4731/.7656/1.8281|
|17|.7566/.3281/.8828|.7656/.3435/.9053|.7671/.3984/.8889|.7656/.4219/.9453|
|18|.3994/.2070/.4609|.4060/.2194/.4844|.3982/.2422/.5004|.4062/.2344/.4648|
|19|.1938/.1094/.2832|.2212/.1172/.2661|.2207/.1525/.3066|.2206/.1426/.2812|
|20|.1135/.0739/.1777|.1171/.0811/.1669|.1416/.1211/.2100|.1338/.1221/.1826|
|21|.0674/.0543/.1349|.0679/.0636/.1175|.0859/.0840/.1572|.0878/.0892/.1387|
|22|.0437/.0420/.1096|.0449/.0469/.0894|.0671/.0669/.1333|.0691/.0696/.1080|
|23|.0341/.0331/.0240|.0342/.0380/.0443|.0549/.0570/.0269|.0560/.0586/.0518|
|24|.0272/.0290/.0162|.0282/.0339/.0320|.0644/.0892/.0181|.0645/.0934/.0372|
|25|.0290/.0437/.0127|.0304/.0496/.0266|.0452/.0481/.0153|.0464/.0518/.0313|
|26|.0224/.0267/.0109|.0229/.0336/.0249|.0439/.0476/.0124|.0490/.0562/.0287|

Baseline: first win sustained through 2²⁶ is **2²³** for 90 1/16 and both 112 cases. For 90 1/2, C first wins at 2²⁵, loses at 2²⁶, wins again at 2²⁸: **no monotone measured crossover** in the sweep. The abrupt change at 2²³ coincides with switching owner-grid scheduling.

### The actual history is not a full factorial

Joined all 1,314 simulated ratio-schedule steps (from recorded fresh sizes, no synthetic deltas) to the CUDA kernel spans in each NVTX configuration. E is exactly the corresponding partition/merge pair for 64-/128-bit words, or the one wide kernel for >128; C includes bootstrap when present. `base-nvtx-results/sequence.csv` contains each individual E/P/C first-kernel-start to last-kernel-end interval. At baseline C won **0/243** packed in-sequence merges. On exactly those merges E/P/C median spans were **10.02 / 6.50 / 75.75 µs**, means **11.43 / 9.91 / 85.68 µs**. In states 90/112 the largest observed outputs are 2,301,108 / 1,399,332; no packed merge anywhere has an incoming/owner ratio as small as 1/16 (minimum **0.09467**). These missing cells have no real-history crossover, not a fabricated scaled sample.

## Avoid materializing the bootstrap owner

[INFERENCE, before change] On 68 xalan packed publications, owner sealing currently produces a disposable packed payload. Its frames, headers and fences *are* needed for the identical layout decision, but its word offsets and encoded bytes are not: the original plain input is still resident and independently addressable. Analyze it once, keep the original plain pointer through planning/writing, and translate its rows directly into destination frames. Expect to remove two launches, a temporary payload and one additional exact-size readback on these 68 merges; keep the sparse cold path unchanged.

Measured **18.101 / 17.184 / 31.285 ms**, packed-only **18.866 ms (77.64 µs/merge)**. The 68 bootstrap in-sequence spans fell from **6.841 to 4.722 ms** in total; their kernel work from 2.935 to 2.183 ms. Those merges now run `analyze/plan_cooperative/write_balanced` rather than `analyze/offsets/pack/plan_cooperative/write_balanced`. The 243 sizing log tuples still exactly match baseline payload bytes and output leaf counts. A two-word regression test compares every header and frame field, every fence, every directory entry and the actual payload words against explicitly sealing the same owner before merging.

## Limit scan fusion to the measured fixed-cost range

[INFERENCE, before change] Combining a many-CTA planner with a one-CTA scan costs more than the saved launch once the retirement counter is heavily contended. In the 1.78M example the fused scan cost 21 µs more device time; in the 100K example it was break-even. Restrict fusion to outputs of at most 128K rows (142 xalan merges). Above that, use the parallel planner without a retirement atomic and dispatch the 256-thread scan. Expect unchanged published bits, better medium/cold timing and less host wait on large in-sequence merges.

## Drain plain chains at the publication boundary

[INFERENCE, before change] E publishes 1,314 merges in 870 admissions (348 multi-merge chains), draining the stream **once per admission**. P/C profiling alone introduces an extra host barrier after every transient merge; unprofiled P/C already use stream order. Remove barriers *between consecutive plain intermediates*, retaining a separate event interval per kernel pair and one final drain before a measured publication completes. Keep the split at a plain→packed transition and every packed-merge drain so packed-only attribution is comparable. Include final host run-list publication in the interval. Expect P to approach E in the largest admission classes without crediting asynchronous unfinished work. Report the preceding synchronization protocol separately; this change improves instrumented admission, not RCCL kernel bandwidth.

## Reproduction

The raw directory contains the exact executables, per-process JSON and tenant checks, captures, SQL correlation and schedule joins (`trace.py`, `history.py`), NCU launch selection (`counters.py`) and independently checked sizing study (`bound.py`). No capture is needed in git. Use the default executable for gates and a separate executable with `--features cq-gpu-runtime/nvtx` for the scoped profilers; do not time the NVTX executable. The captures cover the whole history, then select work *by host range and correlation ID*, not by a nearest timestamp on the GPU.

```sh
# From experiments/gpu in this worktree:
source /dev/shm/gpu-env.sh
R=/data/design-notes/doop-memory/merge-fixed
A=/data/design-notes/doop-memory/real-replay/admission-v2
J=/data/design-notes/doop-memory/real-replay/joint-v2
flock -s /tmp/mini_linq-timing.lock timeout 900 cargo +1.99.1 oxide build \
  --arch sm_89 --materialize-cubin -- --release -p gpu-datalog-runtime --bin packed-replay
# Copy the default binary to $R/<new-label>. The raw runner takes the GPU lock
# and records nvidia-smi compute-process listings immediately before and after.
"$R/runs.sh" <new-label> admission 5
"$R/runs.sh" <new-label> sweep 3      # powers 15 through 26, both states and deltas
"$R/runs.sh" <new-label> cold 3       # powers 26 and 28
"$R/runs.sh" <new-label> sizing
"$R/runs.sh" <new-label> real 3       # PACKED_READ_LEAVES=1
"$R/runs.sh" <new-label> scaled 3     # fixed + merged-leaf reads, both states at 2^28

# After building and saving <nvtx-label> separately with the NVTX feature:
"$R/runs.sh" <nvtx-label> capture
python3 "$R/trace.py" "$R/<nvtx-label>-results"
python3 "$R/history.py" "$R/<nvtx-label>-results"
python3 "$R/counters.py" <nvtx-label>
python3 "$R/bound.py"
```

## In-sequence map, before and after

Actual state-specific xalan observations in the **nearest half-delta bin** (`round(log₂(owner/incoming)) = 1`, so not necessarily exactly 1/2); row bins likewise use nearest power of two. Per-individual-merge median kernel span in ns/output row, including device-idle submission gaps inside that span. No unobserved bin is interpolated. The literal history has no 1/16 bin for these states and none of the requested large-size cells. `sequence.csv` also retains non-half deltas rather than mixing them into these cells.

|State|log₂ rows|Merges|Baseline E/P/C|Final E/P/C|
|---:|---:|---:|---|---|
|90|15|2|.2889 / .1408 / 2.6232|.2848 / .1431 / 1.6048|
|90|16|8|.1395 / .0715 / .7797|.1441 / .0721 / .6577|
|90|17|7|.0766 / .0458 / .4510|.0769 / .0477 / .4298|
|90|18|8|.0408 / .0338 / .2797|.0424 / .0424 / .2738|
|90|20|1|.0139 / .0268 / .1455|.0144 / .0133 / .1398|
|112|15|7|.2615 / .1566 / 2.5146|.2581 / .1589 / 1.3822|
|112|16|5|.1573 / .1079 / 1.6093|.1594 / .1095 / .9040|
|112|17|4|.0781 / .0571 / .4964|.0775 / .0674 / .4631|
|112|18|2|.0535 / .0465 / .6084|.0576 / .0597 / .5109|
|112|19|2|.0239 / .0416 / .2044|.0242 / .0249 / .1948|

Across all 243 packed outputs (all states and ratios), final median E/P/C spans were **10.18 / 6.69 / 57.54 µs** and means **11.48 / 9.05 / 71.66 µs**. C still wins **zero**. The admission ratio law doesn't generate the cold benchmark's balanced, interleaved presealed inputs.

### Final scoped decomposition

Match individual launches within the same four admissions as the baseline. `API→device` and host gaps in µs; D2H is the 24-byte sizing transfer API (the transfer itself measures 0.99 µs). No H2D uploads in any of these four final merge scopes.

|Output rows|Plan API→device|Offsets API→device|Writer API→device|Host gaps µs|D2H API µs|
|---:|---|---|---|---|---:|
|101,500|3.19→40.58|fused|2.88→32.29|58.22|49.62|
|520,202|3.18→42.98|3.39→3.65|3.13→40.67|4.23 / 58.03|49.70|
|1,085,127|3.05→50.79|2.54→5.98|2.69→81.12|2.64 / 66.07|57.82|
|1,780,906|2.97→55.62|2.78→9.15|2.89→96.45|8.48 / 68.70|60.26|

Final launch inventory for the 243 packed merges: 243 planners, 243 writers, **101** separate scans, **68** bootstrap analyze launches: **655 total vs 933** at baseline. Unchanged: fences, frame copies, the 4-byte row directory and header writes are inside the writer; still one exact merge D2H each, no bootstrap D2H. Device-time sums in the final real nsys capture: analyze **0.223 ms**, planner (including fused sizing) **4.835 ms**, separate scan **0.324 ms**, writer **5.541 ms**. Even perfect elimination of submission and all readbacks would leave **10.923 ms** of device work in the packed merges alone at the observed in-sequence workload.

The final four ranges each record nine events (vs ten); excluding those API calls, launches, copies and the post-copy sync leaves 11.52–18.73 µs of host work. A serial addition of profiler API time to kernel durations would count the blocking part of D2H twice. All changes preserve the ordinary reader's fixed and variable ABIs; the regression test for direct bootstrap compares physical output, not just decoded keys.

Full NCU on the same final inputs (`final-traced-results/ncu-*-C.csv`, µs):

|Rows|Plan / separate scan / writer|Plan/writer regs per thread|Plan/writer active warps %|
|---:|---|---|---|
|101,500|118.69 / fused / 89.89|94 / 128|10.21 / 12.94|
|520,202|125.06 / 10.53 / 111.46|96 / 124|32.21 / 27.10|
|1,085,127|140.86 / 17.38 / 217.50|94 / 128|36.64 / 28.53|
|1,780,906|145.70 / 26.46 / 263.14|96 / 124|38.40 / 30.94|

With sizing markers, final vs baseline: separate scans **0.532 vs 1.025 ms summed** (4.19 µs median among the remaining 101; zero means fused, not free), post-scan round trips **2.820 vs 2.848 ms** (p50 11.17 vs 11.65 µs), inclusive host waits **6.756 vs 6.440 ms** (p50 22.94 vs 22.41 µs). All 243 final `(rows, words, blocks)` records match the baseline in order. The saved bootstrap readbacks are **outside** that merge-sizing marker.

## Policy (report only)

The largest **fresh** run in the recorded history is 1,224,859 rows; the largest **output** is 2,301,108. Any plain cutoff above both counts runs *exactly* the existing plain-control path: no bootstrap, no retained compressed runs. Thus P measures the counterfactual directly, without scaling a fitted cold curve or changing the default. This also holds for a state-dependent choice of cold crossover provided each applicable cutoff exceeds its largest output. Standalone compressed reads would no longer have the same footprint.

|Policy over xalan|Final published bytes|Final live pool bytes|Peak live pool bytes|
|---|---:|---:|---:|
|C, default 32,768|115,726,164|115,803,136|123,738,368|
|Plain through any measured cold crossover above 2,301,108|271,192,208|271,244,544|293,530,112|

Larger-cutoff publication is faster but costs **155,466,044 more final bytes** and **169,791,744 more peak bytes**. It is not a win on both axes. The 32,768 default is unchanged.

## Xalan publication: final default build

Five fresh clean processes; admission-class sums divided by merged rows. These are complete wall-time samples, not the kernel spans above.

|Merged rows per admission|Baseline E / P / C ns/row|Final E / P / C ns/row|
|---|---|---|
|1–256|236.38 / 288.37 / 288.67|233.57 / **201.28** / 202.38|
|257–4,096|14.669 / 18.727 / 18.851|14.639 / 12.782 / **12.740**|
|4,097–65,536|.9805 / 1.2719 / 1.8840|.9848 / **.8916** / 1.3630|
|65,537–1,048,576|.11762 / .15274 / .48330|**.11723** / .11754 / .45578|
|>1,048,576|.03942 / .06893 / .19115|**.03947** / .04186 / .18557|
|Total ms|18.143 / 23.519 / 38.031|18.043 / **16.424** / 31.192|

C's packed-only publication **20.557 → 19.145 ms**, **84.60 → 78.78 µs/merge**. Report-only raised cutoff would take **16.424 ms** (P) vs **31.192 ms** at the default, at the memory cost above. P beats E overall by **1.619 ms**, but misses E by **0.3% / 6.1%** in the two largest xalan classes. C meets the historical 37.97 ms median gate, not the **17.8 ms** target. Baseline→final total C improvement **6.839 ms**; of this **1.412 ms** is packed-only. The rest is in plain publication. Final plain-only C wall time is about 12 ms (difference of separate process medians); together with 10.923 ms actual packed device work it already exceeds the target, even before any packed submission, wait or allocation is charged. Further endpoint gains require changing or avoiding the expensive packed planner/writer work, not treating a 24-byte copy as the whole problem.

## Checkpoint ledger

Publication times are medians of five clean default processes (ms). Packing cutoff and reader code remained unchanged.

|Commit|Variant|E|P|C|C packed only|
|---|---|---:|---:|---:|---:|
|`0b2bb9ff`|baseline|18.143|23.519|38.031|20.557|
|`cae95461f3`|plain1|18.123|19.288|34.348|20.724|
|`a20f41e94f`|fused2|17.868|19.122|34.483|20.875|
|`a66625fc5b`|fields3|17.979|19.179|33.706|20.059|
|`1ccd036842`|prefix4|18.162|17.030|32.331|20.009|
|`2f30269464`|direct5|18.101|17.184|31.285|18.866|
|`c1c327c880`|limited6|18.127|17.182|31.118|18.623|
|`b92655bbc9`|chain7|17.909|16.305|31.079|19.069|
|`bc883c41d2`|final|18.043|16.424|31.192|19.145|

The final guard uses the cached retirement counter and RCCL workspace only on their owning pool lane. Related lanes can share memory without sharing a stream, so they use the separate scan and call-local workspace. No reader changes. All eight builds passed their packed test suites, synthetic plain/packed and real admission oracles, and the formal inventory; the final suites passed **31 + 13** tests. No claim about uncollected reader/cold cells is implied by these functional gates.

Why P still misses the two largest wall-time bins: summed individual device spans for E/P were **2.061 / 1.551 ms** (65K–1M) and **.667 / .616 ms** (>1M). P’s kernels were already faster in aggregate in both classes. Synchronization, host submission, per-merge event records and publication outside the kernel intervals make that device gain insufficient.

### Cold checkpoints (ns/output row)

Cells are E/P/C; sample counts are given in parentheses where fewer than three clean processes had completed at handoff. A dash is unmeasured, not zero. The raw replay summary verified sorted full outputs, not only row counts.

|Checkpoint, 2^26 output rows|90 1/16|90 1/2|112 1/16|112 1/2|
|---|---|---|---|---|
|baseline|0.02245 / 0.02666 / 0.01092|0.02295 / 0.03365 / 0.02495|0.04388 / 0.04758 / 0.01236|0.04897 / 0.05620 / 0.02870|
|plain1|0.02251 / 0.02151 / 0.01090|0.02290 / 0.02202 / 0.02509|0.04388 / 0.04291 / 0.01190|0.04915 / 0.04791 / 0.02865|
|fused2|0.02231 / 0.02155 / 0.01096|0.02284 / 0.02184 / 0.02496|0.04373 / 0.04281 / 0.01193|0.04887 / 0.04913 / 0.02836|
|fields3|0.02240 / 0.02141 / 0.01080|0.02283 / 0.02194 / 0.02516|0.04384 / 0.04288 / 0.01225|0.04890 / 0.04809 / 0.02858|
|prefix4|0.02238 / 0.02170 / 0.01076 (2)|0.02381 / 0.02215 / 0.02470 (2)|0.04384 / 0.04326 / 0.01195 (2)|0.04889 / 0.04823 / 0.02878 (2)|
|direct5|0.02240 / 0.02177 / 0.01072 (2)|0.02290 / 0.02232 / 0.02490 (2)|0.04381 / 0.04308 / 0.01161 (2)|0.05002 / 0.04917 / 0.02796 (2)|
|limited6|0.02245 / 0.02182 / 0.01083 (1)|0.02286 / 0.02221 / 0.02487 (1)|0.04376 / 0.04317 / 0.01186 (1)|0.04921 / 0.04790 / 0.02799 (1)|
|chain7|0.02237 / 0.02153 / 0.01068 (1)|0.02272 / 0.02197 / 0.02460 (1)|0.04370 / 0.04311 / 0.01192 (1)|0.05019 / 0.04800 / 0.02797 (1)|
|final|0.02237 / 0.02167 / 0.01082 (1)|0.02298 / 0.02229 / 0.02520 (1)|0.04369 / 0.04309 / 0.01187 (1)|0.04890 / 0.04912 / 0.02803 (1)|

|Checkpoint, 2^28 output rows|90 1/16|90 1/2|112 1/16|112 1/2|
|---|---|---|---|---|
|baseline|0.02132 / 0.02952 / 0.00961|0.02352 / 0.04592 / 0.02252|0.04295 / 0.05766 / 0.01033|0.04676 / 0.07821 / 0.02528|
|plain1|0.02137 / 0.02124 / 0.00969|0.02329 / 0.02316 / 0.02281|0.04287 / 0.04282 / 0.01031|0.04670 / 0.04619 / 0.02528|
|fused2|0.02139 / 0.02116 / 0.00961|0.02342 / 0.02299 / 0.02275|0.04294 / 0.04271 / 0.01035|0.04662 / 0.04625 / 0.02529|
|fields3|0.02136 / 0.02116 / 0.00955|0.02321 / 0.02318 / 0.02246|0.04306 / 0.04284 / 0.01026|0.04652 / 0.04624 / 0.02522|
|prefix4|0.02138 / 0.02122 / 0.00950 (2)|0.02341 / 0.02314 / 0.02263 (2)|0.04301 / 0.04296 / 0.01022 (2)|0.04664 / 0.04642 / 0.02518 (2)|
|direct5|0.02138 / 0.02129 / 0.00953 (1)|0.02332 / 0.02326 / 0.02217 (1)|0.04311 / 0.04284 / 0.01030 (1)|0.04655 / 0.04657 / 0.02515 (1)|
|limited6|0.02138 / 0.02126 / 0.00953 (1)|0.02351 / 0.02331 / 0.02273 (1)|0.04300 / 0.04286 / 0.01020 (1)|—|
|chain7|0.02135 / 0.02122 / 0.00956 (1)|—|—|—|
|final|0.02135 / 0.02115 / 0.00949 (1)|0.02359 / 0.02328 / 0.02263 (1)|0.04293 / 0.04266 / 0.01026 (1)|0.04641 / 0.04621 / 0.02518 (1)|

### Matched-history reader gate

Scaled gate: worst change among five primitive measurements × two states × two layouts at 2^28 rows, relative to matched baseline medians. Real gate: total C device time across recorded real read sites with leaf-history replay. A partial layout set has no all-scaled verdict. N is the number of clean processes; scaled N is the minimum across the four configurations.

|Checkpoint|Real N|Real C ms|Change|Scaled N|Worst scaled change|
|---|---:|---:|---:|---:|---:|
|baseline|3|183.477|+0.00%|3|+0.00%|
|plain1|3|183.176|-0.16%|3|+1.03%|
|fused2|3|182.981|-0.27%|1|+1.76%|
|fields3|2|183.102|-0.20%|0|—|
|prefix4|0|—|—|0|—|
|direct5|0|—|—|0|—|
|limited6|0|—|—|0|—|
|chain7|0|—|—|0|—|
|final|1|183.901|+0.23%|1|+1.17%|

## Final cold crossover map

Single clean process per cell (the baseline table uses three). Ordered E/P/C device-event ns per output row.

|log₂ output rows|90 1/16|90 1/2|112 1/16|112 1/2|
|---:|---|---|---|---|
|15|2.9736 / 1.1221 / 3.3125|3.0000 / 1.1250 / 3.6182|3.0381 / 1.2812 / 3.4062|2.8438 / 1.1914 / 3.6875|
|16|1.4253 / 0.5937 / 1.7969|1.4375 / 0.5937 / 1.7812|1.4380 / 0.7031 / 1.8125|1.4531 / 0.7344 / 1.7676|
|17|0.7561 / 0.3203 / 0.8984|0.7493 / 0.3438 / 0.8684|0.7651 / 0.3857 / 0.9062|0.7424 / 0.4062 / 0.9138|
|18|0.3994 / 0.2235 / 0.4833|0.3757 / 0.2421 / 0.4375|0.3906 / 0.2255 / 0.4766|0.3900 / 0.2158 / 0.4382|
|19|0.2133 / 0.1211 / 0.2761|0.1914 / 0.1250 / 0.2500|0.2109 / 0.1638 / 0.3049|0.2051 / 0.1424 / 0.2891|
|20|0.1035 / 0.0676 / 0.1855|0.1085 / 0.0723 / 0.1604|0.1389 / 0.1017 / 0.2012|0.1456 / 0.0937 / 0.1946|
|21|0.0659 / 0.0517 / 0.1398|0.0635 / 0.0517 / 0.1221|0.0853 / 0.0664 / 0.1558|0.0860 / 0.0664 / 0.1309|
|22|0.0430 / 0.0349 / 0.1189|0.0437 / 0.0352 / 0.0945|0.0653 / 0.0535 / 0.1333|0.0671 / 0.0540 / 0.1065|
|23|0.0309 / 0.0275 / 0.0222|0.0332 / 0.0279 / 0.0446|0.0550 / 0.0491 / 0.0269|0.0548 / 0.0498 / 0.0519|
|24|0.0256 / 0.0237 / 0.0158|0.0283 / 0.0255 / 0.0317|0.0590 / 0.0763 / 0.0175|0.0576 / 0.0750 / 0.0363|
|25|0.0314 / 0.0380 / 0.0126|0.0321 / 0.0392 / 0.0266|0.0450 / 0.0437 / 0.0140|0.0459 / 0.0448 / 0.0298|
|26|0.0224 / 0.0217 / 0.0108|0.0230 / 0.0223 / 0.0252|0.0437 / 0.0431 / 0.0119|0.0489 / 0.0491 / 0.0280|

Before and after, C wins from **2^23** through **2^26** for state 90 at 1/16 and state 112 at either ratio. For state 90 at 1/2 it wins at **2^25**, loses at **2^26**, and wins at the independent **2^28** endpoint. There is no monotone measured threshold for that case. Final C cold change at the eight 2^26/2^28 endpoints is at worst **+1.02%** against the local baseline. P is slightly slower than E at 2^26, state 112 half-delta (**.04912 vs .04890**, +0.45%), and P also loses to E in individual intermediate-size cold cells; this is not universal cold P parity.

The first final 112 fixed sorted-prefix reader was **.21468 ns/probe (+4.90%)**. Two further clean processes yielded **.20265 / .20525**, for a three-process median of **.20525** against baseline **.20465**. Across 20 cells the final worst comparison is **+1.17%** (90 fixed membership). The real leaf-history read total moved **+0.23%**. Other final cells have a single clean sample; these are observations, not timing guarantees. All intermediate synthetic and admission correctness checks finished, but **not all intermediate real/scaled timing gates or cold replicates completed** before handoff. The tables explicitly expose the missing samples rather than copying measurements between executables. Raw evidence is left on disk; no outstanding measurements are running.
