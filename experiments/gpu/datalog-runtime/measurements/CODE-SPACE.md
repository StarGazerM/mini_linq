# Packed-run code-space conversion

## Measured outcome

The final real-read E/P/C medians are **152.159 / 145.809 / 179.849 ms**; admission is **182.996 / 139.219 / 154.772 ms**. Against this run's clean baseline, packed read time falls from **182.456 to 179.849 ms**, and packed admission from **157.853 to 154.772 ms**. Packed remains **18.18% slower than E for reads**, despite being **15.33% faster for admission**.

The memory trade is substantial: C retains **122,555,552 state bytes** and peaks at **131,314,176 pool bytes**, versus E's **271,198,552 / 293,549,568**. Metadata-derived frames cost **25,414,456 bytes more than exact block frames**. This is not an integrated-engine speedup claim.

All **35 final timing processes were clean** under the recorded before/after protocol, as were the final scoped counters. Complete compact evidence, including every specialization's registers/stack and all stall/load counters, is in `CODE-SPACE.json` under `/data/design-notes/doop-memory/`. Measured code is `65f8aa1ed1`; the intentionally uncommitted RCCL manifest/lock override remains outside the commits.


## Contract and measurement boundaries

A row remains a concatenation of per-column residuals. Search, membership, subtraction, merge and enumeration must not reconstruct a global `W` key. Extracting residual bits is not decoding. Translation between source and destination frames is `code + base_source - base_destination`; it must not construct the intervening global value.

The existing 256-row layout, first-key fence array and 32,768-row plain cutoff are retained. Two boundaries are explicitly authorized:

- Oracle/export: `PackedRun::unpack` and `PackedSet::to_host` publish reconstructed keys for verification, outside measured operations.
- Fence maintenance: exactly one key per non-copied output block is translated to the run-wide fence frame by the block writer. This is not a decoded merge input or scratch representation. Copied blocks copy their fences.

Plain input has no frame metadata. It is already in the zero frame. Output frame selection may inspect contributing plain rows, but packed inputs contribute metadata envelopes, not decoded rows. The envelope maximum is saturating `base + (2^width - 1)`; width zero and width 64 require explicit handling.

## Membership and subtraction

Previously membership performed lower bound, then reconstructed the candidate row with `device::row` for equality. Membership now specializes the bound search, encodes the probe once in its selected block, and returns equality from that same code comparison. The membership fence search includes equal fences so that a key equal to a block's first row is checked in that block. Out-of-frame probes cannot be mistaken for the clipped artificial probes used by insertion-bound searches.

Small admission delegates membership to the same code-space implementation after its unchanged shared-memory sort and duplicate elimination. Plain-storage specialization remains direct plain comparison.

## Search

The fallback of the existing bound search applies only to uncompressed blocks or plain runs. It now compares their zero-frame lanes in place, rather than calling the general decoder. Wide packed search compares scalar code fragments without assembling a candidate `W`: two-lane keys load both fragments independently; larger keys compare most-significant fragments first.

## Evidence location

All new raw binaries, logs, process results and profiler captures belong under `/data/design-notes/doop-memory/code-space/`, not in git. Prior numbers in `REAL-READS.md` are historical evidence, not measurements of this conversion. Timing samples must have empty compute-process queries before and after the exclusive GPU lock; compilation uses only the separate shared timing lock. Five fresh processes are required for final timing summaries.

## Merge and frame selection

Co-rank first narrows the diagonal with the unchanged run-wide fences. Its remaining A indices lie in one block; because the output diagonal is block-aligned, the corresponding B indices also lie in one block. The final implementation caches each column's two bit locations, widths and constant base difference once for that pair. The cache is thread-private shared metadata, not a payload tile fetch or a change to the resident layout. Very wide schemas use pool scratch instead of acquiring a new schema-width restriction.

