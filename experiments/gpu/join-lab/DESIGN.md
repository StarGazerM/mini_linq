# Join lab: bounds, measurements and design rounds

Goal: DOOP joins an order of magnitude faster than SRDatalog's, approaching
this GPU's caps. Every claim below is a measurement (command and binary named)
or marked [INFERENCE]. Failed designs stay here with the measured reason.

## 1. Machine caps (RTX 6000 Ada, sm_89, 142 SMs, 96 MiB L2)

`target/release/join-lab caps` (2026-11-22), median of 7–9 event-timed runs,
grid = resident blocks (852 x 256 threads).

| cap | size | measured |
|---|---|---:|
| coalesced read | 2 GiB (DRAM) | 890.6 GB/s (111 G words/s) |
| coalesced read | 32 MiB (L2) | 3,339 GB/s (launch-dominated, lower bound) |
| random 8-byte load | 32 MiB table (L2) | 96.2 G loads/s |
| random 8-byte load | 2 GiB table (DRAM) | 6.92 G loads/s |
| binary search, random queries | 64 Ki keys (512 KiB) | 16.4 G/s |
| | 1 Mi keys (8 MiB) | 9.45 G/s |
| | 4 Mi keys (32 MiB, L2) | 8.29 G/s |
| | 16 Mi keys (128 MiB, > L2) | 2.47 G/s |
| | 256 Mi keys (2 GiB) | 0.79 G/s |
| binary search, ascending queries | 64 Ki – 256 Mi keys | 26.5 – 14.7 G/s |
| u64 atomic add | 1 counter | 2.03 G/s |
| | 1,024 counters | 96.8 G/s |
| | 65,536 counters | 244 G/s |
| empty kernel launch, stream-ordered | | 1.50 us |
| launch + 8-byte readback, host round trip | | 6.31 us |

Consequences:
- A probe that misses in a cache-resident index costs ~0.1 ns of machine
  time (random) or ~0.05 ns (ascending queries); in a 2 GiB index ~1.3 ns.
- Streaming a row costs ~0.009 ns per 8 bytes.
- A per-round host readback costs ~6 us; DOOP scala runs ~400 rounds x
  ~150 rule versions, so one readback per version per round is ~0.4 s.

## 2. Where scala's join time goes (doop-gpu with hand-written VPT_LoadArray@3)

Traced fixpoint 8.99 s = rule kernels 6.10 s + maintenance-only kernels
1.68 s + GPU idle 1.21 s (27,576 device-to-host readbacks, 92,749 launches).
Rule kernels by class (`profile-xlarge/analysis-manual5-scala.json` joined with
`counters-scala.log`):

| class | versions | time | delta rows probed | rate | caps-based bound |
|---|---:|---:|---:|---:|---:|
| A: scan delta, probe per row, ~all miss | 104 | 4.30 s | 8.6 G | 2.0 G rows/s | 1.04 s (random search, 8.3 G/s); 0.43 s (ascending, 20 G/s); 0.08 s (streaming) |
| B: expansions, output mostly duplicates | 18 | 1.24 s | 1.4 G | | |
| C: expansions, output mostly new | 23 | 0.57 s | | | |

Index layouts hold up to 8–9 runs (LSM ratio law), so one logical probe could
be up to 8–9 binary searches. The [INFERENCE] that runs x searches explain the
4x gap to the random-search cap was wrong; measured below.

## 3. Class A diagnosis (Nsight Compute, `/data/design-notes/join-lab/classA-ncu.md`)

Largest launches of holes 136/143/145 (the virtual-dispatch versions driven by
new `VarPointsTo` rows): 2.92–3.02 binary searches per delta row (first index
has one run), but **66–67% of PC samples and 67–68% of warp stall cycles are in
the block-cooperative expansion scans** (6 CTA barriers per flatten batch per
run, paid even when the batch has no children); 13–14% are in search loops.
Achieved occupancy 33–50%, eligible warps 0.13–0.21 per scheduler, issue
active 12–17%: the warps wait at barriers. Per delta row: 0.72–1.15 ns total,
of which searches 0.10–0.15 ns.

Hole 123 is different: one launch expands 2.11 M delta rows into 383.4 M
children before a membership test whose keys were bound from the start
(364.6 searches per delta row, 98.5% at the last layout). That is a plan-order
problem, not a kernel problem (follow-up: `plan-order.md`).

## 4. Design round 1 (class A): prefilter the delta before the join

Hypothesis: if the delta rows that cannot match the first probe never reach the
generated kernel, its scans run on 0.2% of the rows. Test: `src/bin/filter.rs`,
64 Mi synthetic delta rows, symbol domain 8 Mi, 20 K keys per version (0.25%
hits), CPU oracle on counts and on a checksum of the surviving rows.

| variant | versions | keys/version | time | rows/s | bound | oracle |
|---|---:|---:|---:|---:|---:|---|
| mask (per-symbol u32 bitmask, one load/row), per-row atomic | 1 | 20 K | 1.42 ms | 47 G | 1.30 ms | pass |
| same | 4 | 20 K | 1.71 ms | 39 G | 1.30 ms | pass |
| same | 16 | 20 K | 2.13 ms | 32 G | 1.30 ms | pass |
| binary search per version (sorted keys) | 1 | 20 K | 0.63 ms | 106 G | | pass |
| same | 4 | 20 K | 7.91 ms | 8.5 G | | pass |
| same | 16 | 20 K | 50.4 ms | 1.3 G | | pass |
| mask, per-row atomic | 1 | 2.5 M (30% hits) | 11.8 ms | 5.7 G | 1.30 ms | pass |
| mask, per-row atomic | 16 | 200 K | 10.0 ms | 6.7 G | 1.30 ms | pass |

Findings:
- One small index (80 KB of keys) stays in L1: a plain per-row binary search
  runs at 106 G rows/s. The generated kernels' 1–2 G rows/s is the emitter
  (barriers), not the search.
- With several versions sharing the delta, per-version searches degrade
  (16 versions: 1.3 G rows/s); one mask load serving all versions stays at
  32 G rows/s, within 1.6x of the streaming + one-L2-load bound.
- At 30% hits the per-row atomic append is the limit: 20 M appends to one
  counter at the measured 2 G atomics/s is 10 ms of the 11.8 ms. Warp-
  aggregated append (`filter_mask_warp`: one ballot per version, the leader
  reserves the group, lanes write at their rank) measures 1.94 ms (34.6 G
  rows/s, bound 1.30 ms), oracle pass on counts and row checksums.

### Round 1 in the engine: GPU-RootSelect (`sequence/core/src/emit.rs`)

A spread single-chain dispatch whose first step expands a radix/run lookup
keyed by root columns first runs a generated select kernel: per root row it
decodes only the key columns, tests membership in that provider (all runs),
and appends kept positions with warp-aggregated atomics. The dispatch reads
its root through the position list; its domain and spread tile come from the
device count (no readback). 66 dispatches in `doop-gpu` get a select kernel.

`doop-gpu --features manual-joins`, with the capacity-hint change
(`/data/design-notes/join-lab/idle-readbacks.md`), single runs, counts digests
unchanged on all four, xalan exact:

| dataset | before (manual5, median of 3) | capacity hint only (scala, median 3) | + RootSelect |
|---|---:|---:|---:|
| scala | 8.66 s | 8.12 s | 6.42 s |
| soot | 8.17 s | | 7.03 s |
| kotlin | 9.04 s | | 6.28 s |
| jython | 20.77 s | | 19.54 s |

SRDatalog's own DOOP program (`srd-doop-gpu`, field-flow, prepared datasets),
same emitter change, counts identical, single runs; SRDatalog live is its own
engine on the same machine:

| dataset | srd-ff before | srd-ff + RootSelect | SRDatalog live |
|---|---:|---:|---:|
| scala | 10.86 s | 9.69 s | 9.44 s |
| soot | 7.30 s | 6.59 s | 9.93 s |
| kotlin | 10.75 s | 9.35 s | 10.52 s |

Scala rule-kernel time (profile): 6.48 -> 4.44 s; `varpointsto@3#136`
0.79 -> 0.20 s; `reachable@3#143` and `callgraphedge@3#145` left the top 8.
Remaining top versions: `varpointsto@4#123` 0.65 s (plan order, section 3),
`varpointsto@0#122` 0.46 s, `varpointsto@3#132` 0.44 s (duplicates, below),
`arrayindexpointsto@3#126` 0.29 s, `@2#125` 0.24 s, `VPT_LoadArray@2#128`
0.22 s. Measured (`/data/design-notes/join-lab/hole122-ncu.md`): the select
pass on #136/#143/#145 runs at 13.9-14.3 G input rows/s (one binary search per
row into a 433 K-key index) and is 23-29% of each hole's kernel time; the
survivor dispatch is the rest. #122 wrote zero rows (69.1 M membership misses)
and spent 95-97% of sampled warp time polling for work to steal; reordering
its join (plan advice, `plan-advice.md`) took it from 461.3 to 24.5 ms.

### Capacity growth slack (`datalog-runtime/src/arena.rs`)

A hole above the 8 MiB speculative cap was offered exactly its last count, so
it failed (and reran) whenever its output grew: on jython 488 of 12,144
dispatches failed, 1.647 s of kernel time. The hint now adds half the last
count, at most 256 MiB. Same tree, slack 0 vs 256 MiB, two alternating runs,
digests unchanged:

| dataset | slack 0 | slack | peak bytes |
|---|---:|---:|---:|
| jython | 18.21, 18.24 s | 17.59, 17.49 s | +0.4% |
| scala | 5.28, 5.24 s | 5.18, 5.29 s (noise) | +1.1% |

## Synthetic class-A probes: keep the index resident, or re-sort the delta?

`probe` measures **membership**, not output materialization or two-column range expansion.
Delta columns are `u64 a, b` in `(a,b)` order, with unique increasing `a`. For
`M` index keys, each pair of adjacent domain slots has one pseudorandom member
and one in-range nonmember. Each run takes every Rth key, so all R key ranges
overlap. Uniform and discrete Zipf(1.1) target 0.2% hits; hot puts half
the rows on one nonmember and targets 0.4% hits in the other half. Miss is
uniform all-miss; hit30 is uniform with 30% hits. This is a dense symbol-code
domain, **not** an assumption that arbitrary u64 keys fit a direct bitmap.

Rates are G delta rows/s, median of seven CUDA-event intervals after warmup.
“Runs” searches until the first match, at R=1/4/8. Other strategies use a
maintained merged index or exact bitmap: the input to their probe is identical
for each R, and the result is shown once for all three. Building/uploading the
index, merging old runs and oracle readback are excluded. Ascending, merge-path
and distinct are shown **probe only / including delta pair-sort**. Sort is
GPU radix sort of u64 `(b,a)` using the significant bits of `b`; immutable
input restoration between samples is excluded. Merge-path co-ranks fixed
32-element diagonals, including across duplicate groups. Distinct searches
only group heads, then expands hit flags by loading `flag[b]` for each row
(one byte per domain slot). All methods reduce the count and wrapping sum
of `a XOR rotl(b,21)` on matching rows. **Every cell passed both checks**
against CPU binary search; CPU reference results are at right.

**1 Mi delta rows, 128 Ki index keys** (32 KiB bitmap):

| dist | runs 1 | runs 4 | runs 8 | merged | ascending | merge-path | distinct | bitmap | sort ms | hits | checksum (hex) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| uniform | 10.76 | 3.94 | 2.49 | 10.77 | 67.01/12.01 | 51.20/11.38 | 51.20/11.38 | 146.29 | 0.0717 | 2,096 | `00020b9cce07ee33` |
| zipf | 42.67 | 16.25 | 9.75 | 42.67 | 68.84/12.50 | 34.13/10.55 | 74.47/12.67 | 163.84 | 0.0687 | 2,096 | `00002f19e8a7ee33` |
| hot | 21.33 | 8.19 | 5.50 | 21.24 | 68.27/12.49 | 40.45/11.09 | 67.42/12.46 | 146.29 | 0.0686 | 2,067 | `0002033acb038c59` |
| miss | 10.70 | 3.94 | 2.48 | 10.76 | 64.76/11.80 | 49.05/11.15 | 51.69/11.28 | 144.99 | 0.0727 | 0 | `0000000000000000` |
| hit30 | 10.67 | 4.39 | 2.47 | 10.69 | 64.00/11.91 | 40.96/10.78 | 49.20/11.28 | 126.52 | 0.0717 | 314,886 | `0133a15d49039e59` |

**8 Mi delta rows, 4 Mi index keys** (1 MiB bitmap):

| dist | runs 1 | runs 4 | runs 8 | merged | ascending | merge-path | distinct | bitmap | sort ms | hits | checksum (hex) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| uniform | 7.19 | 2.16 | 1.20 | 7.21 | 65.54/7.52 | 26.09/6.41 | 44.28/7.13 | 88.09 | 0.9871 | 16,891 | `020d514e41027ac8` |
| zipf | 25.93 | 6.27 | 3.63 | 25.92 | 71.31/7.50 | 35.31/6.78 | 69.92/7.49 | 331.41 | 1.0005 | 16,891 | `001f1c563d027ac8` |
| hot | 12.64 | 3.23 | 1.56 | 12.66 | 68.84/7.58 | 27.04/6.48 | 55.73/7.39 | 157.54 | 0.9841 | 16,767 | `02094e0cb0c37271` |
| miss | 7.22 | 2.16 | 1.20 | 7.22 | 66.60/7.53 | 26.26/6.42 | 45.01/7.14 | 88.92 | 0.9882 | 0 | `0000000000000000` |
| hit30 | 7.03 | 2.39 | 1.36 | 7.02 | 38.64/6.97 | 19.55/5.92 | 24.75/6.33 | 60.68 | 0.9871 | 2,517,131 | `3364fc55d189415c` |

