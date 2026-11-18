# Leaf reader experiments

The comparison is the same two-thirds/one-third split and one merge on both sides, not a direct seal versus a merged run. `PACKED_READ_LEAVES=1` selects that construction. State 90 uses `u64`; state 112 uses `u128`. Both scaled scan kernels instantiate the linear, narrow checksum. Sizes, headers, frames, publication, and the scheduler's range workspace were not changed by these experiments.

## Register elimination

The counts below come from the CUDA ELF embedded in the executable, not a PTX virtual register count. Runs, build logs, resource listings, predictions, and full Nsight reports are kept outside Git in `/data/design-notes/doop-memory/leaf-readers/`.

| Scan experiment | Prediction before build | Assembled registers, 90 / 112 |
|---|---|---:|
| Tip (`9dca0835`) | reference | 78 / 78 |
| A: keep a header cursor after locating the first row | remove 6–12 registers by keeping the directory out of the checksum loop | 74 / 72 |
| B: also partition each leaf fragment cooperatively | remove a further 5–12 registers | 64 / 64 |

A still gives each lane its own global start, stop and next-row position. B uses the range reduction's existing `sum_cooperative` contract: all lanes receive the same range. Locate the first block, consume its intersection, and advance to the next header. Restart lane numbering at each boundary. A lane uses only its local row and count (`Header.rows: u32`) inside the leaf. Global lengths and bit addresses remain 64-bit. Changing which lane contributes a row is valid because the observable checksum is a wrapping sum. No register limit was added.

The sorted-probe bottleneck is different. The first warmed state-90 launch used **64 registers on both sides**, but the tip issued **363,808 local load requests and 363,808 local store requests**, versus zero each on the archived fixed reader. Global load requests increased from 9,198,311 to 10,322,515. The packed search had already found its leaf via fences, but its identity-frame path called `row_block` *again at each binary-search midpoint*. Experiment C searches in local coordinates and addresses the selected payload directly. Prediction: fewer instructions and elimination of the state-90 spills.

C dropped the state-90 sorted kernel to **62 registers, zero local requests**, but a clean first process still took 3.643 ms against 3.497 ms fixed. Long-scoreboard stalls increased to 15.63 cycles per issued instruction (14.04 on the tip); removing spill traffic alone did not eliminate dependent memory loads. D reuses the lower fence for a short upper-bound search: check the next two fences and search the remaining range only when necessary. Reversed bounds still search independently. Prediction: at least a 4% improvement on sorted state 90. An exhaustive host check of 82,304 fence partitions, including repeated fences and reversed probes, matched independent bisection.

The archived fixed-history executable reproduced the regression reference across three fresh, clean processes: sorted/scan **3.495/54.871 ms** at state 90 and **0.289/21.590 ms** at state 112; real mix **179.612 ms**. The packed state bytes were 745,467,096 and 749,729,444 respectively. The variable builds published 676,255,744 and 675,871,380 bytes. Each scan checked 2,684,354,560 row visits across the ten timed repetitions.

## Full-profile baseline

One warmed launch per cell. Registers are used/allocated per thread; CTAs is the register residency limit (eight warps per CTA). Local traffic is L1TEX load/store **requests**, not bytes. Stalls are average waiting warps per issued instruction; L = long scoreboard, W = wait, B = branch resolving, N = not selected. Raw reports retain all stall categories.