Before conversion, co-rank reconstructed both candidate `W` keys; analysis and packing reconstructed merge tiles into shared `W` slots. The new path never uses decoded merge scratch.

The frame kernel scans **only plain input rows** in the production policy. Packed input contributes its block envelopes, clamped to the column's domain maximum. The former packed-row min/max analysis is gone. The existing metadata/offset/allocation sequence remains: plain inputs are read again by packing, but there is no additional production launch.

Packing stages concatenated **output-frame codes** in shared `u64` slots, not decoded `W` keys. Shared merge-path comparisons inspect these code slots; the permutation consists of `u32` row indices rather than register arrays of full keys. Repacking gathers code bits according to that permutation. A single-source slice inside one input block copies its bit interval directly, including unaligned slices. Identical source/output frames also copy row bits without extracting fields. Raw payload blocks retain the existing zero-frame representation.

`PACKED_MERGE_FRAMES` selects reproducible diagnostic policies:

- `metadata` (default): packed block envelopes plus exact contributing plain extrema.
- `exact`: reduce contributing **codes**, then translate their extrema once into frame metadata. Column bit locations are computed once per input block, not once per row.
- `zero`: metadata envelopes with schema-wide zero frames for metadata-less plain inputs. This is a comparison, not the production choice.
- `fences`: production metadata selection plus a separately timed fence-only diagnostic launch after the ordinary pack. It counts actual packed-frame rebases and fence copies. This process is **not an admission timing sample**. Its isolated event interval includes launch/scheduling and first-row comparison, and is not an additive attribution of the fused pack kernel.

The initial scalar exact-code experiment recovered 97,141,096 final state bytes, but its five-process frame-analysis median was 86.955 ms: it redundantly traversed preceding columns for every row. That implementation was replaced with one bit cursor per input block/column before drawing conclusions about exact-frame costs.

## Enumeration and source-derived probes


Before conversion, balanced-source fallback and recorded-child jobs called `device::row`; strided scans added the frame base to every extracted field. Those operation calls and per-row value reconstructions are gone.
`device::codes` exposes `(code, frame, field)` to a consumer. `checksum_row` and the strided scan reduction add code contributions to frame-base contributions; the latter multiplies each base contribution by the number of rows in that block/stride. The existing lane-weighted checksum oracle is unchanged. Carries across lane boundaries and partial-suffix truncation require algebraic corrections; these are not reconstructed value fields. Full-field reads omit unnecessary truncation masks and corrections. Residual fields of at most 32 bits use 32-bit extraction; wider fields retain the 64-bit path.

`device::code_bound` accepts a source view and row address, translates its residuals directly into the selected target frame, and compares target codes. The cross-frame GPU test exercises this on 64-, 128-, 192- and 256-bit keys, including source/target block geometries of 17/256 rows.

The real replay's child jobs now use the code/frame checksum consumer. Its captured nested probe keys still enter as captured keys: the trace records child addresses and nested probes, **not the parent-child column-carriage edges needed to reconstruct a fused join's data dependency**. No fabricated lookup is added to child jobs. The replay therefore measures actual code-space enumeration, but does not establish fused-join register pressure, dependent probe latency, demanded-column carriage, or concurrent engine cache behavior. `code_bound` provides and tests the missing operation without claiming the recorded replay exercises an unknowable edge.

Another boundary matters for interpretation: the read replay seals each captured immutable run directly. Its read timings do not measure the wider frames produced by the metadata-only admission merge. Admission byte measurements and replay read times must not be silently combined into a claim about integrated engine execution.

## Reconstruction registry

| Marker | Implementation / entry point | Callers and boundary |
|---|---|---|
| P | `device::export_row` (private) | Only `kernels::unpack`; never an operation reader. |
| P | `PackedRun::unpack` | Explicit host oracle/export; outside operation timers. |
| P | `PackedSet::to_host` | Replay final exactness and tests, using exported runs. |
| I | `merge_kernels::write_fence` | One first-row rebase per output block at most, called by `pack_merge` and the opt-in fence diagnostic. Existing fences and plain first rows are copied. This is the expressly authorized fixed run-wide-frame maintenance exception. |
| P | Three new test export blocks | Exact merge, bit-copy and frame-policy host assertions. |