Large uniform: merging takes the probe from 1.20 to 7.21 G rows/s, **87% of
the 8.29 G/s random-search cap**. Ascending goes from 65.54 to 7.52 G/s
once its 0.987 ms sort is charged. Even completely eliminating its downstream
probe would give only 8.50 G/s. Bitmap needs no sort: 88.09 G/s is 73.3×
the eight-run scan, 79.4% of the 111 G u64 words/s DRAM streaming reference.
That reference is not a ceiling for these **warm** intervals: the 64 MiB
large `b` column and the 1 MiB bitset both fit in the 96 MiB L2; `a` is
only needed on hits. Across the same five distributions bitmap wins including
sort at both sizes. This does not measure the maintenance cost of a mutable
bitmap or the cost of consuming the surviving pairs in a join.

Reproduce from `experiments/gpu` (RTX 6000 Ada):

```sh
source /dev/shm/gpu-env.sh
export CARGO_TARGET_DIR=/data/tmp-targets/probelab
flock -s /tmp/mini_linq-timing.lock timeout 1200 cargo +1.99.1 oxide build \
  --arch sm_89 --materialize-cubin -- --release -p join-lab --bin probe
flock /tmp/mini_linq-gpu.lock flock -x /tmp/mini_linq-timing.lock \
  "$CARGO_TARGET_DIR/release/probe"  # full TSV; or: large uniform 8 runs
```

**Why bitmap wins.** Nsight Compute, large input, same grid (852 × 256),
one selected launch after warmup. `inst/row` counts executed *thread*
instructions, `sectors/row` counts 32 B global-load sectors at L1TEX, not
DRAM requests. Long scoreboard is % active warp cycles waiting for L1TEX.

| dist / operator | NCU µs | inst/row | sectors/row | DRAM read B/row | L2 hit % | long scoreboard % | occupancy % |
|---|---:|---:|---:|---:|---:|---:|---:|
| uniform / runs 8 | 6,994.1 | 1,864.85 | 120.96 | 12.06 | 99.52 | 97.26 | 99.19 |
| uniform / bitmap | 97.3 | 21.43 | 1.25 | 8.16 | 79.18 | 97.35 | 97.79 |
| zipf / bitmap | 80.9 | 21.43 | 0.71 | 8.16 | 32.48 | 88.71 | 96.50 |

On uniform, eight runs execute 87× more instructions and request 97× more
sectors per row than the bitmap, even though nearly every L2 lookup hits.
Neither low occupancy nor DRAM capacity explains the 6.98 ms search.
Bitmap still needs a data-dependent lookup after loading `b`. NCU recorded
68.45 MB read in 97.3 µs (703 GB/s vs the measured 891 GB/s sequential
cap); long-scoreboard waits dominate with near-full occupancy. Zipf uses
44% fewer load sectors at the same instruction count. Its warm event
median (25.3 µs) differs materially from its profiled launch (80.9 µs);
do **not** attribute the NCU DRAM bytes to that faster event interval or
interpret 331.4 G rows/s as sustained DRAM bandwidth.

```sh
metrics=gpu__time_duration.sum,smsp__thread_inst_executed.sum,l1tex__t_sectors_pipe_lsu_mem_global_op_ld.sum,dram__bytes_read.sum,lts__t_sector_hit_rate.pct,smsp__warp_issue_stalled_long_scoreboard_per_warp_active.pct,sm__warps_active.avg.pct_of_peak_sustained_active
for variant in 'uniform runs search_runs' 'uniform bitmap bitmap_probe' 'zipf bitmap bitmap_probe'; do
  read -r dist method kernel <<< "$variant"
  flock /tmp/mini_linq-gpu.lock flock -x /tmp/mini_linq-timing.lock \
    ncu --cache-control none --clock-control none --kernel-name-base function \
    -k "$kernel" --launch-skip 1 --launch-count 1 --metrics "$metrics" \
    --page raw --csv --print-units base "$CARGO_TARGET_DIR/release/probe" large "$dist" 8 "$method"
done
```

## Duplicate provenance: real hole 132, not whole-relation cardinalities

`dup` replays every retained hole-132 round. For each `(signature,baseheap)` group, its CPU oracle enumerates reachable load witnesses with `(baseheap,base)` in the **captured full** `varpointsto[0,1]`. It forms `(heap,to)` keys in raw symbol space, independently of device target coding. The multiplicity partition first removes repeated delta triples, then extra witnesses with the same base, then extra bases for a target within a group, then extra groups producing the same output. “Old” counts **distinct** keys present before firing; fresh = unique − old. It does not mean that all fresh keys are exclusive to this rule.

| Round | Delta rows | Groups | Candidates | Excess: repeated delta / same base / multiple bases | Excess across groups | Unique | Old | Fresh |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| jython 88 | 4,984,056 | 8,606 | 26,590,143 | 0 / 0 / 0 | 26,471,868 | 118,275 | 77,390 | 40,885 |
| jython 105 | 3,776,258 | 9,314 | 42,616,697 | 0 / 0 / 8 | 42,431,496 | 185,193 | 73,231 | 111,962 |
| scala 186 | 4,076,615 | 14,264 | 7,379,159 | 0 / 0 / 0 | 7,088,219 | 290,940 | 64,468 | 226,472 |
| scala 199 | 2,397,878 | 15,531 | 2,286,808 | 0 / 0 / 0 | 2,023,559 | 263,249 | 84,456 | 178,793 |

**Group the work, not just the output.** For `g=(signature,baseheap)`, factor `H(g) × T(g)` with distinct heaps and targets. This removes the first three excess categories, but just **eight** writes in four rounds. It *does* cut load-witness examinations from 90,714,170 → 1,245,038 (jython 88), 85,941,367 → 1,326,747 (105), 11,028,712 → 36,460 (scala 186), and 3,945,327 → 38,394 (199). Distinct `(g,to)` sizes are respectively 47,509, 68,269, 22,735, 24,365. Eliminating the last category requires dedup **across** groups. Early subtraction alone isn't enough: in round 105 it still writes 41,003,000 witnesses for 111,962 fresh keys.

**Resident index feasibility.** A full raw-ID `(heap,variable)` bitmap would need 606 GiB on jython and 1,156 GiB on scala. Even a *round-local* dense product of only delta heaps by distinct static load targets takes 158–398 MiB. Instead the replay uses the captured full relation in heap-major order: `u32 offset[heap..heap+1]` indexes a contiguous sorted array of variable IDs. The direct directory is 9.7 / 13.2 MB (jython / scala), variable payloads are 186, 565, 1,139, 1,361 MiB respectively, and maximum range sizes are 15,782, 40,022, 33,349, 36,829. A fresh test is therefore a bounded binary search in an exact per-heap slice, not a search across hundreds of millions of pairs. This is a resident CSR index benchmark; making or updating this index in the runtime is **not** timed. The capture format already consolidated sorted runs; these numbers are not execution times of the generated LSM plan.

### Which GPU operator should write the rows?

Each cell is **join ms / output rows / sort ms**; sort is a separate RCCL-CUB radix sort on the *actual unsorted output*, `u64`, bits `0..64`. The join interval includes clearing counters/hash, constructing the delta group index, building `T`, and producing keys, where applicable. It excludes allocation, CPU oracle, uploads, static index construction, and D2H validation. Scratch capacity is pre-sized from the capture (including group count and hash capacity); CPU does not supply group membership, target lists, fresh flags, or answers. Same 852 × 256 grid, one warmup and median of five stream-event samples per measurement. Sort inputs are restored **outside** the sort interval. Every cell passed: bag count, complete sorted distinct keys (not a checksum), and complete fresh set after applicable subtraction; raw count also matched the original manifest.

| GPU design | jython 88 | jython 105 | scala 186 | scala 199 | Oracle |
|---|---:|---:|---:|---:|---|
| Raw per delta, all witnesses | 3.066 / 26,590,143 / 4.325 | 3.444 / 42,616,697 / 7.045 | 2.227 / 7,379,159 / .799 | 1.291 / 2,286,808 / .196 | pass |
| Group, emit `H × T` | 2.972 / 26,590,143 / 4.393 | 2.474 / 42,616,689 / 7.071 | 1.883 / 7,379,159 / .800 | 1.215 / 2,286,808 / .195 | pass |
| Group + round hash | 1.268 / 118,275 / .074 | 1.323 / 185,193 / .078 | .676 / 290,940 / .085 | .504 / 263,249 / .082 | pass |
| Group + subtract before write | 2.065 / 3,348,646 / .229 | 2.413 / 41,003,000 / 6.810 | 2.211 / 7,289,023 / .783 | 1.384 / 2,177,456 / .188 | pass |
| Group + hash then subtract | 1.295 / 40,885 / .070 | 1.382 / 111,962 / .074 | .857 / 226,472 / .081 | .628 / 178,793 / .078 | pass |
| Group + heap-owned shared bitmap + subtract | 1.051 / 40,885 / .070 | 1.533 / 111,962 / .074 | 1.114 / 226,472 / .081 | .878 / 178,793 / .078 | pass |
| **Bitset union by `(heap,signature)` + hash then subtract** | **.760 / 40,885 / .070** | **.943 / 111,962 / .074** | **.606 / 226,472 / .082** | **.568 / 178,793 / .078** | pass |

The open-addressed round hash occupies 2, 4, 8, 8 MiB respectively (96 MiB L2). It still attempts an insert for every expanded candidate. Owning each heap in one block avoids the global hash and needs just one bitmap of static load targets **per CTA** (17.3/19.4 KiB); it still processes the entire `H × T` bag before dedup. On jython 105 the isolated emit stages take .994 ms (hash only), 1.015 ms (hash + subtract), and 1.122 ms (shared bitmap + subtract). The owner bitmap is **not** invariably faster.

**Choose bitset union for fresh output on these captures.** Order static loads by `(signature,target,base,method)`. For each `(signature,baseheap)` generate a signature-local bitmap of the first load row for each successful target. The captured delta is already sorted by `(heap,signature,baseheap)`: OR the bitmaps of the baseheaps in each consecutive `(heap,signature)` segment **before** enumerating any answer. Each set bit now denotes one distinct target within the segment. Use the small round hash for the remaining cross-signature duplicates; winners search the per-heap `varpointsto` range and append only if absent. No delta sort or intermediate candidate bag. For jython 105 this visits 4,655,367 bitmap words across 54,545 segments and at most 474,753 load positions after union, versus 42,616,689 expanded candidates before union. For jython 88 / scala 186 / scala 199 the analogous `word reads / post-union upper bound` are 5,349,604 / 372,940; 4,096,161 / 670,661; and 2,411,105 / 883,372.

Relative to raw, union saves **4.256, 6.972, .717, .118 ms** of measured sort in the four rounds. Even charging both designs for sorting their output (fresh keys still need sorted publication), isolated join + sort is 8.9× / 10.3× / 4.4× / 2.3× faster. These are **not** full-engine speedups: the 43.4 G jython `varpointsto` candidates include other holes/rounds; neither their duplication nor their savings are inferred here.

### Distance from the reads-plus-one-write bound

[INFERENCE] Optimistic phase model: stream each delta triple and write each unique fresh key once at 890.6 GB/s; serve each `(group,load)`'s 12 B of static load data at the 3,339 GB/s L2 streaming cap; idealize membership as **one** resident random access per reachable group witness plus one per unique answer at 96.2 G/s. Thus `T_stream=(12*delta + 8*fresh)/890.6e9`, `T_idx=12*group_loads/3339e9 + (reachable_group_loads + unique)/96.2e9`. Reachable group loads: 822,945; 884,766; 31,260; 33,179. These are optimistic cap comparisons, not measured DRAM transactions or an impossibility proof: static loads may repeat in cache, phases may overlap, actual CSR membership costs multiple dependent loads, and real global index maintenance is excluded.

| Round | Stream µs | Ideal index µs | Serial cap model µs | Measured union µs | Measured / model |
|---|---:|---:|---:|---:|---:|
| jython 88 | 67.52 | 14.26 | 81.78 | 759.7 | 9.3× |
| jython 105 | 51.89 | 15.89 | 67.78 | 943.1 | 13.9× |
| scala 186 | 56.96 | 3.48 | 60.44 | 606.2 | 10.0× |
| scala 199 | 33.92 | 3.22 | 37.13 | 568.3 | 15.3× |

**Where the gap actually goes, jython 105:** isolated warm stage medians were 111 µs for group/segment indexing, 200 µs for building `T`, and **556 µs** for union/admission. Nsight Compute on one warm execution, no cache flush or clock control:

| Kernel | Profiled µs | Thread instructions, M | Global-load sectors at L1, M | DRAM read / write MB | L2 hit % | Long scoreboard % | Occupancy % |
|---|---:|---:|---:|---:|---:|---:|---:|
| `dup_index_groups` | 118 | 291 | 17.37 | 45.35 / 3.98 | 82.9 | 84.0 | 86.3 |
| `dup_targets` | 276 | 229 | 6.13 | 14.30 / .02 | 46.5 | 32.2 | 19.4 |
| `dup_union` | 640 | 265 | 11.89 | 27.58 / .39 | 87.0 | 42.2 | 33.5 |