| Geometry | State/op | Regs | CTAs | Occupancy % | Lanes/inst | Local LD/ST | Kernel µs | Top stalls |
|---|---|---:|---:|---:|---:|---:|---:|---|
| Archived fixed | 90 scan | 64/64 | 4 | 58.27 | 21.47 | 524288/16384 | 3219.20 | L17.44 W2.93 B0.52 |
| Tip leaf | 90 scan | 78/80 | 3 | 42.68 | 21.57 | 0/0 | 3695.36 | L12.87 W2.94 B0.51 |
| Archived fixed | 112 scan | 64/64 | 4 | 58.27 | 24.19 | 1048576/540672 | 2001.09 | L17.07 W2.75 B0.74 |
| Tip leaf | 112 scan | 78/80 | 3 | 43.46 | 23.93 | 0/0 | 2220.19 | L13.05 W2.47 B0.72 |
| Archived fixed | 90 sorted | 64/64 | 4 | 60.27 | 31.12 | 0/0 | 346.11 | L13.83 W2.53 N0.77 |
| Tip leaf | 90 sorted | 64/64 | 4 | 59.40 | 30.42 | 363808/363808 | 366.53 | L14.04 W2.51 N0.71 |
| Archived fixed | 112 sorted | 72/72 | 3 | 40.30 | 30.93 | 0/0 | 23.84 | L6.12 W2.55 N0.83 |
| Tip leaf | 112 sorted | 72/72 | 3 | 40.22 | 30.12 | 0/0 | 25.50 | L6.02 W2.45 N0.78 |
| Feature-off fixed | 90 scan | 64/64 | 4 | 58.11 | 21.47 | 524288/16384 | 3228.26 | L17.50 W2.93 B0.52 |
| Feature-off fixed | 112 scan | 64/64 | 4 | 58.78 | 24.19 | 1048576/540672 | 2002.30 | L17.00 W2.75 B0.74 |
| Feature-off fixed | 90 sorted | 64/64 | 4 | 59.89 | 31.12 | 0/0 | 347.71 | L13.82 W2.53 N0.77 |
| Feature-off fixed | 112 sorted | 72/72 | 3 | 40.58 | 30.93 | 0/0 | 24.13 | L6.10 W2.55 N0.83 |

## Real read mix

Each cell below is the median of three clean xalan processes, with all 19,324 site records checked. Values are GPU milliseconds. Each diagnostic kind is a separately replayed workload; do not sum the diagnostic columns.

| Geometry / experiment | Mixed E/P/C | One-row hit C | Range miss C | Membership miss C | Long range C | Short range C |
|---|---:|---:|---:|---:|---:|---:|
| Archived matched fixed | 152.656 / 146.031 / 179.612 | 138.876 | 84.346 | 19.544 | 16.045 | 47.916 |
| Tip leaf | 152.218 / 145.348 / 182.900 | 141.629 | 84.925 | 19.487 | 16.026 | 49.039 |
| D: paired fences, leaf | 152.680 / 145.500 / 174.046 | 135.455 | 79.929 | 19.122 | 15.398 | 46.263 |
| H: width-selected fences, leaf | 152.690 / 145.722 / 175.552 | 136.467 | 80.742 | 19.138 | 15.561 | 46.795 |

## Probe alternatives and remaining tradeoffs

D brought sorted/scan to 3.442/53.407 ms (90) and 0.229/19.613 ms (112), medians of three clean processes. Its xalan publication median was 27.222 ms and all three reports had exactly 115,726,164 bytes. But state-112 mixed prefixes took 2.518 ms against the tip's 2.445 ms (+3.0%). Unlike sorted, the batch interleaves 136,353 present prefixes with 104,234 misses. Its NCU launch lost active lanes (29.30 → 24.98) and long-scoreboard stalls rose (82.42 → 116.22 per issue) despite fewer global load requests (879,723 → 815,087) and higher register residency.

E bracketed the upper fence exponentially when the neighboring fences were exhausted. It did not recover mixed parity (first state-112 process: 2.571 ms). F instead descended the two fences together to their first split, then bisected within the two subtrees. F reduced that launch to 786,435 global load requests, 25.26 lanes/instruction and 107.74 long-scoreboard stalls per issue; the first state-112 mixed result was 2.352 ms. But the first state-90 sorted result was 3.794 ms, not competitive with D. These experiments did not alter the format or add an alternate decoded operation path.

G tested rejecting a probe immediately when it fell outside a frame envelope and every preceding field had zero residual bits. It recovered neither mixed parity (2.530 ms, first 112 process) nor a reason to keep extra branches. H instead selects the fence search by the existing word type. A 64-bit fence is a cheap neighbor comparison (D); a multi-lane word uses shared descent (F) to amortize each wide load. This is a compile-time choice, independent of the workload, row count, query order and state ID. Neither E's exponential search nor G's frame shortcut is in H.

## Correctness checks