All markers are registered in `doc/DECODE-SITES.tsv`. The checker validates marker/registry identity, not semantic completeness. After the atomic cutover amend it passed with 523 rules, 478 conformance records, 38 design anchors and 522 decode markers (R=14, P=281, V=76, I=22, ?=129).

## Commit and exactness protocol

The selective commit tool relocated unrelated hunks when selecting search changes. The broken unpushed commit was replaced by atomic cutover `c3536015ae`; the incident was reported to the tool owner. The amended source and test files were byte-for-byte compared against `git show HEAD:<path>`. `git diff HEAD -- .` contained only the intentionally uncommitted RCCL overrides in `experiments/gpu/Cargo.toml` and `Cargo.lock`. The unchanged 31 packed-run tests and five new code-space tests passed before and after that amend. Per-path binaries retained outside git preserve the conversion experiments despite the necessary atomic commit.

Reproduce admission-only comparisons with `packed-replay ADMISSION_DIR --admission-only --output RESULT.json`; set `PACKED_MERGE_FRAMES` explicitly for alternative policies. `code_space_counters.py` selects recorded state-90 admissions by kernel-filtered launch index and records registers, occupancy, active-thread instructions, load requests and four stall categories under the affected NVTX scope.


## Rejected first arithmetic form

The first fully code-space cutover was exact, but its enumeration arithmetic was not acceptable as a performance endpoint. On the 268,435,456-row scaled replay (ten timed repetitions per workload), state 90's packed range-scan total rose from **53.788 to 60.048 ms**; state 112 rose from **21.966 to 29.337 ms**. These are same-workload before/after process observations, not engine improvements. The first form unnecessarily masked complete fields and computed impossible full-field truncation carries. Those operations were removed, and narrow residual extraction was moved to 32-bit arithmetic.

The initial cutover's five-process real-read E/P/C medians were **152.611 / 146.074 / 183.935 ms**, with paired C/P **1.25919**. Its admission E/P/C medians were **182.922 / 142.072 / 157.782 ms**. Packed merge's partition/frame/pack event medians were **5.595 / 5.831 / 11.040 ms**, with **35.834 ms** synchronized host merge time. These results identify the initial form; they are not substituted for the final optimized measurements.

That form's final packed state occupied **122,555,552 bytes**, versus exact-frame **97,141,096 bytes**, an increase of **25,414,456 bytes (26.16%)**. Its peak/live pool bytes were **131,314,176 / 122,629,632**. The zero-plain comparison occupied **158,892,256 state bytes**, confirming why schema-wide plain frames are not the default.

The admission geometry independently contains **55,049,423 rows across 215,153 packed merge output blocks** (255.862 rows per written fence). This is the total number of fence writes, not a claim that every fence needs reconstruction; the opt-in diagnostic distinguishes copied fences from packed-source rebases.


The scoped reduction counters explain why the first arithmetic form was rejected:

| Scaled state / form | Active-thread instructions/row | Registers | Achieved occupancy % | Global load requests/row | Reduction ns |
|---|---:|---:|---:|---:|---:|
| 90 / original | 132.16 | 64 | 58.05 | 0.27070 | 3,181,216 |
| 90 / first code checksum | 156.26 | 80 | 41.98 | 0.27226 | 3,827,072 |
| 112 / original | 130.97 | 64 | 58.49 | 0.18330 | 2,051,648 |
| 112 / first code checksum | 172.96 | 80 | 41.84 | 0.18351 | 2,805,504 |