The union kernel spends 640 µs moving only 28 MB at DRAM (~44 GB/s). The cap model omits 4.66 M dependent `slot → T range → bitmap word` traversals, warp reductions, hash claims, and binary-search steps. The measured 11.89 M global-load sectors, 42% long-scoreboard stalls and 33% occupancy explain why removing candidate writes is not enough to approach streaming bandwidth. The event table, not NCU replay timings, is used for speedups.

```sh
cd /data/mini_linq/experiments/gpu
source /dev/shm/gpu-env.sh
export CARGO_TARGET_DIR=/data/tmp-targets/duplab
/usr/bin/flock -s /tmp/mini_linq-timing.lock timeout 1200 \
  cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release -p join-lab --bin dup
cap=/data/design-notes/join-lab/captures
bin=$CARGO_TARGET_DIR/release/dup
for d in "$cap"/{jython,scala}/*__h132/*; do
  /usr/bin/flock /tmp/mini_linq-gpu.lock /usr/bin/flock -x /tmp/mini_linq-timing.lock \
    "$bin" "$d" all  # `cpu` for provenance only; or name a single design
done
metrics=gpu__time_duration.sum,smsp__thread_inst_executed.sum,l1tex__t_sectors_pipe_lsu_mem_global_op_ld.sum,dram__bytes_read.sum,dram__bytes_write.sum,lts__t_sector_hit_rate.pct,smsp__warp_issue_stalled_long_scoreboard_per_warp_active.pct,sm__warps_active.avg.pct_of_peak_sustained_active
/usr/bin/flock /tmp/mini_linq-gpu.lock /usr/bin/flock -x /tmp/mini_linq-timing.lock \
  ncu --cache-control none --clock-control none --kernel-name-base function \
  -k 'regex:dup_(index_groups|targets|union)$' --launch-skip 3 --launch-count 3 \
  --metrics "$metrics" --page raw --csv --print-units base "$bin" "$cap"/jython/*__h132/105 union
```

## Real virtual dispatch: replay `varpointsto@3#136`, Scala

`replay` runs the entire eight-probe chain from `seqdump/827991b974ddc9bb.input.sequence`, including nonunique expansions and materializing every `(heap,this)` candidate. The baseline assigns one thread per delta row; the prefilter warp-compacts rows whose `base` is in a dense bitset of layout 122, then launches the same chain on those rows, with a device-resident survivor count. Only lane order *within* each warp is preserved. The manifest count, a CPU sorted-array join, both GPU counts, order-independent SplitMix64 sums, and **sorted candidate multisets** agree.

| Round | Delta rows | Pass bitset | After base expansion | Candidates | Checksum |
|---|---:|---:|---:|---:|---|
| 186 | 5,685,221 | 212,132 (3.73%) | 254,958 | 253,606 | `c9e808047a4483b7` |
| 187 | 5,928,628 | 246,248 (4.15%) | 293,816 | 291,913 | `754f74324e61b3e9` |

CUDA-event milliseconds, median of **nine** after warmup. Index build includes zeroing; per-round intervals exclude count clears, uploads, allocations, and oracle downloads. “Both” brackets the two launches on one stream, not the sum of independently timed medians. The bitset occupies 376,240 B for 320,356 distinct keys in 433,372 EDB rows; input 8 was byte-identical across the two captures. At 407 invocations the 9.2–9.4 µs build contributes ~0.023 µs per round.

| Round | Index build | Baseline | Bitset + compact | Survivor chain | Both | Original nsys launch(es) | Speedup vs original successful launch |
|---|---:|---:|---:|---:|---:|---|---:|
| 186 | 0.00944 | 0.64307 | 0.05632 | 0.40448 | 0.46275 | 5.84503 | 12.63× |
| 187 | 0.00922 | 0.86938 | 0.06554 | 0.55706 | 0.63574 | 6.79157 then 6.67106 | 10.49× |

The original numbers are `CUPTI_ACTIVITY_KIND_KERNEL.end-start` in `profile-xlarge/nsys-manual5-scala.sqlite`, short name containing `kernels_827991b974ddc9bb_`. Sorted by start, zero-based launch ordinals **268, 269, 270** of 491 (correlation IDs 1210667, 1217327, 1217335) align with `share/trace-manual3-scala.log`'s 491 ordered dispatches. Round 187 exceeded its 253,606-row hint and retried at 291,913; counting both original launches gives 21.18×. Replay uses manifest-sized output. Replay flattens original runs: `reachable` had 3/4 runs, `methodlookup` 5; the other seven inputs had one. These are per-hole kernel comparisons, **not** whole-engine speedups.

**Bound and remaining cost.** Reading an 8-byte delta row at 890.6 GB/s costs 51.1/53.3 µs (186/187); one L2 lookup per row at 96.2 G/s costs 59.1/61.6 µs. With perfect overlap the larger is the reference; serializing them gives 110.2/114.9 µs. The measured warm filter (56.3/65.5 µs, 101/90 G rows/s) lands near the overlap reference, not a hard floor for a 368 KiB bitset and a 44–45 MiB delta in 96 MiB L2. The **chain**, not the bitset, is 87–88% of the combined time. The join cannot be priced as only one random load per delta row: for example in round 187 all 293,816 expanded invocations pass reachable, 292,656 have a heap type, and 291,913 rows must be written. On the direct baseline, 86,612 of 185,270 input warps had a first hit, averaging only 2.84 hit lanes in those warps (max per-lane base fanout 65). Compaction improves measured full join time 1.37–1.39× vs this baseline, not 24–27× (the inverse passing fraction).

Nsight Compute, round 187, one post-warmup launch, cache/clock control `none`. Instructions count *thread* instructions; sectors count 32-byte global-load sectors requested from L1TEX. Denominator is input rows of the operator, **246,248 for the survivor chain**.

| Operator | Profiled µs | inst/row | sectors/row | DRAM read MB | L2 hit % | Occupancy % |
|---|---:|---:|---:|---:|---:|---:|
| Baseline | 887.0 | 297.8 | 12.24 | 23.28 | 97.64 | 62.92 |
| Bitset + compact | 77.4 | 35.5 | 0.94 | 47.56 | 49.99 | 76.61 |
| Survivor chain | 549.9 | 2,345.5 | 98.49 | 7.39 | 96.25 | 47.41 |

The profiled filter streams 614 GB/s of DRAM reads. The survivor chain reads only 13.4 GB/s from DRAM yet requests 24.25 M load sectors, with long-scoreboard stalls at 78.6% of active warp cycles. Its dependent binary searches and expansions are substantially different work from the stream-plus-one-load reference, even with 96% L2 hits. The timing table uses non-profiled events, not these counter-run durations.

```sh
cd /data/mini_linq/experiments/gpu
source /dev/shm/gpu-env.sh
export CARGO_TARGET_DIR=/data/tmp-targets/capture CUDA_OXIDE_PTX_DIR=/data/tmp-targets/capture/oxide-ir
flock -s /tmp/mini_linq-timing.lock timeout 1200 cargo +1.99.1 oxide build \
  --arch sm_89 --materialize-cubin -- --release -p join-lab --bin replay
cap=/data/design-notes/join-lab/captures/scala/varpointsto@3__h136
bin=$CARGO_TARGET_DIR/release/replay
flock /tmp/mini_linq-gpu.lock flock -x /tmp/mini_linq-timing.lock "$bin" "$cap/186" "$cap/187"
metrics=gpu__time_duration.sum,smsp__thread_inst_executed.sum,l1tex__t_sectors_pipe_lsu_mem_global_op_ld.sum,dram__bytes_read.sum,lts__t_sector_hit_rate.pct,smsp__warp_issue_stalled_long_scoreboard_per_warp_active.pct,sm__warps_active.avg.pct_of_peak_sustained_active
for case in 'replay_chain 1 base' 'replay_filter 1 filter' 'replay_chain 11 compact'; do
  read -r kernel skip name <<< "$case"
  flock /tmp/mini_linq-gpu.lock flock -x /tmp/mini_linq-timing.lock \
    ncu --cache-control none --clock-control none --kernel-name-base function -k "$kernel" \
    --launch-skip "$skip" --launch-count 1 --metrics "$metrics" --page raw --csv --print-units base \
    "$bin" "$cap/187" > "$CARGO_TARGET_DIR/replay-187-$name.csv"
done
```

## Generic round-hash terminal: when should a generated join bypass the candidate bag?

**Decision:** use a shared exact table selectively, per **head and round**, not per hole. On jython a u64 terminal could suppress **41.81 G writes (334.45 GB)** over the fixpoint, but handling a scattered stream in a DRAM-resident hash table is itself expensive. On scala the ceiling is **1.74 G writes (13.91 GB)**; many large heads are close enough to unique that sorting directly wins. This generic terminal does not remove join expansion work like the hole-132 bitset union does.

### Whole-run census

The complete tables (including zero admissions) are `/data/design-notes/join-lab/hashdedup-scala.tsv` (3,416 rows) and `hashdedup-jython.tsv` (2,210 rows): `(stratum,phase,round,relation,candidates,unique,fresh,word_bytes)`. `unique` means **distinct keys from all contributing holes of this head, before subtracting old facts**. Do not use the `rows` on `sort/portion-unique` as its output: it is its **input**. `hashdedup census` aligns canonical portions with each admission, verifies their total input, and distinguishes inter-portion subtraction from the final `uncommitted` subtraction. The last subtraction's input is the cross-portion union; with no final subtraction, all unique rows are fresh. Ignore known-unique layout permutation sorts. All 3,416 scala `gpu_datalog_admission` records matched `profile-xlarge/counters-scala.log` verbatim. The old expansion-counter driver OOMs on jython's 2.85 G-row *per-hole diagnostic* concatenation in round 114; the ordinary portioned driver with both runtime traces enabled completes. No truncation or imputation.

| Head | scala candidates / within-round unique / fresh | jython candidates / within-round unique / fresh |
|---|---:|---:|
| **All heads** | **3,189,911,098 / 1,450,279,792 / 862,010,176** | **43,885,281,816 / 2,076,795,960 / 580,393,674** |
| u64 heads only | 3,004,156,340 / 1,265,733,944 / 678,861,940 | 43,704,407,581 / 1,898,085,043 / 475,131,171 |
| `varpointsto` | 2,774,341,980 / 1,134,392,472 / 612,373,174 | 43,372,222,635 / 1,856,733,709 / 438,304,400 |
| `instancefieldpointsto` (128-bit) | 183,181,137 / 182,035,478 / 180,638,466 | 178,509,741 / 176,349,034 / 102,901,097 |
| `arrayindexpointsto` | 130,066,837 / 113,736,578 / 52,802,723 | 9,938,560 / 5,930,329 / 4,708,109 |
| `callgraphedge` | 43,180,300 / 4,002,141 / 505,069 | 148,288,622 / 9,675,482 / 6,640,187 |
| `reachable` | 43,181,132 / 455,023 / 61,066 | 148,289,334 / 290,628 / 36,775 |

Examples of the spread: jython `varpointsto` round 114 is **3,026,673,382 / 20,068,183 / 10,136,409**; round 167 is **2,142,651,574 / 93,862,232 / 3,362,160**. A low fresh count is not a small table-size estimate! The hash alone reduces jython VPT's 43.37 G to **1.86 G**, not 438 M; subtraction still matters. Wide heads need a full-key representation, not a truncated u64 hash pretending to be equality.

### Hash insert vs actual RCCL sort + unique

`hashdedup bench`: 44 active bits in u64 keys; open addressing, zero sentinel plus a separate zero-key slot; optimistic load, linear probe, atomic CAS on empty. Table capacity = next power of two ≥ `2U`. Hash interval includes table/counter clear and warp-aggregated scatter of first inserts. `match` elects one lane per equal u64 key using `match_any_i64_sync` before probing. The baseline radix-sorts and unique-copies **the identical input**; after hashing, measured **sort(U)** is also charged for sorted publication. Pre-generated input read is common to both; generation, copies used to restore inputs between samples, allocation of benchmark buffers and oracle D2H are not timed. These are not fused engine times. 852 × 256 grid, warmup + median of 5 stream-event samples. All **58** measured cases (45 base + 8 large + 5 crossover) passed full output checks for hash, match, sorted hash outputs, and sort/unique: cardinality plus invertible key decoding and an exact CPU rank bitmap, not a checksum.

Two orderings: `scattered` covers U distinct keys on the first pass, then draws near-uniformly with replacement; `adjacent` places repeated occurrences next to one another. Inputs through 200 M candidates are materialized. To exercise **billions** without requiring both bags and sort scratch to fit in VRAM, `stream` repeats one physical tile and keeps a single hash across tiles. Its sort control uses the runtime's RCCL radix/unique **and** subtracts against the accumulated sorted set between portions. The oracle checks the initial portion's entire set and the final set's cardinality (the former is a subset of the latter). Identical tiles are a controlled cache/locality experiment, not measured DOOP emission order; adjacent run length = tile factor, not whole-stream factor. Full rows/rates in `hashdedup-{bench,large,crossover}.tsv` in the same notes directory.