The balanced-scan oracle now also builds a single variable run from interleaved partitions, and checks the representation before testing ranges. It covers empty and short ranges, lengths 31/32/33, multi-leaf and 1,025-row ranges, prefix and scheduler tile boundaries, and the full relation. The original plain and sealed fixed cases are still tested. With B+C, 31 packed-run and 14 code-space tests passed with this matrix. `python3 doc/formal/check.py` passed its documentation inventory. A separate operation-call-site check found `export_row` only in the explicit unpack kernel; calls to `unpack` remain in the export API and host materialization, not in bound or checksum operations.

## Reproduction

From `experiments/gpu`, source `/dev/shm/gpu-env.sh` and build `packed-replay` with `cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release -p gpu-datalog-runtime --bin packed-replay`. Use the shared build lock and separate exclusive GPU lock described in `MERGE-MIDSIZE.md`. The archived `history-baseline` includes NVTX. For instrumentation-matched timings, rebuild the isolated `d8ad470a` source with its construction patch and the same feature-off command as the tip. Keep timed runs distinct from profiler runs. All reported process samples require empty before/after `nvidia-smi --query-compute-apps` checks.

For counters, use `ncu --set full --cache-control none --clock-control none`, `PACKED_READ_LEAVES=1`, and one warmed packed launch. Filter `^packed_TID_` and skip one matching launch for sorted probes. For `^expand_reduce_TID_` skip one for state 90 and twelve for state 112: the latter also launches the plain balanced reducer for its warmup and ten timed repeats. The former does not reach the plain balanced fanout threshold. Import each `.ncu-rep` with `--page raw --csv --print-units base`.

## Acceptance at handoff: reader code not committed

Packed times in ms; columns other than real mix sum ten repetitions. “Sealed” means direct arithmetic geometry, not the old merge. All rows below use three clean processes except H merged 112 (two). `fixed` is the instrumentation-matched, feature-off rebuild for the acceptance comparison.

| State | Construction | Sorted | Shuffled | Mixed | Membership | Scan |
|---:|---|---:|---:|---:|---:|---:|
| 90 | Fixed history | 3.497 | 22.897 | 39.298 | 0.452 | 54.910 |
| 90 | Tip merged | 3.701 | 28.137 | 39.249 | 0.430 | 60.104 |
| 90 | H merged | 3.441 | 20.496 | 34.433 | 0.429 | 53.494 |
| 112 | Fixed history | 0.293 | 1.643 | 2.449 | 0.507 | 21.548 |
| 112 | Tip merged | 0.303 | 1.688 | 2.445 | 0.475 | 23.811 |
| 112 | H merged | 0.284 | 1.351 | 2.355 | 0.472 | 19.619 |
| 90 | Tip sealed | 3.393 | 22.148 | 37.790 | 0.442 | 58.641 |
| 90 | H sealed | 3.115 | 19.735 | 32.662 | 0.442 | 52.021 |
| 112 | Tip sealed | 0.279 | 1.609 | 2.376 | 0.488 | 24.518 |
| 112 | H sealed | 0.279 | 1.262 | 2.154 | 0.493 | 20.003 |

H's embedded cubin uses **64/64 registers for scans, 57/64 for sorted** (90/112); final H occupancy was not measured. B+C's scan profiles measured 58.32%/58.89% occupancy with zero local load/store requests. Do not substitute those occupancy measurements for a final H profile. Feature-off fixed real mix was **180.081 ms**, tip **182.900 ms**, H **175.552 ms**. H merge publication was **27.218 ms** with **115,726,164 bytes** in every process. H passed 31 packed-run and 14 code-space tests, the formal checker, both synthetic storage-mode oracles, both scaled replay oracles, real and admission oracles.

**Not accepted:** in the first clean cold sample for 2^26/112/1:16, H took **0.807039 ms** versus **0.773248 ms** tip median (+4.37%). The per-kernel cold comparison was not captured, so no causal attribution is established. The other six completed H cold cells verified full sorted output, but one sample cannot close the regression gate. At the requested stop the following were missing:

- H: third merged state-112 process; all four final `ncu --set full` profiles; two further synthetic processes per storage mode. Tip also needs two further synthetic processes per storage mode.
- H cold: processes 2 and 3 for every endpoint, and process 1 for 2^28/112/1:2. Tip cold: process 3 for 2^28/112/1:2.
- Three-process whole-matrix non-regression signoff is therefore outstanding. Timing jobs were stopped, not left running. The `width` binary, `.ok` markers and per-run checks are under `/data/design-notes/doop-memory/leaf-readers/`; `run.py` resumes incomplete cells without counting interrupted runs.