These are counter captures, not clean timing samples. Loads barely change, while instructions and registers increase and occupancy falls. **[INFERENCE]** The regression is unnecessary checksum correction/control state rather than avoided decoding loads. Merely narrowing extraction did not fix it: the intermediate cached variant still used 80 registers and 160.06 / 180.92 instructions per row.

The retained design selects the checksum consumer from the existing schema and demanded suffix: complete fields within lanes use a linear code sum, and fields known to be at most 32 bits use a narrow extractor. Partial/cross-lane cases retain the exact general algebra. Widths themselves remain runtime values; there is no 32-way width specialization, payload tile fetch, interleaved storage, or register cap.

## Relation-wide-frame model (not implemented)

An offline pass over the captured fresh native keys computes a single exact column frame per final state. Keeping the same run geometry, plain cutoff, headers and fences, that model needs **153,074,376 payload bytes / 161,013,360 total state bytes**, versus exact block-frame **89,202,112 / 97,141,096 bytes**. Uniform frames therefore add **65.75%** over exact block frames and **31.38%** over the metadata-merge result. This is a byte model from captured data, not an implementation or timing result.

Uniform frames would remove repeated cross-block translation setup, not payload bit extraction, binary-search dependencies, scheduling, checksum work or output traffic. The final counter tables bound those costs; instruction totals are not an attribution of all extra instructions to frame differences.


## Retention decisions

- Membership reuses encoded-probe equality; the representative admission kernel reduces instructions from 2,029.75 to 1,691.71 per probe and registers from 48 to 40.
- The first scalar checksum form is rejected above. Schema-selected linear code sums restore 64-register reduction kernels; generic partial/cross-lane oracles remain separate specializations.
- The first co-rank cache reduced instructions but lengthened its representative kernel from 17.184 to 18.400 us. Spreading independent partitions across 64-thread CTAs, rather than 256-thread CTAs, reduced the measured sample to 14.880 us without changing the payload layout.
- Code-space small admission initially exposed more load-wait time despite fewer instructions. Only sort tiles larger than 256 candidates now use 512 threads. The 1,023-row sample fell from 77.024 to 49.920 us; smaller tiles retain 256 threads. This changes actual work distribution, not register caps.
- The one-fragment-at-a-time wide comparison regressed state 112's mixed-prefix trial from 2.389 to 2.702 ms. A two-scalar 128-bit comparison restored the targeted trial to 2.369 ms, while retaining codes rather than constructing a `W` candidate. Final five-process measurements use that form.

These are workload-specific decisions, not a claim that every query class or wider generic specialization is faster. In particular, packed real-read kernels can retain more registers than before even while executing fewer instructions.

## Measurement quality and reproduction

The initial baseline process set overlapped host compilation and is retained as diagnostic history only. The final before/after tables use `baseline-clean/` and `committed-final/`. Each final timing set contains five fresh processes. Raw before/after compute-process lists, executable hashes, oracle results and counters are retained alongside each sample.

One intermediate cached counter capture was rejected when `/usr/share/rustdesk/rustdesk` appeared after profiling. It is not silently treated as a clean timing. At the user's direction, the new runners accept **only** that named desktop session with `--allow-rustdesk`, preserve its process lists, and label affected times **indicative (rustdesk present)**. Other contenders remain errors. Counter counts remain usable; contended timing is never relabelled clean.

```sh
source /dev/shm/gpu-env.sh
R=/data/design-notes/doop-memory/code-space
T=/data/design-notes/doop-memory/real-replay
M=/data/design-notes/doop-memory/wire-in/cursor/census-tree/experiments/gpu/datalog-runtime/measurements
python3 "$M"/code_space_measure.py \
  --processes 5 --allow-rustdesk "$R/dual-replay" "$R/new-final" -- \
  "$T/admission-v2" --real-trace "$T/joint-v2"
python3 "$M"/code_space_counters.py \
  "$R/dual-replay" "$T/admission-v2" "$R/new-counters" \
  /opt/nvidia/hpc_sdk/Linux_x86_64/25.9/compilers/bin/ncu --allow-rustdesk
```