| U; table | Dup factor; order | Raw hash / match (ms) | G input attempts/s, raw / match | sort(U) after raw (ms) | Direct sort+unique (ms) |
|---|---|---:|---:|---:|---:|
| 0.1 M; 2 MiB | 1; scattered | .015 / .015 | 6.57 / 6.51 | .060 | .071 |
| 0.1 M; 2 MiB | 400; scattered | .650 / .673 | 61.5 / 59.5 | .060 | 6.407 |
| 0.1 M; 2 MiB | 400; adjacent | .382 / .380 | 104.7 / 105.3 | .057 | 6.596 |
| 1 M; 16 MiB | 64; scattered | 1.101 / 1.152 | 58.2 / 55.6 | .102 | 10.541 |
| 1 M; 16 MiB | 64; adjacent | 1.291 / .726 | 49.6 / 88.2 | .103 | 10.591 |
| 4 M; 64 MiB | 16; scattered | 2.475 / 2.475 | 25.9 / 25.9 | .260 | 10.499 |
| 8 M; 128 MiB | 16; scattered | 12.169 / 12.172 | 10.5 / 10.5 | .721 | 21.407 |
| 8 M; 128 MiB | 16; adjacent | 4.213 / 3.241 | 30.4 / 39.5 | .717 | 21.629 |
| 8 M; 128 MiB | 400; scattered, 25 portions | 273.55 / 273.69 | 11.70 / 11.69 | .700 | 602.95 |
| 20 M; 512 MiB | 8; scattered | 27.34 / 27.32 | 5.85 / 5.86 | 2.462 | 27.63 |
| 20 M; 512 MiB | 160; scattered, 20 portions | 464.71 / 464.79 | 6.89 / 6.89 | 2.459 | 653.47 |
| 20 M; 512 MiB | 160; adjacent, 20 portions | 79.16 / 79.31 | 40.43 / 40.35 | 2.461 | 657.97 |
| 100 M; 2 GiB | 2; scattered | 54.21 / 54.09 | 3.69 / 3.70 | 13.25 | 36.21 |
| 100 M; 2 GiB | 24; scattered, 12 portions | 405.28 / 405.35 | 5.92 / 5.92 | 13.22 | 568.73 |
| 100 M; 2 GiB | 24; adjacent, 12 portions | 220.75 / 220.76 | 10.87 / 10.87 | 13.24 | 556.80 |

**Break-even depends on table, locality, and publication order.** For scattered keys and `hash + sort(U)`, first winning tested factor: 2× at 0.1 M and 1 M U (margins only 1–2 µs; use 4×), 2× at 4 M, 4× at 8 M. At 20 M U, 8× loses (29.80 vs 27.63 ms) and 16× wins (56.39 vs 63.93 ms). At 100 M U, 4× loses (99.17 vs 85.61 ms), 8× wins (163.02 vs 181.54 ms); these last comparisons **include inter-portion dedup** on the sort side. `match_any` pays on genuinely within-warp duplicates (1 M U adjacent at 64×: 1.78× faster insert) but not on scattered keys. High overall duplication does *not* imply intra-warp duplication.

**Measured reason the L2 result doesn't extrapolate:** NCU `hd_insert`, scattered, first timed raw hash launch, expressed per candidate (counters exclude the separate clears). At 1 M U / 16×: **6.93 DRAM read B**, 0.72 write B, 87.9% L2 hits, 78.2% long-scoreboard, 94.6% occupancy. At 8 M / 16×: **26.47 read B**, 2.66 write B, 53.7% hits, 96.3% long-scoreboard, 98.9% occupancy. At 100 M / 2×: **41.83 read B**, 20.16 write B, 40.3% hits, 96.5% long-scoreboard, 99.4% occupancy. The random claims/probes cost sectors, not just one 8-byte key load. High occupancy does not hide all of the round trips.

### Capacity, replay, and projected whole-run cost

[INFERENCE/design] Share one table across **all** contributing holes before admission. For u64 heads, retain up to the last eight completed counts for the same stratum and relation. Require ≥800 K historical candidates total. Let `S=next_pow2(max(200000, 4*max(previous U)))` slots; sort directly if `8*S > 2 GiB` or total history `C/U < 4` (table ≤128 MiB), `< 16` otherwise. Default to raw hash for unknown locality. The 4× history headroom is deliberate; capacity from last **fresh** is unsafe. Cap unique output at `S/2` to keep probe chains short, and check the allocator's actual remaining budget across heads, unique output and pending publication. A 128 MiB table is already *larger* than this GPU's L2, not an L2 guarantee.

On table/output overflow or the 128-probe budget, **set the failure bit; do not publish any partial output**. Replay all contributing holes for that head against the same frozen input snapshot with the ordinary sort terminal (no hash side effects from a failed capacity attempt may survive). `hashdedup overflow` exhausts a 64-slot / 32-output table, discards the partial result and oracle-checks all 1,024 keys from sorting the original 8,192 candidates. The microbenchmark retains the bag; a fused implementation must retain replayable inputs instead. Do not silently drop colliding keys or assume a capacity retry can re-use the old table.

`hashdedup policy` backtests that rule using **only earlier** rounds to decide and size. On scala: 394 nonempty selected rounds, 0 cardinality overflows, **77.28 M → 3.71 M** writes. On jython: 387 fitting rounds, **30.06 G → 1.012 G** writes, plus 3 fallback rounds totalling 5.18 M candidates, all `callgraphedge`. Largest concurrent sum of selected nonempty tables: 4 MiB / 2,082 MiB respectively, *before* output/sort space. Actual skew can hit the probe limit earlier. For comparison, using 2× *last round* U with the same capacity/rate tiers left **46** jython requests over half capacity (15.54 G candidates in failed rounds); a sizing miss at the wrong peak can dominate the run.

[INFERENCE, **not** an engine speedup] Calibrating the fitting rounds to scattered synthetic samples, scala hash / post-sort / direct sort+unique ≈ **2.0 / 22.5 / 29.0 ms**; jython ≈ **4,973 / 142 / 6,337 ms**. That is only **~1.22 s modeled savings** on jython's selected rounds despite eliminating 29.05 G writes. `policy` selects the nearest sample in log-size/dup-factor space, scales hash and direct sort by candidate count and post-sort by U, and applies minimum launch/sort/clear costs. This omits join replay on overflow, interference with live join indexes, real key ordering, kernel fusion/register costs and subtraction. Do not subtract these estimates from measured fixpoint time. The 79 vs 465 ms observed on two orders of the 20 M / 160× stream is reason to measure a fused prototype on real candidate order before shipping.

### Reproduce

```sh
cd /data/mini_linq/experiments/gpu && source /dev/shm/gpu-env.sh
export CARGO_TARGET_DIR=/data/tmp-targets/duplab
out=/data/design-notes/join-lab
prof=/data/design-notes/doop-memory/profile-xlarge
data=/data/doop-bench/corpus-20261110/raw
bin=$CARGO_TARGET_DIR/release/hashdedup
/usr/bin/flock -s /tmp/mini_linq-timing.lock timeout 1200 cargo +1.99.1 oxide build \
  --arch sm_89 --materialize-cubin -- --release -p join-lab --bin hashdedup
locked() { /usr/bin/flock /tmp/mini_linq-gpu.lock /usr/bin/flock -x /tmp/mini_linq-timing.lock "$@"; }
for d in scala jython; do
  driver=$prof/doop-counters
  [ "$d" != jython ] || driver=/data/tmp-targets/capture/doop-plain
  locked env GPU_DATALOG_TRACE_KERNELS=1 GPU_DATALOG_TRACE_MEMORY=1 GPU_DATALOG_PACKED_RUN=0 \
    timeout 600 "$driver" "$data/$d" --gpu-only >"$CARGO_TARGET_DIR/trace-$d.log" 2>&1
  "$bin" census "$CARGO_TARGET_DIR/trace-$d.log" >"$out/hashdedup-$d.tsv"
done
locked "$bin" bench >"$out/hashdedup-bench.tsv"
for spec in 'bench 20000000 2 scattered' 'bench 20000000 4 scattered' 'bench 20000000 8 scattered' \
  'stream 20000000 8 scattered 20' 'stream 20000000 8 adjacent 20' 'stream 100000000 2 scattered 12' \
  'stream 100000000 2 adjacent 12' 'stream 8000000 16 scattered 25'; do
  locked env GPU_DATALOG_PACKED_RUN=0 "$bin" $spec
done >"$out/hashdedup-large.tsv"
for spec in '20000000 8 scattered 2' '20000000 8 scattered 4' '100000000 2 scattered 2' \
  '100000000 2 scattered 4' '100000000 2 scattered 8'; do
  locked env GPU_DATALOG_PACKED_RUN=0 "$bin" stream $spec
done >"$out/hashdedup-crossover.tsv"
locked "$bin" overflow
for d in scala jython; do "$bin" policy "$out/hashdedup-$d.tsv" "$out"/hashdedup-{bench,large,crossover}.tsv; done
for spec in '1000000 16' '8000000 16' '100000000 2'; do
  locked ncu --cache-control none --clock-control none --kernel-name-base function \
    -k hd_insert --launch-skip 1 --launch-count 1 \
    --metrics gpu__time_duration.sum,dram__bytes_read.sum,dram__bytes_write.sum,lts__t_sector_hit_rate.pct,smsp__warp_issue_stalled_long_scoreboard_per_warp_active.pct,sm__warps_active.avg.pct_of_peak_sustained_active \
    --page raw --csv --print-units base "$bin" bench $spec scattered
done
```

## Scala maintenance: price the committed side before choosing set difference

The `share/doop-gpu-select` fixpoint took 6.866 s in a fresh Nsight Systems trace. `share/profile-select-scala.log` attributes **2.201 s** to the seven maintenance steps below, **excluding setup**. The profile synchronizes at each boundary; the Nsight column is a separate run's **GPU kernel execution only**. Neither column should be added to the other. Rates are *logical input rows* per profile-step second; merge counts **both** source lengths, not just the smaller input.

| Step | Calls | Total rows | Mean rows/call | Profile ms | G rows/s | Nsys kernel ms; attribution |
|---|---:|---:|---:|---:|---:|---|
| Sort + unique | 3,083 | 3,190,120,307 | 1,034,745 | 840.96 | 3.79 | 662.18; OneSweep keys 439.04, pairs 38.97, adjacent-unique marks 39.19, scatter 56.13, lookback scan 24.68 |
| Subtract + stable select | 3,011 | 1,436,353,037 unique | 477,035 | 648.18 | 2.22 | 570.59; `absent` 483.06, `mark_flagged` 20.63, lookback scan 23.83, scatter 37.70 |
| Merge retained runs | 3,213 | 12,838,032,909 | 3,995,653 | 410.75 | 31.26 | 296.64; `device_partition_merge_path` 34.48, `device_merge_keys_agent` 262.16 |
| Other-order sort (already unique) | 1,121 | 797,403,578 | 711,332 | 150.55 | 5.30 | 108.52; OneSweep keys 48.77, pairs 37.13, final wide gather 2.98 |
| Permute | 2,104 | 981,131,489 | 466,317 | 28.42 | 34.52 | `permute` 10.92 across 1,827 launches (not every recorded step launches a kernel) |
| Concatenate bags | 3,418 | 3,190,120,307 | 933,330 | 102.20 | 31.22 | Stream-ordered D2D copies, not a kernel |
| Append excluding its merge steps | 3,365 | 1,478,702,595 | 439,436 | 20.24 | 73.06 | Adopts already-sorted buffers; assigning one full-row memcpy to each append would be wrong |

**How rows were attributed.** Use the interval between `gpu_datalog/setup_end` and `gpu_datalog/fixpoint_end` in `/data/tmp-targets/capture/nsys-select-maint.sqlite`. In launch order, match sort graphs to successive nonempty `sort/portion-*` in `select-maint-census.log`: a narrow radix sort or `gather_lane` + radix pairs for each wide lane; `mark_adjacent_unique` through `scatter_selected` closes an unknown-unique sort; `gather_sorted` closes a wide known-unique sort. Exclude internal rule sorts after `keys_and_rows`. This matches all 3,083 + 1,121 logical sorts. `absent` through `scatter_selected` and `device_partition_merge_path` + `device_merge_keys_agent` bound the subtraction and merge graphs. Across all operations `device_radix_sort_onesweep_keys` alone launched 13,392 times / 487.81 ms; median launch 6.43 µs. Medians for `absent`/merge-agent were 11.62/3.20 µs; their maxima were 3.28/9.75 ms.

**The subtract denominator matters.** Select's census reports 3,175,788,077 *capacity* slots, not that many unique keys. The pre-device-count `census-doop-scala.log` recorded 1,436,353,037 live unique rows for the same 3,011 subtractions; the raw input counts for every admission and fresh counts for every append match in chronological order across the two traces. At round 186 `varpointsto` had 29,138,567 raw, 9,730,816 unique and 5,928,628 fresh. `absent` took 2.507 ms (3.88 G **live** queries/s); even a sorted-input binary search is not modeled by the **0.79 G/s random independent queries** cap. Whole-run `absent` delivered 2.97 G live rows/s. A one-search-per-row reference is 1.82 s at 0.79 G/s or 0.098 s at the 14.7 G/s ascending-query cap; neither is an exact bound for variable run counts and key locality.

### Whole-step bounds and proposed changes