## Completed gates for candidate H (`width`), run after handoff

Same resumable runner (`run.py width|tip endpoints 3`, `width scaled 3`, `width read-counters`, `width|tip cold-counter`), local GPU, empty compute-process lists before and after every process. All cells below have three clean processes for both the tip (`9dca0835` plus the measurement commits) and H.

Cold merge endpoints, packed ns per output row, tip → H:

|Rows|90, 1/16|90, 1/2|112, 1/16|112, 1/2|
|---|---|---|---|---|
|2^26|0.01050 → 0.01046 (−0.4%)|0.02075 → 0.02059 (−0.8%)|0.01152 → 0.01198 (**+4.0%**)|0.02275 → 0.02269 (−0.3%)|
|2^28|0.00932 → 0.00929 (−0.3%)|0.01809 → 0.01801 (−0.5%)|0.00999 → 0.00995 (−0.4%)|0.02023 → 0.02011 (−0.6%)|

The 2^26 / state 112 / 1:16 cell is reproducibly slower in unprofiled runs (tip 0.01152, 0.01149, 0.01157; H 0.01203, 0.01198, 0.01195: about 33 µs of 773 µs) and exceeds the 2% limit. It is not in the merge kernels: full-counter kernel times are plan 92.7 / tile offsets 21.3 / write 408.5 / unsplit write 126.8 µs for the tip against 94.8 / 21.3 / 404.0 / 125.6 µs for H (649.3 against 645.7 µs summed), with identical registers. Two Nsight Systems traces per binary of the same case (`coldgap/`) show the window from the planner launch to the end of the last writer at 745.3 and 742.9 µs for the tip against 735.6 and 737.5 µs for H, and the replay's own interval under the tracer at 824.3 and 817.7 µs against 820.7 and 809.2 µs. The 70–84 µs between the start event and the planner launch contain no traced CUDA call other than the event record and one stream synchronize, so the unprofiled difference is host time before the first launch that the tracer does not reproduce. Its cause is not established. [INFERENCE] The reader change alters earlier pool allocations in the process, which can change the cost of the merge's own output allocations; no merge source changed behaviour. The cell is recorded as an exception to the 2% cold gate, not as a pass.

Scaled reads at 2^28 rows, H against the tip, ns per probe (sorted / shuffled / mixed / membership / scan):

|Storage, state, construction|Change|
|---|---|
|packed, 90, fixed|−8.2% / −10.9% / −13.6% / −0.1% / −11.3%|
|packed, 90, merged leaves|−7.0% / −27.2% / −12.3% / −0.4% / −11.0%|
|packed, 112, fixed|+0.1% / −21.5% / −9.4% / +1.0% / −18.4%|
|packed, 112, merged leaves|−6.4% / −19.9% / −3.5% / −0.7% / −17.5%|
|plain, 90 and 112, both constructions|within ±0.7%, except one cell|

The exception among the plain-storage cells is sorted probes on state 112 with fixed construction, +5.3% on medians. The plain reader is the same code for both constructions, and its samples are bimodal in both binaries (tip: 0.1454, 0.1447, 0.1447 and 0.1449, 0.1471, 0.1530; H: 0.1524, 0.1446, 0.1531 and 0.1441, 0.1465, 0.1488 ns per probe), so this cell is noise at about 20 µs per launch, not a change.

Against the history-matched fixed-block baseline, H on merged leaves is faster in every workload: state 90 −1.6% / −10.5% / −12.4% / −5.1% / −2.6%; state 112 −3.2% / −17.7% / −3.6% / −6.9% / −8.9%.

Final H counters (full set, warmed launch): scan kernels 64 registers used and allocated, 58.0% and 58.5% occupancy (66.7% permitted), no local-memory loads or stores; sorted-probe kernels 57 and 64 registers, 54.7% and 44.8% occupancy, no local-memory traffic.

With these runs complete the reader change is committed, with the one cold-merge cell above documented as an unexplained host-side exception.