The runners take the exclusive GPU lock themselves. Builds use only the separate shared timing lock, with `cargo +1.99.1 oxide ... --arch sm_89 --materialize-cubin`; the two locks are never held together. No cloud GPU or RCCL source change is involved.


## Final real-read tables

Times are five-process medians of CUDA-event stream intervals. E/P/C are measured in the same process. Kind rows are separate stratified launches and **must not be added** to the mixed total.

| Workload | E ms | P ms | C ms |
|---|---:|---:|---:|
| Mixed total | 152.159 | 145.809 | 179.849 |
| Membership misses | 19.257 | 18.146 | 19.460 |
| Prefix-range misses | 69.845 | 67.763 | 84.797 |
| One-row hits | 119.334 | 114.171 | 138.446 |
| Short ranges | 40.305 | 38.446 | 48.008 |
| Long ranges | 13.621 | 13.052 | 16.083 |

The replay checks **222,416,739 probes and 69,170,748 actual children**, in **19,324 nonempty launch/site groups**. Paired total P/E, C/E and C/P are **0.95741 / 1.18178 / 1.23427**. The clean before-conversion totals were **152.382 / 145.898 / 182.456 ms**.

## Final admission and memory

These are synchronized host-operation medians. Operation medians need not sum to the independently computed total median. Fused small admission is a **subset** of subtraction, not an additional charge. E exposes sort/subtraction as one existing operation; it was not instrumented or modified to invent separate E costs.

| Operation | E ms | P ms | C ms |
|---|---:|---:|---:|
| Total | 182.996 | 139.219 | 154.772 |
| E sort + subtraction | 164.611 | — | — |
| Sort | — | 65.608 | 65.285 |
| Subtraction, including small fused path | — | 47.623 | 46.172 |
| Small fused subset | — | 27.998 | 27.954 |
| Seal / plain adoption | 0.527 | 2.602 | 7.586 |
| Merge / publication | 17.866 | 23.487 | 35.773 |

Paired admission P/E, C/E and C/P are **0.76175 / 0.84670 / 1.11152**.

| Packed merge interval | Before ms | After ms |
|---|---:|---:|
| Host merge | 37.175 | 35.773 |
| Partition event interval | 5.373 | 5.019 |
| Frame-selection event interval | 6.163 | 5.803 |
| Pack event interval | 12.530 | 11.578 |

Event intervals are not exclusive kernel attribution: short launches include submission gaps, and plain merges retain the existing event-mark protocol. Offsets, payload-size readback, allocation and other host/stream work are outside the three displayed device intervals.

| Representation | Final state bytes | Final live pool bytes | Peak pool bytes |
|---|---:|---:|---:|
| E | 271,198,552 | 271,222,784 | 293,549,568 |
| P | 271,192,208 | 271,216,640 | 293,502,208 |
| C | 122,555,552 | 122,629,632 | 131,314,176 |

These are application-buffer allocations, not total process VRAM or allocator-reserved memory. No retained metadata-policy block fell back to raw payload storage in this final captured admission state; small plain runs remain unchanged.

### Frame-policy comparison

All alternatives use five fresh processes, identical admissions and the unchanged cutoff/layout.

| Policy | State bytes | Payload bytes | Peak pool bytes | Admission ms | Merge ms | Frame interval ms |
|---|---:|---:|---:|---:|---:|---:|
| Metadata + exact plain extrema (kept) | 122,555,552 | 114,616,568 | 131,314,176 | 154.772 | 35.773 | 5.803 |
| Exact contributing codes | 97,141,096 | 89,202,112 | 103,871,744 | 167.999 | 49.554 | 19.903 |
| Zero-frame plain envelope (not the default) | 158,892,256 | 150,953,272 | 175,216,896 | 151.618 | 32.222 | 3.085 |