* **Sorting:** charge each actual OneSweep digit `16 * rows` bytes (u64 keys) or `24 * rows` (u64 + u32 row index), matched to its logical input rather than treating every launch equally. Canonical: 26.091 G row-digits, **435.35 GB / 890.6 GB/s = 489 ms**, vs 841 ms wall (1.72×). Other orders: 7.109 G row-digits, **131.22 GB = 147 ms**, vs 151 ms (1.02×); some small or repeated work is cache resident. These are DRAM-equivalent references, not measured DRAM bytes or absolute lower bounds. They exclude small single-tile sort traffic, histograms, copies, gathering and compaction. [INFERENCE] Prioritize eliminating duplicate inputs **before** canonical sort (a correct per-head early-union operator), or packing bounded symbol codes more tightly and sorting only the resulting significant bits. Merely shortening a sort's bit window without changing a gapped packing is incorrect. Additional other-order speedup must principally remove row-digits, not tune the bandwidth ceiling.
* **Merging:** exact mixed-width source bytes 114.99 GB; read both sources once and write their union once = 229.98 GB, **258 ms** at the cap. The two RCCL merge kernels took 297 ms (775 GB/s, 87% of cap; 1.15×), vs 411 ms profiled step (1.59×). [INFERENCE] Keep RCCL for large merges. `append_sorted` currently cascades binary merges while the ratio test holds, rewriting newly merged rows. Coalesce the participating suffix in a single multi-run merge when the saved intermediate bytes justify it; preserve immutable published snapshots. First measure saved **bytes**, not just the number of merges.
* **Permute / concatenate / append:** one read+write of the recorded widths costs 24.2 / 60.7 ms vs 28.4 / 102.2 ms (1.17× / 1.68×). `concatenate_ranges` copies batches to a temporary, then `stable_keys` copies again into the first sort buffer. [INFERENCE] Concatenate directly into that owned first sort buffer, applying the permutation if required; publish an existing sorted buffer on append rather than copying its rows. Append already does the latter (`set_segment`); no streaming bound applies to its 20 ms of bookkeeping.

### Stable difference on a real committed relation

Scala round 186: **298,465,609 real committed** `(heap,var)` rows from hole 132 input 2, and **5,928,628 real fresh** rows from hole 136 round 187 input 0. The 3,802,188 old keys re-proposed by the rule batch were not captured; substitute equally spaced ranks in the committed set and merge-sort them with the fresh keys. Thus the set sizes, old/fresh data, and hit count are real; the old-hit distribution is synthetic. The ratio-4 append history gives retained sizes 251,565,383 / 41,215,005 / 5,685,221. The last run is the actual hole 136 round 186 delta; the two older partitions use proportional ranks of the remaining old keys (original run boundaries not recorded). The one-run variant uses the exact captured global order without partitioning.

`maint` materializes sorted unique A\\B. Binary does one lower bound per surviving query per run, then RCCL stable selection. Merge-path partitions A+B into balanced 2,048-row diagonals per run, stages coalesced keys in shared memory, handles equalities across tile boundaries, combines hit flags, and selects once. Bounded binary partitions each run by successive 128-row candidate windows before searching smaller intervals. RCCL is its native sorted `SetDifference`, chained once per run; the timed calls use preflighted workspace and actual previously observed intermediate counts, excluding readbacks/allocations (favors RCCL). Median of nine warmed CUDA events. CPU streaming difference and **every output element in order** verified, count 5,928,628, checksum `e637a9b7bc92c3d4`.

| Method | 1 run: mark / full ms | 3 runs: mark / full ms |
|---|---:|---:|
| Binary search | **2.514 / 2.861** | **2.535 / 2.868** |
| Balanced merge-path | 3.431 / 3.786 | 3.732 / 4.071 |
| Bounded binary (includes partition cost) | 2.778 / 3.129 | 3.079 / 3.432 |
| RCCL sorted set difference | — / 3.186 | — / 3.496 |

**Do not replace `absent` unconditionally.** The one-run read-both/write-fresh streaming reference is **2.822 ms**, so even a perfect such implementation rivals, rather than greatly improves on, binary's 2.861 ms. For three runs, reading candidates three times plus old rows and one fresh write costs 2.997 ms before flags or compaction. More decisively, across `varpointsto`'s 404 nonempty Scala subtractions, reading the whole committed side each time would read **130.219 G rows / 1.042 TB** (from cumulative admissions in `share/trace-final-scala.log`): **1.170 s** at peak DRAM bandwidth *just for those reads*, vs **0.437 s** profiled for that relation's full subtraction.

NCU, three-run case, warm binary mark vs **only the largest run's** merge-path marking kernel (no cuts, smaller runs or select in those intervals):

| Kernel | µs | DRAM read / write MB | Read GB/s | Load sectors | Thread instructions | L2 hit | Long scoreboard | Occupancy |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `maint_binary` (all 3) | 2,510 | 1,120 / 40 | 446 | 91.14 M | 6.48 G | 19.64% | 95.46% | 85.54% |
| `maint_merge` (largest only) | 2,498 | 2,091 / 78 | 837 | 73.64 M | 12.59 G | 11.86% | 77.58% | 81.93% |

Binary is latency-limited, yet it reads **less**: merging even the first run is already as expensive while reading at 94% of the DRAM cap. A concrete subtraction change worth testing instead [INFERENCE]: keep binary for sparse candidates, feed the resident **live unique count** into a stable select that avoids scanning unused capacity (do not reintroduce a host barrier). In this invocation select's post-`absent` kernels took **0.919 ms** on 29.14 M slots; manual5's live-length graph took **0.300 ms** on 9.73 M. This is a comparison of separate traces, not an end-to-end speedup claim.

```sh
cd /data/mini_linq/experiments/gpu; source /dev/shm/gpu-env.sh
export CARGO_TARGET_DIR=/data/tmp-targets/capture CUDA_OXIDE_PTX_DIR=/data/tmp-targets/capture/oxide-ir
flock -s /tmp/mini_linq-timing.lock timeout 1200 cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release -p join-lab --bin maint
cap=/data/design-notes/join-lab/captures/scala; bin=$CARGO_TARGET_DIR/release/maint
flock /tmp/mini_linq-gpu.lock flock -x /tmp/mini_linq-timing.lock "$bin" "$cap"  # 1 and 3 runs
metrics=gpu__time_duration.sum,smsp__thread_inst_executed.sum,l1tex__t_sectors_pipe_lsu_mem_global_op_ld.sum,dram__bytes_read.sum,dram__bytes_write.sum,lts__t_sector_hit_rate.pct,smsp__warp_issue_stalled_long_scoreboard_per_warp_active.pct,sm__warps_active.avg.pct_of_peak_sustained_active
for case in 'maint_binary 1 binary' 'maint_merge 3 merge-large'; do
  read -r kernel skip name <<< "$case"
  flock /tmp/mini_linq-gpu.lock flock -x /tmp/mini_linq-timing.lock ncu \
    --cache-control none --clock-control none --kernel-name-base function -k "$kernel" \
    --launch-skip "$skip" --launch-count 1 --metrics "$metrics" --page raw --csv --print-units base \
    "$bin" "$cap" 3 > "$CARGO_TARGET_DIR/maint-3-$name.csv"
done
P=/data/design-notes/doop-memory/profile-xlarge; D=/data/doop-bench/corpus-20261110/raw/scala
flock /tmp/mini_linq-gpu.lock flock -x /tmp/mini_linq-timing.lock env GPU_DATALOG_PHASES=1 \
  /opt/nvidia/hpc_sdk/Linux_x86_64/25.9/compilers/bin/nsys profile -t cuda,nvtx --sample=none --cpuctxsw=none \
  --export=sqlite --force-overwrite=true -o "$CARGO_TARGET_DIR/nsys-select-maint" "$P/share/doop-gpu-select" "$D" --gpu-only
flock /tmp/mini_linq-gpu.lock flock -x /tmp/mini_linq-timing.lock env GPU_DATALOG_PHASES=1 \
  GPU_DATALOG_TRACE_MEMORY=1 GPU_DATALOG_TRACE_ROUNDS=1 "$P/share/doop-gpu-select" "$D" --gpu-only > "$CARGO_TARGET_DIR/select-maint-census.log" 2>&1
```

## Separating the two assignment producers: jython holes 118/119

Both are versions of `varpointsto(heap,to) :- assign(from,to), varpointsto(heap,from)` (`features/datalog/doop/shared/src/mini_engine.rs`, not the cast rule). `doop-gpu-select2` maps #118 to `c697c4171aee9b27`, scanning **delta Assign(from,to)** into `VPT[from] → heaps`; #119 maps to `0ccd3dd63af228c1`, scanning **delta VPT(heap,from)** into `Assign[from] → targets`. The bound output column in the *driving row* is `to` for #118 but **heap** for #119. The latter does not have a `to` to group by before the probe. Union heap ranges per `to` on #118; use the symmetric union of target ranges per `heap` on #119. Alternatively reverse #119 into a `to`-major scan of the full assign index and a `from`-major index on delta VPT. All three group orientations compute the identical per-version unique set by construction.

**Paired real snapshots.** Capture `/data/design-notes/join-lab/assign-captures/` contains both versions in the *same* jython rounds, including the peak output round of each. A CPU set oracle groups independently, unions **all** live ranges, decodes original symbol IDs, and tests the resulting `(heap,to)` against the full `VPT[to] → heaps` index from #118's capture. This tests membership at the actual admission boundary; “fresh” is not exclusive ownership of a new fact.

| Round / hole | Delta / full index rows | Raw witness rows | Unique after per-head-column union | Locally old | Locally fresh | Duplicate factor |
|---|---:|---:|---:|---:|---:|---:|
| 114 / 118 | 1,628,455 / 224,541,907 | 2,854,157,954 | 10,587,167 | 4,259,027 | 6,328,140 | 269.6× |
| 114 / 119 | 7,333,616 / 7,394,065 | 153,797,466 | 8,298,756 | 5,833,498 | 2,465,258 | 18.5× |
| 167 / 118 | 395,468 / 398,389,294 | 541,288,772 | 49,693,386 | 47,926,302 | 1,767,084 | 10.9× |
| 167 / 119 | 6,432,951 / 15,155,210 | 1,588,982,996 | 43,597,123 | 42,612,695 | 984,428 | 36.4× |

Each input is a set. Within a version, repeated output therefore comes from *distinct* `from` witnesses; the union removes all of it. Across versions, round 114 still shares 1,549,984 distinct keys (617,962 old), leaving **17,335,939** joint unique and **7,861,376** joint fresh. Round 167 shares 422,056 (294,537 old), leaving **92,868,453** joint unique and **2,623,993** joint fresh. Relative to the all-version census above, other versions add a further 2,732,244 / 993,779 distinct keys and 2,275,033 / 738,167 fresh keys exclusively, respectively. Group union does not replace either cross-version dedup or subtraction.

**Skew.** `k` counts distinct nonempty child ranges per group; `children` counts their members with multiplicity. The quantiles below are across nonempty groups, not weighted by children.

| Round / hole / group axis | Groups | k: p50 / p99 / max | children: p50 / p99 / max | Raw children in groups with k>256 |
|---|---:|---:|---:|---:|
| 114 / 118 / `to` | 8,690 | 1 / 1,210 / 5,518 | 1 / 4,449,827 / 4,457,712 | 98.35% |
| 167 / 118 / `to` | 10,215 | 10 / 24 / 8,855 | 49,700 / 119,328 / 820,160 | 3.92% |
| 114 / 119 / `heap` | 12,288 | 14 / 1,581 / 4,373 | 18 / 418,446 / 1,117,614 | 97.74% |
| 167 / 119 / `heap` | 13,361 | 35 / 1,563 / 3,953 | 47 / 414,233 / 1,324,648 | 99.92% |

Grouping #119 literally by `to` instead yields 40,887 / 43,023 targets in rounds 114 / 167; `k` p50/p99/max = 1/3,375/3,376 and 2/3,213/3,213; children = 59/92,820/146,995 and 99/252,519/253,208. The implemented heap-major orientation doesn't pay to build the reverse delta index.

**Skew-aware GPU union.** Input is the captured delta plus a resident CSR (`from → child`) with order-preserving dense child IDs and an inverse dictionary. Canonicalizing captured runs into that index and H2D upload are fixture setup for *every* method; the raw control uses precisely the same resident index. No CPU grouping, output keys, group schedule or mixture proportions are uploaded. Buffer capacities are sized from the lab oracle. The GPU counts live references, scans per group and scatters them. A warp copies singleton ranges directly; a 256-thread CTA ORs up to 256 ranges in shared memory (835–4,121 words, depending on the capture). For split groups it writes partial bitmaps, then a final warp per group ORs them and compacts set bits. All methods produce the canonical packed `(heap,to)`; comparator is the **entire sorted vector**, not just a count. Three variants:

- **CSR:** shared atomic OR for each input child; no secondary resident index.
- **Hybrid:** replace any resident CSR range of at least `ceil(child_domain/32)` elements by the same number of contiguous bitmap words or fewer. Small ranges remain CSR. Build and maintenance of this optional secondary index are not free (below).
- **Reuse:** sort live `(group,from)` pairs, intern each complete sorted `from` list with a fingerprint plus **exact** equality, compute partial unions for representatives only, then enumerate one output per original group. It trades a full-delta sort and list comparison for fewer unions. No dense resident index. In table order (114/118, 114/119, 167/118, 167/119), there are only **248 / 701 / 62 / 132** different lists; the most reused list has **5,541 / 2,763 / 8,960 / 4,203** groups. Paying once per distinct list would visit **17.04 / 49.48 / 1.81 / 18.01 M** child IDs. Those counts include singletons once; the implementation reads singleton ranges directly per group. Reuse shrinks the actual multi-range CTA counts from **7,225 / 18,473 / 10,443 / 26,144** to **174 / 1,593 / 102 / 324**. This does not imply the same speedup: an extremely long representative can become the tail.

**Controls and boundaries.** Raw uses a delta range-length scan and warp-window range expansion followed by the runtime's actual 64-bit RCCL radix sort/unique. If there are more than 267,386,880 candidates, it streams chunks of that size, subtracts the preceding chunks' distinct set and merges the chunks' sorted results. This makes the 2.85 B occurrence control feasible without silently benchmarking a sample. Raw's packing time alone is 38.93 / 2.93 / 6.10 / 27.83 ms, *included* in the sort/union comparison. The grouped variants write exactly the local unique column above, unordered; sorted publication adds an actual 64-bit known-unique sort on the emitted keys. Neither side here subtracts the committed relation or merges the other rule versions. All four raw results as well as all 12 grouped unions passed full CPU key equality.

CUDA-event milliseconds, RTX 6000 Ada. Group columns include exact GPU scheduling/compaction; “sort U” is the selected method's separately measured known-unique sort. Raw includes root scan, emit, sort, dedup and interchunk union. Speedup compares **sorted result to sorted result**, with a resident index on both sides.

| Round / hole | Raw to sorted | CSR union | Hybrid union | Reuse union | Best resident + sort U | Sorted speedup | Hybrid index build / MB |
|---|---:|---:|---:|---:|---:|---:|---:|
| 114 / 118 | 677.44 | 6.285 | **1.458** | 1.813 | 1.458 + 1.713 | **213.6×** | 4.771 / 182 |
| 114 / 119 | 38.00 | **2.093** | 2.194 | 2.849 | 2.093 + 2.116 | **9.0×** | 0.038 / 12 |
| 167 / 118 | 137.16 | 2.046 | **1.516** | 2.820 | 1.516 + 9.569 | **12.4×** | 8.499 / 275 |
| 167 / 119 | 387.72 | 6.804 | 6.861 | **3.022** | 3.022 + 8.968 | **32.3×** | 0.064 / 19 |

This is evidence for replacing raw output, **not** for rebuilding the VPT bitmap each time. From a cold CSR, choose reuse on 114/118, CSR on 167/118; if the bitmap is already current, hybrid wins both. For 119/167 interned lists save 3.78 ms vs CSR despite paying 1.43 ms to schedule. For 119/114 plain CSR wins: scheduling reuse alone costs 1.82 ms vs 0.36 ms for CSR. Publication is already 50–86% of the selected sorted pipeline. The two versions together reduce intermediate writes by 23.91 / 16.30 GB in rounds 114 / 167 *before* counting radix passes. These are two real peak rounds, not a factor applied to all rounds or to end-to-end DOOP wall time.

**Bounds.** At this card's measured 890.6 GB/s, reading each referenced 32-bit child **occurrence** once and writing each unique `u64` once gives `(4C+8U)/BW` = **12.91 / 0.77 / 2.88 / 7.53 ms** (table order). The CSR list for a repeated `from` need not come from DRAM again; hybrid also replaces rows by bits, and reuse need not visit the same occurrence. Thus “beating” this logical-traffic model is not beating a hardware bound. Count the distinct resident child IDs **by physical range slot** needed by the entire version: **6,855,440 / 2,934,002 / 1,805,436 / 4,123,412**. Reading just these once plus the actual 8-byte delta rows and writing each unique key once gives an optimistic cold-CSR data movement target **0.141 / 0.154 / 0.458 / 0.468 ms** (no indexing, ordering or instructions). The selected unordered unions are **10.4× / 13.6× / 3.3× / 6.5×** this target; hybrid uses a different resident encoding. Warp rereads across groups, divergent range lengths, bitmap partials and final ordering are real costs absent from the target.

**Counter evidence, 114/118.** Hybrid's `as_tile` takes 1.062 ms; it issues 121.4 M global load sectors (3.89 GB at L1) but reads only 13.8 MB from DRAM, with **99.55% L2 hit**, 86.0% active warps, 54.9% long-scoreboard stall and 10.09 B executed lane instructions. Finish takes 0.277 ms. Cache-local gather/OR work, not DRAM bandwidth, explains the gap. Interning drops tile load sectors to 4.31 M and lane instructions to 206 M, yet tile still takes 1.024 ms: only **174** useful union CTAs, 30.6% average active warps vs 86.0%, with unequal range weights. Finish costs 0.321 ms, and warm non-profiled scheduling is 0.451 vs 0.140 ms. Counting ranges rather than actual elements splits heavily skewed representatives poorly. Primary artifacts: `/data/design-notes/join-lab/assign-timings.log`, `/data/tmp-targets/duplab/assign-ncu-*.csv`.

```bash
cd /data/mini_linq/experiments/gpu; source /dev/shm/gpu-env.sh
export CARGO_TARGET_DIR=/data/tmp-targets/duplab
T=$CARGO_TARGET_DIR/release; C=/data/design-notes/join-lab/assign-captures
build() { /usr/bin/flock -s /tmp/mini_linq-timing.lock timeout 1200 cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release "$@"; }
gpu() { /usr/bin/flock /tmp/mini_linq-gpu.lock /usr/bin/flock -x /tmp/mini_linq-timing.lock bash -ec 'test -z "$(nvidia-smi --query-compute-apps=pid --format=csv,noheader)"; "$@"' bash "$@"; }
build -p doop-gpu --features cq-gpu-runtime/nvtx,capture-joins
# Use a fresh JOIN_CAPTURE_DIR to create rather than replay the historical snapshots.
JOIN_CAPTURE_DIR="$C" JOIN_CAPTURE_HOLES=118,119 JOIN_CAPTURE_ROUNDS=114,167 JOIN_CAPTURE_MAX=2 \
  gpu "$T/doop-gpu" /data/doop-bench/corpus-20261110/raw/jython --gpu-only
build -p join-lab --bin assign
for r in 114 167; do GPU_DATALOG_PACKED_RUN=0 gpu "$T/assign" "$C/varpointsto@0__h118/$r" "$C/varpointsto@1__h119/$r" all; done
M=gpu__time_duration.sum,smsp__thread_inst_executed.sum,l1tex__t_sectors_pipe_lsu_mem_global_op_ld.sum,dram__bytes_read.sum,dram__bytes_write.sum,lts__t_sector_hit_rate.pct,smsp__warp_issue_stalled_long_scoreboard_per_warp_active.pct,sm__warps_active.avg.pct_of_peak_sustained_active
for spec in 114:hybrid118 114:reuse118 167:reuse119; do r=${spec%:*}; v=${spec#*:}
  gpu ncu --page raw --csv --kernel-name-base function -k 'regex:as_(tile|finish)$' --launch-skip 2 --launch-count 2 --cache-control none --clock-control none --metrics "$M" --log-file "$CARGO_TARGET_DIR/assign-ncu-$v.csv" "$T/assign" "$C/varpointsto@0__h118/$r" "$C/varpointsto@1__h119/$r" "$v"
done
```

**Counter evidence, 167/119.** Reuse's 324 partial-union tasks complete in 0.506 ms (17.8 MB DRAM reads, 36.0% active warps). Enumerating 43.6 M distinct keys is more expensive: `as_finish` takes 1.189 ms, executes 4.65 B lane instructions, writes 315 MB to DRAM, and stalls on the long scoreboard 74.1% of active-warp cycles despite a 99.51% L2 hit rate. Here the remaining costs are the 1.43 ms measured scheduler, explicit key enumeration, and especially the subsequent 8.97 ms known-unique sort—not the CSR child reads.

## Scala maintenance on `doop-gpu-select2`: index committed membership, not candidate capacity

The baseline here is **select2**, RootSelect + capacity hint + trim, not the archived device-count experiment above. `share/run.sh … maint2 profile scala`: fixpoint **6.639534 s**, peak pool **20.826 GB**, counts digest **75b7003b**. Independent Nsight Systems run: **6.898946 s** fixpoint, **5.685067 s** summed GPU kernels. The current subtract operates on **1.436 B live unique rows**, not the earlier 3.176 B allocated rows.

| Fixpoint maintenance | Rows (B) | Profile ms | G rows/s | Nsight kernel ms | Caps target ms (G rows/s) | Profile minus target ms |
|---|---:|---:|---:|---:|---:|---:|
| Canonical sort + unique | 3.1901 | 867.35 | 3.68 | 662.65 | 488.83 (6.53) | 378.52 |
| Subtract committed | 1.4364 | 593.47 | 2.42 | 523.68 | 97.71 (14.70) | **495.75** |
| Merge (both input sides) | 12.8380 | 403.25 | 31.84 | 295.95 | 258.24 (49.71) | 145.01 |
| Other order: known-unique sort | 0.7974 | 150.39 | 5.30 | 109.31 | 147.34 (5.41) | 3.05 |
| Order permutation | 0.9811 | 26.15 | 37.52 | 9.96 (`permute`) | 24.19 (40.56) | 1.95 |

Profile times include host/launch/barrier costs; kernels are counted only between `gpu_datalog/setup_end` and `fixpoint_end`. Pair each logical sort with its actual radix-kernel sequence (exclude join-generated sorts): 435.354 / 131.220 GB of global OneSweep pass traffic for canonical / alternate orders, respectively (16 B per row-digit for keys; 24 for key/index). Compare this to measured 890.6 GB/s; these are *cold DRAM models*, not hard lower bounds: hot cache can beat them. Merge and permute use twice the census source bytes (114.992 / 10.773 GB). Subtract uses the measured ascending, single-search 14.7 G/s reference—optimistic for multiple runs and excluding compaction. The winning target is still subtract on **kernel time alone** (426 ms gap vs 174 ms for canonical sort). `absent` itself costs **478.77 ms**; RCCL selection accounts for the other **44.91 ms**. Other profiled accounting: concatenate 100.81 ms / 3.190 B rows; append 19.38 ms / 1.479 B. None of these differences is a promised wall-time saving.

**Prototype.** `maint {scala-capture-dir} 3 bitmap`: keep the canonical committed pair relation in a monotone sparse bitmap *as well as* its sorted runs. Assign heap IDs dense ranks on first encounter; the rank map persists across batches. For each `(var, heap_rank >> 11)` allocate a 2,048-bit leaf only when populated. Each sorted unique candidate atomically tests-and-sets its own bit; keep newly set candidates via stable RCCL flagged select. This includes maintaining the index for the next invocation. Separating heap/page assignment into launches avoids spinning on an assigning CTA. No host hash, no precomputed answer IDs, no scan of the committed runs per invocation. Query replay uses the same real 298,465,609 committed and 5,928,628 fresh rows as above; 3,802,188 synthetic *known hits* uniformly spaced by committed rank bring the query to the measured 9,730,816-unique size. Both implementations match the independently checked CPU difference **element-by-element** (`e637a9b7bc92c3d4`); a second admission of the same query returns no fresh flags. Before each timed trial restore the exact pre-invocation state device-to-device, outside the events. Thus every trial includes actual insertion and page allocation, not just existing-key tests.

| Round 186 trial (median 9, CUDA events) | Mark/update ms | With stable selection ms | Persistent reservation |
|---|---:|---:|---:|
| Three-run binary probe, replay control | 2.535 | 2.868 | No side index |
| Exact sparse bitmap | **0.836** | **1.163** | 1,358,954,508 B |
| Flat `u64` linear-probing hash, 2²⁹ slots | 4.177 | 4.528 | 4,294,967,296 B |

The *actual* select2 round in Nsight has exactly one 9,730,816-row subtract: **2.291 ms absent + 0.296 ms selection = 2.587 ms**. [INFERENCE] The bitmap saves about **1.42 ms** for that round against real code; matched replay controls save **1.705 ms (2.47×)**. One-time build from the real committed rows: bitmap **11.705 ms**, hash **110.679 ms**, excluding zero-allocation. The bitmap goes from 38,213 to 38,397 heaps and 1,744,379 to 1,759,867 live pages in the measured run. Its 22-bit code bound, 65,536 heap ranks and 3,145,728-page reserve are **benchmark capacities**, not limits proved for the whole dataset.

**Why bitmap, not hash?** NCU on candidate launch 2, 9.731 M rows: bitmap *test/set kernel alone* **0.575 ms**, 171.8 MB DRAM read / 80.2 MB write, 15.20 M warp instructions, 87.0% active warps, 85.3% long-scoreboard stalls. Hash insertion **4.394 ms**, 504.7 / 453.1 MB, 71.66 M warp instructions, 77.5% active warps, 96.3% long-scoreboard stalls. The bitmap touches fewer bytes with far fewer dependent probes; separately timed mark/update above also pays heap and page discovery.

**Expected scope, not an end-to-end result.** Select2 spends **390.164 profiled ms** on `varpointsto` subtract alone (1,134,392,471 unique rows, 404 calls). Match each main-SCC fresh-row increment from the memory census to the admission that permutes exactly that many rows; in-order `absent` through `scatter_selected` identifies the matching Nsight kernels. This accounts for **403 calls / 1,134,392,470 rows**. Restrict to calls with ≥1 M unique rows: **236 calls / 1,113,734,257 rows**, **363.59 ms** measured GPU time. [INFERENCE] Applying round 186's 1.163264 ms per 9,730,816 rows to just those calls gives **133.14 ms**, or **230.45 ms gross GPU saving** (3.5% of profiled fixpoint). Initial build/zero, capacity growth, small-round updates, actual hit/locality changes, host dispatch, and committed-state rollback are not in that extrapolation. No savings are credited to other heads or to sort/merge. This is a reason to trial the side index, not a measured 230 ms engine improvement.

**Proposed integration, not applied:** preserve the read-only contract of `datalog-runtime/src/radix_run.rs::fresh`/`uncommitted`. Give `datalog/src/print.rs::admit_publish` a separate canonical-relation admission state for monotone binary set heads: use the runtime packing to extract the two code fields, maintain it across admissions independently of LSM merges, and use test/set only inside the publication transaction. Do not mutate a published snapshot from `fresh(&self)`. Never leave the published relation and index out of sync on abort: invalidate/rebuild the shadow index or fall back to the original subtract until rebuilt. Before enabling, budget actual per-domain directory/page capacities, grow or fall back exactly on exhaustion, and charge build/zero/growth against measured savings. Keep original sorted canonical and alternate orders for joins. No production runtime edit for this proposal.