Exact tightening recovers all **25,414,456 extra bytes**, but costs about **13.8 ms more merge time** in this implementation. It is not worthwhile for the measured admission latency objective; it remains an explicit diagnostic option when footprint matters more. This is not a claim that every possible exact-code reduction implementation must cost that much.

The fence-only diagnostic observes **215,153 writes: 183,941 packed-source rebases and 31,212 copies**, over 243 launches. Its isolated event sum is **1.450125 ms**. Normal packing performs the fence maintenance in its existing kernel; the diagnostic's extra launches are not present in timing samples and their time is not added to the fused pack interval.

## Synthetic and scaled workloads

Read times below cover **ten warmed repetitions**, using synchronized host intervals, not the real replay's event intervals.

The existing synthetic trace has **10,740 rows**. All retained runs are plain under the unchanged cutoff: P/C equality is not a compression win. E/C share each process; the P control uses a separate five-process cohort.

| Synthetic | E | P | C |
|---|---:|---:|---:|
| Admission ms | 4.273 | 3.926 | 3.925 |
| Sorted prefix ms | 0.463 | 0.486 | 0.485 |
| Shuffled prefix ms | 0.551 | 0.503 | 0.503 |
| Mixed prefix ms | 0.545 | 0.494 | 0.493 |
| Membership ms | 0.610 | 0.508 | 0.508 |
| Range scan ms | 0.678 | 0.783 | 0.786 |
| Final state bytes | 368,640 | 215,568 | 215,568 |
| Peak pool bytes | 529,152 | 436,992 | 436,992 |

Both scaled states contain **268,435,456 rows**. Each E/P/C row uses five fresh paired processes.

| State / representation | Sorted prefix ms | Shuffled prefix ms | Mixed prefix ms | Membership ms | Range scan ms |
|---|---:|---:|---:|---:|---:|
| 90 / E | 5.474 | 38.109 | 65.362 | 1.031 | 66.495 |
| 90 / P | 5.490 | 37.667 | 64.590 | 1.066 | 59.125 |
| 90 / C | 3.384 | 21.678 | 37.289 | 0.437 | 52.762 |
| 112 / E | 0.218 | 1.658 | 3.645 | 1.528 | 85.363 |
| 112 / P | 0.198 | 1.615 | 4.098 | 1.514 | 51.608 |
| 112 / C | 0.277 | 1.610 | 2.373 | 0.488 | 20.575 |

Packed scan totals improve from the before-conversion **53.788 / 21.966 ms** to **52.762 / 20.575 ms** for states 90/112. Not every subcase improves: state 90's already-code-space mixed-prefix path measured **36.257 ms before versus 37.289 ms after** (one baseline process versus five final processes). No universal lookup speedup is claimed.

| State / representation | Final state bytes | Peak pool bytes |
|---|---:|---:|
| 90 / E | 2,147,483,704 | 2,306,863,616 |
| 90 / P | 2,147,483,712 | 2,306,863,360 |
| 90 / C | 630,745,528 | 2,778,229,760 |
| 112 / E | 4,294,967,352 | 4,306,516,224 |
| 112 / P | 4,294,967,360 | 4,306,515,968 |
| 112 / C | 643,309,764 | 4,938,277,632 |

The scaled C construction peak includes the native input batch and compressed output simultaneously. Compression lowers retained bytes but does **not** lower this one-shot sealing peak.

## Final scoped counters

Entries are before/after. Admission samples are state 90: 93,097 membership probes, 1,023 small candidates, and 2,301,108 merge output rows. Requests are global-load requests, not bytes or sectors. Counter durations are microseconds and are not the five-process timing samples.