Reproduction (from `experiments/gpu`; exclusive GPU+timing locks and empty GPU process list for GPU commands):

```bash
source /dev/shm/gpu-env.sh
P=/data/design-notes/doop-memory/profile-xlarge; T=/data/tmp-targets/capture
B=$P/share/doop-gpu-select2; D=/data/doop-bench/corpus-20261110/raw/scala
C=/data/design-notes/join-lab/captures/scala
gpu() { /usr/bin/flock /tmp/mini_linq-gpu.lock /usr/bin/flock -x /tmp/mini_linq-timing.lock "$@"; }
/bin/bash "$P/share/run.sh" "$B" maint2 profile scala
gpu env GPU_DATALOG_PHASES=1 /opt/nvidia/hpc_sdk/Linux_x86_64/25.9/compilers/bin/nsys profile -t cuda,nvtx --sample=none --cpuctxsw=none --export=sqlite --force-overwrite=true -o "$T/nsys-select2-maint" "$B" "$D" --gpu-only
gpu env GPU_DATALOG_PHASES=1 GPU_DATALOG_TRACE_MEMORY=1 GPU_DATALOG_TRACE_ROUNDS=1 "$B" "$D" --gpu-only >"$T/select2-maint-census.log" 2>&1
export CARGO_TARGET_DIR=$T CUDA_OXIDE_PTX_DIR=$T/oxide-ir
/usr/bin/flock -s /tmp/mini_linq-timing.lock timeout 1200 cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release -p join-lab --bin maint
M=gpu__time_duration.sum,dram__bytes_read.sum,dram__bytes_write.sum,sm__warps_active.avg.pct_of_peak_sustained_active,smsp__inst_executed.sum,smsp__warp_issue_stalled_long_scoreboard_per_warp_active.pct
for method in bitmap hash; do
  gpu "$T/release/maint" "$C" 3 "$method"
  k=maint_bitmap_test_set; test "$method" = hash && k=maint_hash_insert
  gpu ncu --csv --kernel-name "regex:$k" --launch-skip 2 --launch-count 1 --metrics "$M" -f --log-file "$T/sideindex-$method.csv" "$T/release/maint" "$C" 3 "$method"
done
```

## `manual-joins`: assignment union directly on the engine's runs

`experiments/gpu/doop/src/manual_assign.rs` replaces both `VPT_Assign` versions. `@0` reads `delta assign` and `varpointsto[1, 0]`; `@1` reads `delta varpointsto` and `assign[0, 1]`. These are assertions on the printer contract. Unlike the capture experiment, the engine provides **neither** a merged CSR **nor** dense per-role IDs. The implementation uses the same `decode_key`, `encode_prefix`, `run` and `interval` used by the generated plans. It decodes delta edges into `(group,from)`, deduplicates `from`, probes each referenced `from` once per live run, and reads only hit child ranges. Sorted neighboring child IDs with the same upper 27 bits become `(child >> 5, membership mask)` fragments. A source may have several fragments for the same word because it spans windows or runs; this is valid because all consumers OR the masks. Only hit groups and referenced raw word IDs receive compact IDs, rebuilt per invocation without any EDB mapping or cache invalidation assumption.

One warp per delta edge contributes its source fragments to the destination group: different warps/CTAs can update the same group's words safely. A 256 MiB cap stripes the group × word bitmap over disjoint groups when necessary. Only the emitter, and in a striped case only bitmap fill + emitter, reruns if the arena hint was too small. A canonical `u64 (heap,to)` is emitted exactly once per set bit, so the output support equals the bag plan's even if many `from` witnesses or runs support the same fact. Other versions' duplicates and old facts still go through the usual admission path.

**How the real code distribution changes the work.** Applying the 32-child-window encoding to the actual captured run order gives the following. “ORs” counts fragment visits after multiplying by delta edges; it is not a DRAM-transaction count. Published keys are the four exact set sizes in the previous assignment section.

| Round / version | Physical child slots read once | Active source fragments | Fragment ORs / raw occurrences | Group × live-word bitmap before striping |
|---|---:|---:|---:|---:|
| 114 / `@0` | 6.86 M | 2.26 M | 941 M / 2,854 M | 8,690 × 3,104 = 108 MB |
| 114 / `@1` | 2.93 M | 2.73 M | 140 M / 154 M | 12,288 × 12,722 = 625 MB |
| 167 / `@0` | 1.81 M | 0.57 M | 166 M / 541 M | 10,215 × 3,241 = 132 MB |
| 167 / `@1` | 4.12 M | 3.73 M | 1,423 M / 1,589 M | 13,361 × 14,696 = 785 MB |

**Production NCU, not a replay of the lab.** Profile the first jython invocation of each version with at least 1,048,576 delta edges. Each NCU driver completed with the unchanged `66b903ac` digest. Values are for the *first* stripe/attempt, with kernel replay and no clock or cache control; do not add them to host fixpoint times.

| Version | Range probe | Make fragments | Atomic word union | Publish | Union L2 hit / active warps / long-scoreboard |
|---|---:|---:|---:|---:|---:|
| `@0` | 0.040 ms | 0.335 ms | **6.661 ms** | 0.206 ms | 99.76% / 96.02% / 62.82% |
| `@1` | 0.020 ms | 0.035 ms | 0.211 ms | 0.220 ms | 81.81% / 76.47% / 86.86% |

For `@0` the union issues **357.4 M** global-load sectors (11.44 GB at L1), executes **9.05 B** lane instructions and reads only **29.4 MB** of DRAM. Direct bitmap OR across the many repeated lists is less work than emitting and sorting witnesses, but it is not the lab's representative-list reuse. DRAM reads alone are not the bound on this kernel. With `@1` the publish pass reads **138.1 MB** of DRAM to enumerate a sparse result; scanning/clearing a mostly zero bitmap can cost as much as its union. Further optimization should reduce repeated word contributions or skip empty bitmap regions rather than tuning the binary searches.

**Reproduction.** The control and treatment snapshots are `/data/tmp-targets/duplab/{engine-src,engine-treatment}`. The control keeps `manual-joins` **on**: the two `VPT_Assign` hooks alone are absent, and the existing `VPT_LoadArray@3` replacement, plans, generated printer, runtime, data and build options are unchanged. Each build uses its own target **and** oxide PTX directory. The final source binary SHA-256 is `7971c08d6defd7534e8e436ce208507ab6689e07167bd830cd4482b96b77833e`; control `ed8fe7ca38c70508be494633fd5fcc27cf1e797f8d8bf2f5f96ef046dcc2b724`. The NCU/profile binary `b2ff458506f97be37602b16af2b78919db38470eed02b21c973f436cce99143d` precedes a readability-only source pass.

```bash
source /dev/shm/gpu-env.sh
P=/data/tmp-targets/duplab; CTRL=/data/tmp-targets/duplab-control
for version in control treatment; do
  if test "$version" = control; then src=$P/engine-src; t=$CTRL; else src=$P/engine-treatment; t=$P; fi
  (cd "$src/experiments/gpu"; export CARGO_TARGET_DIR=$t CUDA_OXIDE_PTX_DIR=$t/oxide-ir
   /usr/bin/flock -s /tmp/mini_linq-timing.lock timeout 1200 cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release -p doop-gpu --features manual-joins,cq-gpu-runtime/nvtx)
done
H=/data/design-notes/doop-memory/profile-xlarge/share
/bin/bash "$H/run.sh" "$P/release/doop-gpu" manual-assign-exact exact xalan
/bin/bash "$H/run.sh" "$P/release/doop-gpu" manual-assign-time time scala soot kotlin jython
for mode in time profile; do
  /bin/bash "$H/run.sh" "$CTRL/release/doop-gpu" "assign-control-$mode" "$mode" jython scala
  /bin/bash "$H/run.sh" "$P/release/doop-gpu" "assign-treatment-$mode" "$mode" jython scala
done
python3 "$H/phases.py" "$H"/assign-{control,treatment}-profile-{jython,scala}.log
M=gpu__time_duration.sum,dram__bytes_read.sum,dram__bytes_write.sum,l1tex__t_sectors_pipe_lsu_mem_global_op_ld.sum,lts__t_sector_hit_rate.pct,sm__warps_active.avg.pct_of_peak_sustained_active,smsp__warp_issue_stalled_long_scoreboard_per_warp_active.pct,smsp__thread_inst_executed.sum
for k in edges points; do
  /usr/bin/flock /tmp/mini_linq-gpu.lock /usr/bin/flock -x /tmp/mini_linq-timing.lock ncu --nvtx --nvtx-include "assign_delta_${k}/assign_ge1m/" --kernel-name-base function --kernel-name 'regex:ma_(ranges|fragments|union|publish)' --launch-count 4 --cache-control none --clock-control none --page raw --csv --metrics "$M" --log-file "$P/assign-engine-$k-ncu.csv" "$P/release/doop-gpu" /data/doop-bench/corpus-20261110/raw/jython --gpu-only
done
```

**Whole-fixpoint attribution (seconds, measured by `share/run.sh profile`).** Peak is decimal GB of live device pool, not process RSS. Logs are `share/assign-os-{c0,e1}-profile-{jython,scala}.log`; the harness checked for foreign compute processes inside the GPU/timing locks.

| Dataset / implementation | Fixpoint | All joins | Admission sort/unique | Subtract | Merge | Orders | Other maintenance | Peak GB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| jython / generated assignment | 17.612 | 5.839 | 10.200 | .479 | .289 | .194 | .592 | 38.79 |
| jython / grouped assignment | **7.260** | 5.357 | **.998** | .385 | .246 | .141 | .116 | **18.00** |
| scala / generated assignment | **5.405** | **3.196** | .867 | .594 | .417 | .168 | 21.06 |
| scala / grouped assignment | 5.829 | 3.939 | **.565** | .606 | .417 | .165 | 20.82 |

On jython the two manual versions publish **662,688,466 + 1,150,612,831 = 1,813,301,297** keys summed across version-rounds, down from **41,981,269,037** generated occurrences, with the same final relation-count digest. The entire `varpointsto` admission input falls **43,372,222,635 → 3,204,254,895** (376 → 255 sort portions; 9.903 → .782 s). This is a producer optimization, not a faster radix sort. The NCU invocations above are round **109** for `@0` (1,105,308 roots, 9,529,544 candidates) and **73** for `@1` (1,273,161 roots, 833,756 candidates); the full run's nonempty invocations read only 337,260,103 and 314,967,780 physical child slots, respectively.

On scala the same hooks eliminate **1.112 B** admission inputs (2,774,341,980 → 1,662,273,006), but the `varpointsto` sort savings are just **.296 s** (.608 → .311); all joins cost **.743 s more**. The maintenance savings do not cover the grouped representation. Keep the manual result workload-dependent: it is not evidence for blanket replacement of generated assignment on all datasets.

**Unprofiled fixpoint and correctness.** Two independently locked sample pairs (`share/assign-{os-*,final-*}-time-{jython,scala}.log`); the second treatment is the final-source binary. Jython: **17.532 → 7.147 s**, **17.601 → 7.174 s** (2.45× in both). Scala: **5.212 → 5.607 s**, **5.191 → 5.634 s** (7.6% and 8.5% slower). Final-source xalan exact compared all **26/26 full-tuple outputs**, digest `081e42b9`. All four production count digests were unchanged: **scala `75b7003b`, soot `384bb362`, kotlin `c543cddb`, jython `66b903ac`**, exit 0 throughout.

## Sparse committed-pair membership in the actual publication path

The integration differs from the offline test-and-set benchmark above: `fresh` is read-only. An indexed subtract only tests bits and uses the original RCCL stable selection/count path. On *successful publication* of the fresh batch, the radix state inserts those pairs. Build/merge the unpublished sorted run first; remove the cache from the state while enqueueing its update; only retain it if all ranks and pages fit. Thus an append error leaves the old runs intact without a stale index. Replacement drops the cache. Existing snapshots never reference it. The canonical and secondary sorted runs—and the kernels that read them—are unchanged.

**Eligibility and budget.** Consider only an admission that has at least 524,288 distinct input words and at least 16,777,216 committed rows, in a binary layout with two ≤32-bit fields packed into one `u64`. Decode *encoded* fields using the actual width and canonical permutation, not hard-coded DOOP shifts. At most one index per process. Its entire reservation is bounded by `min(2 GiB, total_VRAM/16, currently_free_VRAM/4)` and by the size of its code directory. Reject an unaffordable shape before allocating. Unknown majors/minors are misses for a complete index; if an **insert** exceeds a code, rank or page limit, discard the index and use the unchanged binary subtraction. There is no approximate-membership or overflow-as-absent case. The overflow check costs one 4-byte readback per indexed append, included in timings.

**Isolating where time went.** On a generated-join snapshot (not the select2 manual-plan acceptance run), the Scala `varpointsto` subtraction changed from 389.9 ms to 5.7 ms binary + 55.8 ms bitmap (1.111 B indexed rows / 314 calls). Its append and merge steps, which include updating the side index, jointly cost 57.5 ms more. On Jython: 356.9 ms to 92.6 ms binary + 92.2 ms bitmap (1.831 B / 184), append and merge jointly 45.2 ms more. `GPU_DATALOG_PROFILE=1` fixpoints: 10.626 → 10.374 s and 16.239 → 16.115 s respectively. These isolate the mechanism, not the end-to-end claim on the chosen plan. Logs: `/data/tmp-targets/capture/pb-profile-{scala,jython}-{0,1}.log`.

**Runtime checks:** `cargo +1.99.1 oxide test --arch sm_89 --materialize-cubin -- --release -p gpu-datalog-runtime -- --test-threads=1`: 97 passed, zero failed, one pre-existing ignored. The new test checks repeated reads before publication, retained snapshot visibility, replacement, and fallback on rank, page and both code-domain overflows. `/data/tmp-targets/capture/runtime-pb-final-test.log`.

**Acceptance: keep.** Independent control and treatment builds of the same private workspace snapshot, differing only in the four runtime files. Both use the select2 planning state (`HEAD` of `doop/src/{lib,manual}.rs` and `features/datalog/doop/shared/src/mini_engine.rs`) and `cq-gpu-runtime/nvtx,manual-joins`. Runs alternate control/treatment within each dataset under the same exclusive GPU and timing locks. Mean gain is positive when the treatment is faster; wall includes startup, input, fixpoint and output. Peak is maximum `fixpoint_pool.peak_bytes` across samples, not all driver memory.

| Dataset | Control fixpoint s | Bitmap fixpoint s | Mean gain s | Mean process wall s | Peak bytes, control → bitmap | Counts |
|---|---:|---:|---:|---:|---:|---|
| Scala | 6.337 / 6.328 | 6.074 / 6.102 | +0.245 | 13.385 → 13.200 | 21,063,682,072 → 23,211,165,476 | `75b7003b` |
| Jython | 18.310 / 18.468 | 18.258 / 18.315 | +0.103 | 23.155 → 23.070 | 38,786,411,990 → 40,933,852,290 | `66b903ac` |
| Soot | 6.924 | 6.732 | +0.192 | 12.550 → 12.330 | 16,036,844,568 → 18,184,327,972 | `384bb362` |
| Kotlin | 6.232 | 6.233 | −0.001 | 18.270 → 18.420 | 24,545,550,968 → 24,545,550,968 | `c543cddb` |

Both Scala pairs improved (3.86% mean fixpoint), both Jython pairs improved (0.56% mean); no Kotlin win. One pair each on Soot/Kotlin does not establish a distribution. Jython's maximum measured pool peak is 40.934 GB; its *increase* is 2.147 GB. Xalan's independent exact check compared **all 26 full-tuple outputs**, counts digest `081e42b9`, treatment pool peak 300,603,488 bytes. No CPU tuple equality is claimed from a digest on the four larger datasets.

Artifacts: `/data/tmp-targets/capture/{stablepb-ab.tsv,stablepb-*.log,stablepb-*.log.wall,doop-pb-select2-{off,on}}`. Build from each snapshot's `experiments/gpu` after `source /dev/shm/gpu-env.sh`, with a separate `CARGO_TARGET_DIR` and `CUDA_OXIDE_PTX_DIR=$CARGO_TARGET_DIR/oxide-ir`:

```bash
flock -s /tmp/mini_linq-timing.lock timeout 1200 cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release -p doop-gpu --features cq-gpu-runtime/nvtx,manual-joins
T=/data/tmp-targets/capture; D=/data/doop-bench/corpus-20261110/raw
# Inside the exclusive GPU/timing locks, with no other GPU compute processes:
for d in scala jython soot kotlin; do
  reps=1; if [[ $d == scala || $d == jython ]]; then reps=2; fi
  for ((i=1;i<=reps;i++)); do for b in off on; do
    /usr/bin/time -f %e env GPU_DATALOG_PHASES=1 timeout 480 "$T/doop-pb-select2-$b" "$D/$d" --gpu-only
  done; done
done
flock /tmp/mini_linq-gpu.lock env GPU_DATALOG_PHASES=1 timeout 300 "$T/doop-pb-select2-on" "$D/xalan"
```

## Assignment dispatch: avoid paying for a union that does not save a sort

The first engine prototype always built the bitmap. On scala `ΔVPT → Assign` scans **612.4 M delta rows** for **972.3 M witnesses**; on jython the same orientation scans **438.3 M** for **14.78 B**. Delta size alone is the wrong selector. Also, `ma_ranges` counts children **once per distinct `from`**; it does **not** count the raw bag per delta edge. Waiting to build that index before choosing raw pays much of the cost that hurt scala.

The manual hole now estimates raw rows `Ĉ` before constructing the union: probe the midpoints of up to **1,024 equal-width delta strata** against all live RHS runs, count only, and scale by `N / draws`. Deltas below 256 use raw first without sampling. For each version choose union if **`Ĉ ≥ 2,000,000 + 4 N`**, otherwise directly emit witnesses into the candidate arena. Both routes have identical support; multiplicities are not a published obligation. The pilot never rejects a row. On a raw capacity miss the exact measured count can redirect an underestimated heavy round to union rather than growing the bag; no raw candidate exceeding the validated lease is published.

Direct emission uses a warp per long-range assignment edge. For sparse `ΔVPT → Assign` it searches several adjacent roots per warp, reserves once per warp/run and cooperates on long spans. When a large delta scans a smaller Assign relation, a temporary bounded per-run directory maps raw `from` codes directly to half-open ranges, including misses. No run persists across publications. In scala rounds with **N ≥ 65,536 and RHS < N/4**, directory-based raw expansion costs **36.8 ms across 150 calls**, versus **74.0 ms** with repeated binary search; for RHS ≥ N the directory no longer pays reliably. Giving each high-fanout `ΔAssign` edge its own warp cuts total *raw join* time on scala from **580 ms to 77 ms**. The large improvement is not just a threshold change.

**Per-round evidence.** `/data/design-notes/join-lab/assign-choice-{scala,jython}.tsv` has all **770 / 490** invocations with nonempty inputs, paired by `(version, actual recursive round, delta relation length, RHS length, run count)` in full production runs. Columns include raw and union join wall µs, actual bag `C`, unique `U`, and pilot estimate. The timed region includes the pilot, device work/allocation, readback, and capacity retries; no diagnostic print is inside the interval. The two admission-model columns separately add **0.22 ns/key** for sorting (same-tree `varpointsto` admission scopes previously measured 0.219/0.228 ns/key). Actual mixed-head admission is not separately timed per hole. The forced raw measurements include the directory through RHS ≤ 3N; dispatch tightens this to RHS ≤ N from the measured directory crossover. Both forced routes gave the expected dataset digests.

| Dataset / version | Predicted raw / union calls | Always raw | Always union | One shared cutoff | Oracle per-call min |
|---|---:|---:|---:|---:|---:|
| scala / `@0` | 203 / 168 | .306 | .270 | **.239** | .236 |
| scala / `@1` | 398 / 1 | .286 | .916 | **.289** | .285 |
| jython / `@0` | 149 / 88 | 6.541 | .347 | **.325** | .324 |
| jython / `@1` | 161 / 92 | 3.717 | 1.002 | **.961** | .934 |

Times above are **measured join + modeled marginal sort seconds**, not fixpoint. A grid over fixed thresholds 0.125–16 M and linear coefficients 0–64 on both versions and corpora chose `(2 M, 4)`, total **1.813 s** vs **1.780 s** choosing the faster completed route retrospectively. Even rounds choose the same rule; fitting only odd rounds picks `(4 M, 2)` with **1.821 s** on all rows. The selector uses no dataset name, iteration or hindsight unique count.

Reproduce the experiment with the same isolated build instructions above. Adaptive SHA-256 `2a62f57bb612cdc5519c16e67716c24a6509dc1aebb150322fa46e264d0850a9`; the unchanged, manual-array-only control remains `ed8fe7ca38c70508be494633fd5fcc27cf1e797f8d8bf2f5f96ef046dcc2b724`. The bounded forced-route flag is diagnostic only; leave it unset for fixpoint timing. The timestamps enclose the entire path including pilot and retries, not the set-admission sort that follows it.

```bash
P=/data/tmp-targets/duplab; C=/data/tmp-targets/duplab-control/control
D=/data/design-notes/join-lab; H=/data/design-notes/doop-memory/profile-xlarge/share/run.sh
for d in scala jython; do
  for p in raw union; do
    DOOP_ASSIGN_PROBE=$p /bin/bash "$H" "$P/assign-adaptive-v1" "assign-replay-$p" time "$d"
  done
  python3 "$D/assign-choice-analysis.py" --dataset "$d" --raw-label assign-replay --union-label assign-replay --output "$D/assign-replay-$d.tsv"
done
python3 "$D/assign-choice-fit.py" "$D"/assign-replay-{scala,jython}.tsv
/bin/bash "$H" "$P/assign-adaptive-v1" assign-adaptive-exact exact xalan
for d in scala soot kotlin jython; do
  for r in 1 2; do
    /bin/bash "$H" "$C" "assign-choice-c$r" time "$d"
    /bin/bash "$H" "$P/assign-adaptive-v1" "assign-choice-a$r" time "$d"
  done
done
```

**Final alternating fixpoints.** Seconds in execution order within each dataset; each run held the GPU and timing locks with no foreign compute tenant before or after. The only control/treatment source difference affecting the engine is the assignment-hole substitution. Both builds retain the manual array-load rule.

| Dataset | Generated 1 | Adaptive 1 | Generated 2 | Adaptive 2 | Mean speedup | Output-count digest |
|---|---:|---:|---:|---:|---:|---|
| scala | 5.2260 | **5.0326** | 5.2165 | **5.0320** | 1.038× | `75b7003b` |
| soot | 6.2989 | 6.2816 | 6.2677 | 6.2507 | 1.003× | `384bb362` |
| kotlin | 4.9312 | **4.8165** | 4.9982 | **4.7926** | 1.033× | `c543cddb` |
| jython | 17.5376 | **7.0314** | 17.5998 | **7.0598** | **2.494×** | `66b903ac` |

Xalan exact: **all 26 full-tuple output sets match**, digest `081e42b9`; all 16 timing runs exited 0 with the expected digests. Soot is effectively unchanged. Scala's former **7.6–8.5% regression** becomes **3.5–3.7% lower fixpoint time**. In the adaptive scala profile the join scope is **3.173 s** (generated 3.196, always-union 3.939), while total admission sort/unique is **.711 s** (generated .867). Consequently the cheaper producer more than pays for sorting somewhat more than the always-union prototype. Jython still sorts just **3.287 B** `varpointsto` rows instead of **43.372 B**; its adaptive profile is **5.237 s joins + 1.006 s sort/unique**, with a **7.157 s** fixpoint and **18.00 GB** peak device pool (control **38.79 GB**). The table is unprofiled; these profile scopes are separate runs.

## Summary: per class, before vs now vs bound (commit `07c9f483`)

Kernel seconds per rule version from `GPU_DATALOG_PROFILE` (scala unless
noted): before = `profile-manual5-scala.log` (hand-written VPT_LoadArray@3
only), now = `final1prof-scala.log` / `final1prof-jython.log`. Bounds are the
lab's measured kernels on captured rounds or the caps model named.

| class | rule versions | before | now | bound / best lab | what closed the gap |
|---|---|---:|---:|---|---|
| A: delta probe, nearly all miss | 136, 143, 145 (virtual dispatch) | 0.80 + 0.46 + 0.46 s | 0.21 + 0.16 + 0.16 s | replay of 136: 0.46-0.64 ms/round vs now 0.51 ms/round (0.209 s / 407) | GPU-RootSelect (no cooperative scans for rows with no match) |
| A, plan order | 123 cast `@4`, 122 cast `@0` | 0.70 + 0.46 s | 0.17 + 0.03 s | 123: >=10.45 G of 10.93 G children avoidable; 122: zero output | authored orders (`plan-advice.md`) |
| B: multi-level expansion | 129 VPT_LoadArray@3, 128 @2, 125/126 AIPT_Store | 0.11 + 0.39 + 0.38 + 0.45 s | 0.12 + 0.06 + 0.17 + 0.24 s | 129: 0.36 G probes at the L2 search cap (~16 G/s) + 1.4 GB writes = ~25 ms | orders for 128/125/126; 129 unchanged |
| C: duplicates, field load | 132 VPT_LoadField@3 | 0.44 s (jython 2.94 s) | 0.23 s (jython 0.25 s) | bitset-union replay 0.6-0.9 ms/round | static load relation first (order) |
| C: duplicates, assignment | 118/119 VPT_Assign (jython) | 41.98 G candidates; sort/unique 10.19 s | 3.29 G sorted rows; sort/unique 1.01 s | union read+write model 0.8-12.9 ms/round | grouped-union hole with raw fallback |

Whole program (fixpoint, two runs, `final-validate.sh`): scala 4.29 s, soot
4.86 s, kotlin 4.24 s, jython 4.64 s; SRDatalog 9.44 / 9.93 / 10.52 s live,
jython 15.10 s published: 2.0-3.3x. Remaining split (profile): scala joins
2.75 s + sort/unique 0.71 + subtract 0.27 + merge 0.45; jython joins 3.00 s +
sort/unique 1.01 + subtract 0.21 + merge 0.28.

**Class B decision (measured):** the triangle-style fused kernel was the plan
for VPT_LoadArray, but after the orders that class is 0.12 + 0.06 s on scala
(0.04 s jython); a kernel at the ~25 ms bound would save under 0.1 s of
4.29 s. The largest remaining items are elsewhere:
- 136/143/145 run the same body three times for three heads (0.52 s scala,
  1.02 s jython): one body evaluation writing three arenas would remove two
  of them.
- AIPT_Store@2/@3 (0.41 s scala), VPT_LoadField@3 (0.23 s scala, 0.25 s
  jython), VPT_Assign@1 (0.64 s jython).
- Per-launch fixed cost of survivor dispatches (~100 µs median across 400+
  rounds per version).