| Kernel | Instructions/atom | Registers | Occupancy % | Load requests/atom | Kernel us |
|---|---:|---:|---:|---:|---:|
| Membership | 2029.75 / 1691.71 | 48 / 40 | 40.18 / 40.35 | 3.48219 / 2.77613 | 12.224 / 10.656 |
| Small admission | 4375.99 / 4107.03 | 54 / 40 | 16.65 / 33.32 | 5.98436 / 4.77126 | 77.024 / 50.016 |
| Co-rank partition | 15.01 / 10.86 | 40 / 48 | 15.50 / 4.08 | 0.03014 / 0.01169 | 17.184 / 14.496 |
| Merge frames | 353.68 / 120.17 | 46 / 64 | 79.87 / 60.57 | 0.50463 / 0.09858 | 115.872 / 26.592 |
| Merge pack | 589.55 / 665.81 | 60 / 80 | 64.74 / 48.12 | 0.93824 / 0.89445 | 118.336 / 105.408 |

Pack executes more instructions and uses more registers, yet its sampled kernel and full merge improve. Fewer instructions or higher occupancy alone are not sufficient performance models. The old full-key temporary arrays are gone; the complete specialization-level stack/register evidence is retained in the JSON.

The same three heavy recorded read sites are selected before and after, using the clean baseline's ranking and each site's largest probe-plus-child launch. C instructions/probe include child work.

| C site rank | Instructions/probe before/after | Registers before/after | Occupancy % before/after | Kernel us before/after |
|---|---:|---:|---:|---:|
| 1 | 3337.44 / 2148.67 | 64 / 68 | 64.18 / 47.80 | 178.080 / 135.552 |
| 2 | 1822.85 / 1821.54 | 64 / 68 | 42.47 / 41.76 | 14.272 / 13.824 |
| 3 | 2220.46 / 1879.37 | 64 / 68 | 33.87 / 35.92 | 23.200 / 18.432 |

The matched E/P read kernels retain **34 / 30 registers** at these sites. C's 68-register allocation is an important remaining cost, not a claimed register reduction.

| Reduction | Instructions/row before/after | Registers | Occupancy % before/after | Kernel us before/after |
|---|---:|---:|---:|---:|
| State 90 | 132.16 / 105.64 | 64 / 64 | 58.05 / 58.11 | 3181.216 / 3112.800 |
| State 112 | 130.97 / 71.75 | 64 / 64 | 58.49 / 58.89 | 2051.648 / 1918.080 |

All four requested stall categories and load requests are retained before/after in the compact JSON. For example, the final reductions have long-scoreboard shares **74.82% / 75.51%**: removing arithmetic leaves a more visibly memory-wait-dominated kernel. Stall percentages are per-active-warp categories, not additive wall-time attribution.

## What remains

| Cost class | Remaining work |
|---|---|
| Bit extraction | Packed payload loads, masks, shifts and straddling-word handling; assembling output bitstream fragments. |
| Per-block frame differences | Probe translation, cached cross-frame differences, output-frame selection, row-code rebasing/repacking, and authorized run-wide fence maintenance. Metadata-derived frame widening increases bytes. |
| Neither | Fence and row binary-search control, merge-path scheduling, sorting/selection, range-prefix scheduling, reduction/atomics, replay input/output traffic, checksum weighting, launches, synchronization, allocation and payload-size readback. |

For the selected large merge, the **entire** frame kernel is 120.17 instructions/output row, 0.09858 global-load requests/output row and 26.592 us. These are upper bounds on work that relation-wide frames could remove; metadata writes remain. The entire partition kernel is only 10.86 instructions/output row and 14.496 us, and uniform frames cannot remove its fence/search dependencies. Its removable translation subset is smaller. Combined with the measured uniform-frame byte model, these counters do not justify claiming a free uniform-frame speedup.

Final correctness covers all 31 unchanged packed-run tests, six added cross-frame/code-probe/frame-policy/copy tests, every synthetic replay, both scaled states, all real read bounds/checksums and admission full-tuple oracles. No engine code, resident block layout, fence layout, cutoff, filter or directory was changed.

