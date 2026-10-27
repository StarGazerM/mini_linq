# Mac (Apple silicon) calibration and cost model

Branch `mac-cost-model`. The experiments use standalone workspaces; the native
provider and join improvements live in the main workspace. Five experiment pieces:

- `calib/` — `maccal`: measured primitive costs on this machine (CPU, GPU, and
  CPU+GPU concurrently). Rust + Metal; see `calib/README.md` for every primitive.
- `model/` — trace-driven cost model over those primitives:
  `model.py` (structures × policies × placements × workload) and `analyze.py`
  (best structure per placement over a scenario grid).
- `index-bench/` — pinned C++ research artifacts and hash baselines, exercised on
  common exact lookup / grouped join workloads; see the benchmark section below.
- `gpu-index/` — real Metal build/merge/directory experiments, including
  flat/binary-carry histories; [measured results and commands](gpu-index/README.md).
- `query-expansion/` — matched CPU/GPU index construction and query experiments.

The consolidated GPU runtime lives in [`cq-mac/`](cq-mac/src/lib.rs), backed by
[`mccl/`](mccl/README.md) and its canonical Metal kernels. These remain standalone
workspaces outside the portable core; all active commands run from this repository.

The [Mac implementation plan](../../doc/MAC-IMPLEMENTATION.md) separates
GPU-assisted host providers, LSM-like snapshot maintenance, Datalog publication,
and a future Metal-resident Sequence backend, including their compiler boundaries.

Primitive-calibration machine: Apple M5 Pro, 16-core GPU, **15 logical CPUs**
(5 super + 10 perf), 24 GB unified memory, macOS 27.0.1, Metal 4.
Results dated 2026-10-23. The separate index benchmark runs on an **M3 Max,
128 GiB, 16 logical CPUs**; do not combine its timings with the M5 primitive curves.

## Measured primitives (1 GiB footprints unless noted)

| primitive | GPU | CPU 1 thread | CPU 15 threads |
|---|---:|---:|---:|
| seq_read | 275 GB/s | 86 GB/s | 238 GB/s |
| seq_write | 237 GB/s | — | — |
| seq_copy | 260 GB/s | — | — |
| merge (nominal 3 B moved per element byte) | 308 GB/s (0.04 ns/elem) | 6.8 GB/s (1.75 ns) | 77 GB/s (0.16 ns) |
| sort u32 | 4 × partition_pass @ 0.11 ns/pair ≈ 0.45 ns/key | 11.2 ns/key | 1.54 ns/key |
| hist+scan 2^16 bins | 0.08 ns/key | 0.48 ns | 0.08 ns |
| random line gather | **0.69 ns/line** (186 GB/s) | 6.1 ns | **0.68 ns/line** (189 GB/s) |
| random RMW (atomic add) | 1.35 ns/line | 12.1 ns | 2.38 ns |
| hashbrown probe, 50 % hits (64 MiB / 1 GiB) | — | 6.8 / 11.2 ns | 0.92 / 1.44 ns |
| dependent-load latency (64 MiB / 1 GiB / 4 GiB) | — | 105 / 119 / 185 ns | — |

Overheads: Metal command buffer with one dispatch, commit→completed **128 µs**
(17 µs for an empty command buffer); **+1.8 µs per additional dispatch**;
`MTLSharedEvent` CPU↔GPU round trip **1.8 µs** (CPU spinning) / **13 µs**
(listener notify); CPU pool wake 0.8–1.5 µs (≤10 threads), 56 µs at 15 threads
(all cores busy). Single-pass decoupled-lookback scan makes forward progress on
this GPU and is 1.33× faster than reduce-then-scan at 1 GiB.

## CPU + GPU concurrently (contention, 1 GiB footprints)

| CPU job (threads) | solo → concurrent GB/s | GPU job | solo → concurrent GB/s |
|---|---|---|---|
| seq_read (15) | 222 → 134 (1.66×) | seq_read | 274 → 138 (1.98×) |
| gather_line (15) | 188 → 105 (1.78×) | merge_path | 303 → 175 (1.73×) |
| hashbrown_probe (15) | 95 → 62 (1.53×) | merge_path | 304 → 197 (1.54×) |
| hashbrown_probe (15) | 94 → 56 (1.67×) | partition_pass | 162 → 123 (1.31×) |
| merge_u32 (15) | 88 → 83 (1.06×) | gather_line | 185 → 184 (1.01×) |
| gather_line (5) | 95 → 85 (1.12×) | gather_line | 188 → 185 (1.01×) |
| gather_line (10) | 169 → 139 (1.22×) | gather_line | 185 → 181 (1.02×) |

The memory pool is ~290 GB/s shared; two streaming jobs just split it (overlap
gains nothing). A CPU job at ≤ ~100 GB/s next to a GPU gather costs both ≈ nothing.

## Initial model results (historical, superseded)

Workload: semi-naive rounds with δ rows each, ρ probes per δ row, rows absorbed
into the index every round; level-0 capacity 64 K rows; 8 B rows unless noted.

These initial estimates predate both the footprint-sweep revision and the
build/probe ordering correction below. In particular, the bulk row priced probes
against an empty index; its hash winner and 12.5× payoff are not valid comparisons.
Use the corrected level-aware grid for current structure decisions.

| scenario | best @gpu | best @cpu (15 thr) | best hybrid (cpu probe / gpu merge) | GPU payoff vs CPU |
|---|---|---|---|---:|
| bulk build 15 M + 60 M probes | hash in-place **10** | sorted+dir leveled 126 | hash 47 | 12.5× |
| bulk-ish Datalog, 37 M rows / 33 rounds | sorted+dir leveled **66** (hash 82) | sorted+dir leveled 204 | sorted+dir leveled 113 | 3.1× |
| … + 800 tail rounds × 1 000 rows | hash 297 (219 of it launch) | sorted+dir leveled **232** | hash 229 | 1.0× |
| pure tail: 10 M + 2 000 × 2 000, ρ=3 | hash 545 (526 launch) | sorted+dir leveled **160** | hash 314 | 0.5× |
| wide 56 B rows, bulk-ish | hash **163** | hash 367 | hash 195 | 2.3× |
| wide 56 B rows + tail | hash 380 | hash 376 | hash **311** | 1.2× |

Reading:
1. **GPU parallel indexing pays off in proportion to bytes merged per launch.** Bulk
   rounds: 3–12× over the CPU; the sorted+directory leveled index beats the in-place
   hash on the GPU once rounds repeat (66 vs 82 ms) because merge runs at 308 GB/s
   while random RMW inserts run at 1.35 ns/line.
2. **The long tail is decided by the 128 µs command-buffer round trip, not by the
   structure.** At 2 000 rounds the GPU spends 0.5 s in launches for 20 ms of work;
   the CPU wins by 3× with the same structure. Batching rounds into one command
   buffer (GPU-driven fixpoint, indirect dispatch) is worth more than any index choice.
3. **Probes are a wash between processors**: CPU(15) and GPU both gather random lines
   at ~0.68 ns. So "CPU probes while GPU merges" buys nothing on probe speed and loses
   ~1.5× on both sides when both stream; it only helps when the merge is large and the
   CPU probe set is cache-resident.
4. **Wide rows push toward in-place/positional**: merging 56 B rows multiplies merge
   bytes 7×; the model's positional-payload variant and the in-place hash win there.
   Clustered payload should be a per-relation choice.
5. The 1-line packed directory is essential: the 2-line SoA directory costs
   +50–90 % on every probe-dominated scenario.

## Caveats

- Primitives are measured; compositions are arithmetic. No end-to-end structure has
  been built and timed on this machine yet — the next validation step is one leveled
  merge build and one probe batch (Crystal-style predicted vs measured).
- The in-place hash model assumes 1.5 lines/probe, 2 CAS stages on Metal (no 64-bit
  CAS), no duplicate-key contention, amortized doubling; real tables with Datalog's
  duplicate-heavy keys will do worse.
- Synthetic round shapes; the cvc5/oracle traces live on the CUDA host and have not
  been extracted yet (`model.py --trace` accepts `[{delta, probes, touched}]`).
- Hybrid solos are only reliable with footprints ≤ 1 GiB in one process (24 GB box);
  the 18-thread rows in `results/*-debug-18thr.*` are oversubscription artifacts.

## Level-aware revision (footprint sweeps, `results/*-sweep-*.json`)

13 footprints from 256 KiB to 4 GiB per primitive. The model now builds a monotone,
log-interpolated cost curve per (primitive, processor, threads) instead of picking the
nearest of four buckets. What the sweeps show:

| level signature | CPU (1 thread dependent latency) | GPU (random line, ns) |
|---|---:|---:|
| ≤ 8 MiB | 8 ns (L2) | 0.06–0.08 |
| 16 MiB | 24 ns | 0.11 |
| 32 MiB | 67 ns | 0.17 |
| 64–128 MiB | 105–110 ns | 0.27–0.31 |
| 256–512 MiB | 111–113 ns | 0.40–0.61 |
| ≥ 1 GiB | 114–119 ns | 0.73–0.76 |

CPU random-line throughput at 15 threads reaches 0.07 ns/line in L2, 0.5–0.6 at
64–256 MiB, 0.68 at ≥ 1 GiB (= GPU). The GPU's gradual rise between 16 MiB and
512 MiB is not one cache edge; it is treated as an empirical curve, not fitted to a
capacity. Tiny-footprint/many-thread cells are overhead-dominated and are flattened by
the monotonicity rule (overheads are modeled separately).

Grid with level-aware curves and the extra workloads (ms, best structure per placement):

| scenario | @gpu | @cpu | hybrid | GPU payoff |
|---|---|---|---|---:|
| bulk build 15 M + 60 M probes | leveled **25** | leveled 77 | leveled 40 | 3.0× |
| TPC-H SF10 chain, 5 builds probed by 60 M | leveled **96** | leveled 304 | leveled 114 | 3.2× |
| bulk-ish Datalog 37 M / 33 rounds | leveled **60** | leveled 165 | leveled 71 | 2.8× |
| TC 1 M nodes, 200 M pairs, 30 rounds | leveled **424** | hash 952 | leveled 423 | 2.3× |
| TC 100 k nodes, 200 M pairs, 12 rounds | leveled **309** | leveled 872 | leveled 312 | 2.8× |
| SG-like 20 M pairs, 60 rounds | leveled **53** | hash 88 | leveled 54 | 1.7× |
| wide 56 B rows bulk-ish | hash **155** | hash 215 | hash 181 | 1.4× |
| + 800-round tail | leveled 292 | leveled **187** | blocked 202 | 0.9× |
| doop-like 30 M rows, 400 rounds, 6 probes, 24 B | hash 157 | hash **114** | hash 145 | 0.8× |
| doop-like long tail (10 M rows, 400 rounds) | hash 111 | hash **19** | hash 66 | 0.3× |
| pure tail 2 000 × 2 000 | hash 543 | hash **83** | hash 312 | 0.3× |

The level-aware curves lowered the CPU side substantially (tail inserts and small
deltas live in L2/SLC). With build-before-probe ordering corrected, GPU payoff is
about 2–3× where rounds are few and big (bulk joins, TC/SG), ≤ 1× once the round count
passes a few hundred, regardless of structure, because the 128 µs command-buffer
floor dominates.

## Build/probe dependencies and structure decisions

`Round.build_first` makes the index snapshot used by a probe explicit:

| workload | probe snapshot | same-round CPU probe / GPU maintenance overlap |
|---|---|---|
| semi-naive (`build_first=false`, default) | previous rounds' rows and live runs | permitted by the existing overlap model |
| bulk build/probe (`build_first=true`) | rows and live runs after this build | forbidden: probe depends on build completion |
| independent join chain | fresh index after each build (`reset=true`, `build_first=true`) | forbidden within each pair |

The default preserves existing Datalog traces. JSON traces may set `build_first`
and `reset` per round; both default to false. Maintenance always receives the
pre-build row count, and each delta is counted once. Build-first probes use the
post-build footprint **and** the resulting run count, not just a larger byte count.
`--overlap` cannot override this dependency.

This changes the bulk structure choice: with the measured footprint sweeps,
15 M rows plus 60 M probes selects sorted+directory leveled rather than hash.
Its GPU probe estimate is 19.18 ms, versus the incorrect empty-index 3.82 ms.
The TPC-H chain likewise selects leveled for all three placements. These are
corrected model predictions, not measurements of an implemented index.

For the data-structure design, each searchable run needs a published directory
matching its row storage. Build-first execution must finish publishing the new
run set before probing it. Overlapping semi-naive execution instead requires
readers to retain the old run set while maintenance prepares its replacement;
in-place mutation does not automatically provide that isolation. The current
overlap arithmetic does not model snapshot retention or its memory cost.
The default scenario grid keeps overlap disabled.

Reproduce the current grid from the repository root (exclude debug/oversubscribed
calibrations):

```sh
python3 experiments/mac/model/analyze.py \
  experiments/mac/calib/results/cpu-2026-10-23.json \
  experiments/mac/calib/results/gpu-2026-10-23.json \
  experiments/mac/calib/results/cpu-sweep-2026-10-23.json \
  experiments/mac/calib/results/gpu-sweep-2026-10-23.json \
  experiments/mac/calib/results/hybrid-2026-10-23.json
python3 -m unittest discover -s experiments/mac/model -p 'test_*.py' -v
```

## Native sorted/hash artifact benchmark

`index-bench/run.py` fetches exact revisions from `index-bench/artifacts.json`,
builds with native Clang C++20 optimization, checks correctness, and records CSV
measurements plus compiler, hardware, arguments, artifact revisions, and source
hashes. Checkouts and binaries remain under ignored `target/`; upstream files are
not vendored or modified.

### Artifact choices and inspection findings

| candidate | source and configuration |
|---|---|
| PGM / PGM++ | [author artifact](https://github.com/qyliu-hkust/bench_search), original branchy epsilon64 and branchless epsilon16/64; recursive epsilon4; PGM++ internal linear threshold8 |
| RadixSpline | [author implementation](https://github.com/learnedsystems/RadixSpline), radix12/18, error32 |
| Compact Hist-Tree | [Mihail Stoian implementation](https://github.com/stoianmihail/CHT), not the original paper artifact; offline fanout64, terminal32/128 |
| Dense hash | [unordered_dense](https://github.com/martinus/unordered_dense), reserved SIMD-group hash map, mapping key to group rank |
| Simple baselines | libc++ `std::lower_bound`, artifact branchless search, reserved `std::unordered_map` with SplitMix64 hashing |
| SIMD blocks | Local NEON full-key leaf scans over 16/64-key blocks, with a binary-searched fence directory; **not LiBox** |
| Bloom | Optional local 64-byte-block, eight-bit-probe Bloom filter at approximately 12 bits/key, wrapping CHT and dense hash; **not a filter-paper reproduction** |

- The PGM artifact performs floating-to-integer conversion before capping a
  prediction at the next segment. Sanitizers found overflow on large-gap inputs.
  `pgm-safe-projection.patch` saturates conversion and addition in a generated
  header copy. All PGM results carry `_safe_projection`; this is not an unmodified
  artifact reproduction. The original and PGM++ variants receive the same fix.
- PGM reserves `UINT64_MAX`; the adapter retains that key as an explicit final
  rank. Empty/singleton inputs are handled without invoking undefined zero-span
  operations in upstream builders.
- CHT has inclusive-span edge restrictions. A power-of-two span with positive
  minimum uses a declared minimum lowered by one, without changing keys or ranks.
  Nontrivial spans ≤32, power-of-two spans starting at zero, and unsupported rank
  widths are explicitly rejected/reported, **not replaced by another algorithm**.
- LiBox was fetched and inspected, but its unconditional x86 AVX-512/OpenMP code
  is not benchmarked on ARM. No scalar substitute is presented as LiBox.

### Workload and timing contract

All candidates see identical sorted distinct 64-bit keys, contiguous payload
groups, and shuffled probes. Keys are uniform over a broad numeric domain or
clustered into 64 separated ranges. Absent probes are odd keys adjacent to stored
even keys, so misses occur within the occupied regions rather than being
artificially easy out-of-domain rejects. Group lengths vary around the requested
fanout. Fixed seeds keep data comparable across hit-rate configurations.

| mode | timed work |
|---|---|
| `lookup` | Exact key-to-group-rank lookup, hits and rank checksum |
| `expand` | Lookup followed immediately by enumeration of every matching payload |
| `retained` | Store exact group ranks, then enumerate their payloads without re-searching |
| `sorted_*` | Corresponding mode over sorted probes; probe sorting reported separately and added once to `total_ms` |
| `merge` | Sequential merge of sorted probes with sorted distinct keys, enumerating matching payload groups |

`retained` isolates one LE-style lookup/expand kernel, not a full multiway LE or
Yannakakis plan: there is no intervening selective rule predicate. Repeated probe
keys retain their full join multiplicity. Payload enumeration feeds a count and
checksum sink; it is **not full output-tuple materialization**.

Each repetition constructs the index and warms each query mode before timing.
Input generation, correctness checks, and buffer allocation are untimed.
`build_ms` is CPU auxiliary-index construction **from already sorted keys**,
including Bloom construction when enabled, not GPU construction or a complete
unsorted-relation build. `total_ms` is query plus in-place probe sorting, not build
plus query. Copying probes into the sort buffer is excluded; its memory is
reported. Sorted-data advantages must therefore be considered both with and
without the separately visible sorting charge.

Memory columns distinguish shared keys/offsets/payload from auxiliary index,
probe, and witness buffers. Sorted adapters report logical auxiliary bytes;
dense hash reports allocated index/value capacity; standard hash is an explicit
node/bucket estimate. These are not comparable peak-RSS or construction-workspace
measurements. Filter false positives are measured on each actual probe workload.

Every candidate is checked against independent `std::lower_bound` ranks outside
timing; immediate, retained, and merge outputs must agree in count and payload
checksum. The self-test covers empty/singleton inputs, zero and maximum keys,
internal/boundary misses, duplicate probes/groups, and explicit artifact rejection.

From the repository root:

```sh
python3 experiments/mac/index-bench/run.py --sanitize
python3 experiments/mac/index-bench/run.py --quick
python3 experiments/mac/index-bench/run.py --reps 5 --output /path/to/new-results-directory
```

The full suite includes cache-sized and larger indexes, uniform/clustered keys,
10/50/90% hit rates, and narrow/wider match groups. It is a single-thread CPU
baseline, not a CPU saturation test, GPU-build benchmark, or heterogeneous
end-to-end Datalog result. Existing cost-model coefficients are intentionally
unchanged.

### First M3 Max results

[Raw measurements and provenance](index-bench/results/m3max-2026-10-23/)
contain 637 rows: seven workloads, fifteen index/filter variants with six query
modes each, and one merge baseline per workload. Five repetitions, one CPU thread,
warm query mode; all timed results passed rank/count/checksum checks.
ASan/UBSan self-tests pass with non-recovering diagnostics after the recorded PGM
fix. No candidate was unsupported on these seven generated workloads.

Representative workload: **1,048,576 distinct uniform keys, 262,144 probes,
50% hits, approximately four payloads/key**. Payload enumeration uses the
checksum sink described above. Build time is excluded from these query columns.

| structure | random lookup ns/probe | random lookup+expand ms | random retained+expand ms | sorted lookup+expand ms, including sort |
|---|---:|---:|---:|---:|
| `std::lower_bound` | 72.21 | 32.63 | 23.06 | 20.06 |
| artifact branchless binary search | 43.74 | 28.37 | 15.29 | 16.75 |
| CHT fanout64 / terminal32 | 22.16 | 12.42 | 9.31 | 10.18 |
| RadixSpline radix12 / error32 | 19.99 | 14.47 | 9.03 | 12.12 |
| PGM original epsilon64, safety patch | 36.35 | 21.18 | 12.96 | 13.21 |
| PGM++ epsilon64, safety patch | 23.31 | 17.59 | 9.80 | 12.35 |
| local NEON fence blocks16 | 65.28 | 22.70 | 20.57 | 14.39 |
| reserved standard hash map | 28.37 | 12.36 | 12.05 | 12.17 |
| reserved dense hash map | 9.32 | 8.65 | 6.68 | 8.22 |

Sorted merge enumerates this workload in **2.75 ms** once probes are ordered,
or **6.76 ms including sorting**: approximately tied with retained dense hash,
not an unqualified win. For 90% hits, sort-merge including sorting takes
**6.84 ms**, versus **7.74 ms** for retained dense hash. At roughly 32 payloads/key
and 50% hits, those totals are **10.42 ms** and **11.85 ms**, respectively.
Conversely, at 10% hits/fanout4 retained dense hash takes **2.14 ms**, versus
**5.96 ms** for sort-merge including sorting.

Interpretation:

- **A fast hash baseline matters.** Dense hash wins random lookup in all seven
  workloads. Beating standard `unordered_map` alone would overstate the sorted
  indexes' competitiveness.
- **The directory is not the whole join.** RadixSpline wins sorted-structure
  random lookup in the representative workload, but CHT wins immediate expansion.
  Retained processing improves both and also improves hashing. Sorting helps
  hash expansion too; its output access follows the same clustered groups.
- **Space and tuning differ sharply.** In that workload RadixSpline radix12 uses
  about 30 KiB logical auxiliary storage, CHT about 1.02 MiB, and dense hash
  about 27 MiB allocated auxiliary storage. Increasing RadixSpline radix bits
  to18 grows its directory to about 1.01 MiB while lookup slows to 21.59 ns.
  These comparisons exclude common keys/payload and use the accounting
  distinctions above; they are not RSS measurements.
- **Bloom is optional.** At 10% hits on the 1M-key workload the local filter
  changes CHT random lookup from 5.27 to5.34 ms and dense hash from 1.24 to4.76 ms.
  At 4M clustered keys/50% hits it helps CHT, from 21.28 to18.21 ms, but still
  slows dense hash, from 4.06 to8.38 ms. The measured filter FPR is around
  0.5–0.6% in these cases. This does not establish the best blocked-filter kernel.
- **SIMD alone is insufficient.** The local fence/NEON implementation does not
  approach the optimized directory candidates. These results do not establish
  a SIMD-search ceiling or justify discarding other blocked layouts.

For the cost model, structure and processing mode must be separate choices:
random/ordered probes, retained witnesses, merge traversal, and preprocessing
reuse. Keep the measured CPU directory costs separate from the still-unmeasured
GPU construction of those same directories. These runs do not show that
CPU-only hash construction is the best heterogeneous design.

### Native backend RadixSpline provider and processing advice

`cq_storage::RadixRun<K,V>` is a native Rust RadixSpline provider, not the old
`SortedRun` search algorithm and not a per-lookup C++ FFI wrapper. It uses the
measured radix12/error32 family, with an extra conservative floating-point
rounding allowance. Only grouping and Copy payload-range cursors are shared with
the other run provider. Payload lookup has no hash/full-array binary-search
fallback. Dense-domain membership can lazily project the sorted keys into an
exact presence bitmap: at most 128 KiB and one byte per distinct key, including
word rounding. Sparse membership retains spline search; payload-only use never
allocates the bitmap. This is a membership side index, not a replacement for the
payload directory.

Select it through the existing checked Structures advice mechanism:

```rust
pipeline {
    structures = [
        cq_rewrites::radix_runs,
        cq_rewrites::retain_lookups,
    ];
}
```

`radix_runs` selects eligible explicitly built singleton i32/i64/u32/u64 payload
indexes, including borrowed and computed/owned keys. Composite/non-integral
keys, host fallbacks, membership-only providers, multi-level tries, and
smallest-intersection candidates remain unchanged. Default hash selection is
unchanged. The prior `sorted_runs` advice still means the old binary/rank provider;
it is not an alias for RadixRun.

Processing is a separate checked decision:

- `radix_runs` alone uses immediate exact lookup/expansion.
- Add `retain_lookups` to carry an exact range from an eligible eager semijoin
  to its later matching probe without repeating the directory search.
- Optionally add `expand_lookups(32)` for segmented traversal of length-known
  ranges. Compatibility is established, **not a universal speedup**.
- The Sequential catalog exposes Exact/Range/Membership, not Ordered or GPU
  capability. Sorted storage does not establish that the probe input is ordered;
  no merge join is selected implicitly.

Native provider validation covers full-width integer extrema, empty/singleton
relations, irregular gaps, misses, duplicate occurrences and stable payload
order. The runnable backend example produces the same 150 exact answers through
default hash, RadixRun immediate, retained, staged, and retained+staged routes:

```sh
cargo run --manifest-path features/rewrites/Cargo.toml --example radix_runs
cargo test --manifest-path features/rewrites/Cargo.toml
cargo test -p cq-storage -p cq-lang-structures -p cq-sequence-pull --all-features
cargo run --release --locked --manifest-path experiments/mac/index-bench/native/Cargo.toml -- 1048576 262144 5
```

### Native Rust provider measurements

[CSV](index-bench/native/results/m3max-2026-10-24.csv) and
[provenance](index-bench/native/results/m3max-2026-10-24.json):
M3 Max, one CPU thread, 1,048,576 keys, 262,144 probes, 50% hits, variable two-to-six
payload occurrences/key, five warm repetitions. All modes agree on 524,228
enumerated occurrences and their checksum. These are native Rust runtime
measurements, **not the earlier C++ workload or timings**.

| provider | membership ns/probe | immediate expand ms | retained expand ms | segmented32 ms |
|---|---:|---:|---:|---:|
| RadixRun radix12/error32 | 15.63 | 13.69 | 10.41 | 17.36 |
| old SortedRun, binary directory | 39.04 | 26.68 | 21.67 | 33.71 |
| backend HashIndex, AHash | 7.72 | 12.23 | 11.30 | not applicable |
| hashbrown 0.15.5, foldhash, group-rank directory | 6.95 | 6.58 | 4.17 | not applicable |

RadixRun improves over the old binary provider, but it does not beat the strongest
hash baseline on this random workload. Retaining its ranges reduces enumeration
time by about 24%; segmented32 is slower, so it remains independent opt-in advice.
This supports a cost-model choice of **provider × processing mode**, not an
unconditional sorted-processing recipe.

With already ordered probes, RadixRun immediate expansion is 7.12 ms. In-place
probe sorting costs another 2.16 ms (copying the sort buffer is excluded).
Sequential merge over the same pre-existing grouped payload takes 2.91 ms before
that sorting charge; it is measured as a separate algorithm, not wired into the
advice as a silent reorder.

Native builds include stable payload grouping for runtime providers; the standalone
hashbrown baseline builds only a key-to-rank map over pre-existing payload/offset
arrays. Its construction time therefore is not a like-for-like build comparison.
Build columns are single observations; query columns are medians. Checksums are
output sinks, not full tuple materialization. CPU/GPU overlap and GPU construction
of the RadixSpline directory remain unmeasured; the earlier M5 cost coefficients
are intentionally unchanged.

## TPC-H-derived hash versus RadixRun comparison

This first full-suite record predates the profiler optimizations. The
[broader optimized comparison](#broader-optimized-cpu-comparison) below uses the
current runtime and remote H19 interning; keep the two source snapshots separate.

[Complete table](index-bench/results/tpch-m3max-2026-10-24/comparison.tsv) and
[raw samples/provenance](index-bench/results/tpch-m3max-2026-10-24/comparison.json)
come from one final complete invocation on the M3 Max: **SF1, 29 kernels,
three configurations, five measured executions after one warm execution**.
All 87 configurations pass exact DuckDB v1.5.5 checks at SF0.1 and SF1, including
every timed result; separate counter executions agree. Query times include fresh
index construction and typed output collection, but exclude loading, formatting,
answer comparison and result destruction. No GPU participates.

These are the repository's derived/canonical kernels, not a standard 22-query
TPC-H score. The ordinary comparison preserves each query's default Physical
plan: `radix` only changes eligible provider types; `radix_retained` additionally
requests compatible lookup reuse. It does not select a different LE plan.

Across all 29 kernels, the geometric mean of Radix/default time is **1.122×**.
Across the **15 kernels whose counted hash work changes**, it is **1.247×**.
The latter is a more informative subset for provider costs; unchanged controls
must not be interpreted as evidence about a structure switch. Retained/radix
time on that same subset is **1.003×** geometrically: no consistent benefit in
these particular default plans. Five-run medians are descriptive, not confidence
bounds or a claim that tiny percentage differences are significant.

| kernel | default hash-oriented ms | Radix advice ms | Radix/default |
|---|---:|---:|---:|
| H02 | 37.38 | 53.47 | 1.43× |
| H03 | 28.46 | 50.55 | 1.78× |
| H05 | 28.45 | 45.65 | 1.60× |
| H07 | 25.04 | 30.62 | 1.22× |
| H07-block | 56.32 | 87.71 | 1.56× |
| H08 | 25.92 | 46.01 | 1.78× |
| H10 | 152.42 | 148.19 | 0.97× |
| H11 | 12.81 | 13.29 | 1.04× |
| H20 | 140.14 | 139.95 | 1.00× |
| H21-positive | 133.24 | 139.64 | 1.05× |
| D19 | 16.50 | 17.59 | 1.07× |
| D27 | 12.76 | 19.92 | 1.56× |
| D72-inner | 97.45 | 99.22 | 1.02× |
| D85 | 19.28 | 21.36 | 1.11× |
| Q19 | 14.72 | 14.83 | 1.01× |

### H09: order choice and a real provider switch

The old written order put Orders immediately after filtered Part, before their
linking Lineitem relation. The compiler emitted an empty-key probe. Local data
has 10,664 green Parts and 1,500,000 Orders: **15,996,000,000 candidate pairs**.
Boolean short-circuit or disjunction support cannot fix that disconnected prefix.

[Exact cardinality queries and all 18 ranked orders](index-bench/results/tpch-m3max-2026-10-24/h09-cardinalities.json)
keep the opaque Part predicate first. The smallest sum of prefix rows is
`Part → PartSupp → Supplier → Nation → Lineitem → Orders` (777,440).
The measured alternative `Part → Lineitem → PartSupp → Orders → Supplier → Nation`
has more prefix rows (1,607,684), but a narrower large Lineitem index and lower
runtime. The authored body retains that second order; SQL and predicate domain
are unchanged. This is the best of the two measured index-shape candidates, not
a globally proven runtime optimum.

| H09 setup | provider policy | median ms |
|---|---|---:|
| original disconnected order | default host fallbacks | 389,345.45 |
| minimum-prefix-row candidate | default host fallbacks | 669.81 |
| selected single-key-Lineitem candidate | default host fallbacks | 466.84 |
| selected order + explicit host materialization | `built_hash` | 479.73 |
| same order and materialization | `built_radix` | 863.55 |

The first three rows are preserved order-selection trials, not the final full
suite's H09 samples. The last two form the actual provider comparison:
**RadixRun costs 1.80× hash time** here, including construction.
[Original order](index-bench/results/tpch-m3max-2026-10-24/h09-original-written.json),
[minimum-prefix candidate](index-bench/results/tpch-m3max-2026-10-24/h09-min-prefix-comparison.json),
[selected candidate](index-bench/results/tpch-m3max-2026-10-24/h09-single-key-comparison.json),
and [materialization-matched provider results](index-bench/results/tpch-m3max-2026-10-24/h09-materialized-comparison.json)
retain the evidence.

Why the extra pair is necessary: ordinary H09 Structures have zero Built entries,
five keyed HashIndex host reads, and no retained witnesses. Ordinary radix advice
does not rewrite host fallbacks, so its H09 timing is a control, not a RadixRun
measurement. Both `built_*` variants first apply `build_host_reads`; their Physical
IR is equal, and a regression test requires an actual RadixRun in `built_radix`.
Composite keys remain hashed in both.

Reproduce from the repository root:

```sh
cargo run --manifest-path features/workloads/xtask/Cargo.toml --features oracle -- \
  bench --tpch --sf 1 --runs 5 --index-comparison
cargo run --manifest-path features/workloads/xtask/Cargo.toml --features oracle -- \
  bench --tpch --sf 1 --runs 5 --query retail_h09 --config built_hash
cargo run --manifest-path features/workloads/xtask/Cargo.toml --features oracle -- \
  bench --tpch --sf 1 --runs 5 --query retail_h09 --config built_radix
```

Legacy retained-byte and direct-range row counters do not cover all RadixRun
storage/iteration, so their values are not full memory-footprint or traffic
comparisons. The cost model still needs separately measured GPU construction and
CPU/GPU contention; these CPU end-to-end results do not establish heterogeneous
speedup.

### Profiler-guided CPU optimization

[Raw profiles, samples, and provenance](index-bench/results/profiling-m3max-2026-10-24/comparison.json)
record a targeted SF1 follow-up, not a replacement for the full 29-kernel table
above. Medians below use 11 warm, uninstrumented executions on the same M3 Max;
construction and typed result collection remain inside the timing.

| Kernel/configuration | Before (ms) | Final (ms) | Before/final |
|---|---:|---:|---:|
| H03 default hash | 27.796 | 28.635 | 0.971 |
| H03 RadixRun | 47.842 | 28.678 | 1.668 |
| H09 built_hash | 458.770 | 474.287 | 0.967 |
| H09 built_radix | 865.677 | 493.161 | 1.755 |

The matched hash controls drifted by about 3%; the optimized sorted providers
close most of the gap, but do not establish a win over hash.

Operator profiles, macOS `sample`, expanded Rust, and native disassembly exposed
two avoidable costs:

- H09 sorts `((&i64,), (&i64, &i64))` tuples. Repeated borrowed-key comparisons
  dominated construction. Cache each order-preserving numeric key for the stable
  sort, preserving duplicate occurrence order. Small, sorted, and strictly
  reverse-ordered inputs avoid the auxiliary key array. The Lineitem index's
  instrumented exclusive build fell from 713.043 to 325.653 ms.
- H03's generated semijoin calls `ExactMembership::contains`, not payload
  enumeration. Its dense Customer domain now uses the bounded lazy presence
  bitmap described above. The instrumented semijoin span fell from 28.650 to
  8.804 ms; that span includes its input scan and is not pure lookup time.

[Compiled-code excerpts and native stacks](index-bench/results/profiling-m3max-2026-10-24/compiled-code-evidence.json)
preserve the evidence. Instrumentation overhead in the final runs was
14.9–21.6%, so only plain-worker times establish end-to-end speedups.

A tempting assembly-level change was rejected: replacing ordinary `i128` rank
conversion with checked native-width conversion slowed an alternating
conversion-only experiment from 14.657 to 15.459 ms. The original conversion
remains. [Paired ablations](index-bench/results/profiling-m3max-2026-10-24/ablations.json)
also checked sorted-input grouping: 7.130 ms for the standard sort path versus
7.042 ms for the new ordered fast path, with exact array equality.

The native sparse-key control uses no bitmap. It preserved every mode's answer
count/checksum, but did not show a general probe improvement: membership changed
from 3.993 to 4.276 ms and immediate enumeration from 13.260 to 14.930 ms; some
unchanged controls also shifted. Its one cold full-build observation changed
from 29.361 to 34.556 ms, not a repeated construction median. These results are
workload-specific, not evidence of saturated sorted-index performance.

All four final TPC-H profiles matched DuckDB exactly. Regression coverage checks
borrowed duplicate order, sparse/full-width keys, concurrent lazy initialization,
and live payload cursors; the executable immediate/retained/staged routes still
produce 150 identical answers. The lazy cache changes the compiler-internal
`Freeze` auto-trait, not the provider's public methods or Send/Sync bounds.

Reproduce either targeted profile with a separate writable output directory:

```sh
cargo run --manifest-path features/workloads/xtask/Cargo.toml --features oracle -- \
  profile --tpch --sf 1 --query retail_h09 --config built_radix \
  --output "$PWD/target/mac-profile/reproduction"
```

The cost model must distinguish sorting key ownership/order, membership-only
versus payload reads, and the bounded projection's first-use construction cost.
The bitmap is not a Bloom filter. No GPU construction, overlapping CPU/GPU work,
or shared-bandwidth contention is measured by this CPU optimization.

### Broader optimized CPU comparison

After merging remote `4cbe5eea` (including CPU H19 string interning), the complete
catalog ran on the same M3 Max: **29 kernels, 121 configurations, 605 timed
executions**. Every configuration passed exact DuckDB checks at SF0.1 before
timing, then at SF1 for warmup, all five measured executions, and both independent
counter runs. All compilation preceded timing; execution was sequential.

[Full timings and samples](index-bench/results/tpch-m3max-2026-10-24-optimized/comparison.json),
[the complete timing table](index-bench/results/tpch-m3max-2026-10-24-optimized/comparison.tsv),
and [provider-aware pairs](index-bench/results/tpch-m3max-2026-10-24-optimized/provider-comparison.tsv)
preserve all results. This is the 27 authored CQ-shaped kernels plus canonical
Q6/Q19, not all 22 full standard TPC-H queries. Loading and H19 dictionary
preparation are outside the query timer; query-time construction is included.

Concrete constructor sites were counted with `syn` over all 29 rustc-expanded
workers. Every pair with emitted RadixRun builds also reduced measured hash
computations; every no-op pair had zero RadixRun builds and equal hash counts.
The two families overlap: **17 unique kernels actually switch providers**.

| Comparison family | Actual provider switches | Geomean Radix/hash | >10% faster | Within ±10% | >10% slower |
|---|---:|---:|---:|---:|---:|
| Default plan: radix versus default | 15 | 1.159 | 0 | 7 | 8 |
| Explicit materialization: built_radix versus built_hash | 13 | 1.139 | 1 | 5 | 7 |

Ratios below one favor RadixRun. Geometric means weight each query equally
within its family; ±10% is a descriptive band, not a statistical equivalence
test. The 14 ordinary and four materialized no-op pairs are excluded from these
aggregates. Their apparent parity is not sorted-index evidence.

Representative same-plan results, in milliseconds:

| Kernel/family | Hash | RadixRun | Radix/hash |
|---|---:|---:|---:|
| H03 ordinary | 28.765 | 28.511 | 0.991 |
| H09 materialized | 490.757 | 489.223 | 0.997 |
| H16 materialized | 18.659 | 11.256 | 0.603 |
| H05 ordinary | 27.545 | 38.882 | 1.412 |
| H07-block ordinary | 55.001 | 92.643 | 1.684 |
| D27 ordinary | 12.824 | 19.999 | 1.560 |

Retained-range advice does not close the aggregate gap: its geomean versus
default is 1.164 on the same 15 changed-provider kernels. It remains a separate
processing choice, not an automatic requirement for sorted storage.

The broader conclusion is narrower than universal hash parity: sorted storage
can match or beat hash on particular plans, but is still about 14–16% slower
geometrically on these changed-provider families, with substantial outliers.
Keep provider/processing selection workload-sensitive. End-to-end totals alone
do not identify whether each remaining loss is construction or probing; these
results do not establish GPU speedup or sorted-index saturation.

Reproduce with `cargo xtask bench --tpch --sf 1 --runs 5 --index-comparison`
from the workloads workspace. The command now includes the materialized pair
where advertised, alongside default/radix/radix_retained everywhere.

### Explicit predicates and transfer diagnosis

The 2026-10-25 workload update expresses H05, H07-block, D27, and H08 string
restrictions as named equality predicates. SQL and answer heads are unchanged.
This deliberately uses predicate-read semantics instead of literal-bearing
Rename retention; it does not change the compiler's meaning of literal binding.
Earlier tables above retain their original source snapshots.

[Fresh literal/predicate controls and all configurations](index-bench/results/predicate-transfer-m3max-2026-10-25/matrix.tsv),
[raw samples and operator profiles](index-bench/results/predicate-transfer-m3max-2026-10-25/results.json),
and [diagnosis/provenance](index-bench/results/predicate-transfer-m3max-2026-10-25/summary.json)
cover 50 configurations checked exactly at SF0.1 and SF1. The following CPU
times are SF1 medians of five plain executions, with construction included:

| Predicate-form kernel | Default (ms) | CQ Transfer (ms) | Transfer + checked absorption, diagnostic only (ms) |
|---|---:|---:|---:|
| H05 | 27.926 | 35.001 | 27.408 |
| H07-block | 26.565 | 45.071 | 25.408 |
| D27 | 14.741 | 24.218 | 14.691 |
| H08 | 22.271 | 27.446 | 22.270 |

The temporary absorption configuration applies the existing checked
`cq_elaborate::simplify_reductions` after Transfer and verifies CQ correspondence.
Production Transfer currently does not take that path. The ablation removes
backward support versions, restores useful scan filters, and avoids retaining
fact intermediates needed only by those versions. No compiler fix was applied.
Some absorbed plans still construct the same scan/build/key filter twice for
different covering operators; those costs remain visible.

D27 exposes a separate lowering/runtime pathology. Its literal-form transfer
streams the fact filters into one final 357-row collection. The predicate form
instead constructs successive positional bitmap snapshots, including snapshots
for intermediate Physical pipes. These slice hosts expose one block spanning all
6,001,215 Lineitem rows. `Selection::narrow` visits every position in that block
even with only 9,225 selected inputs. A matched enumeration-only diagnostic
measured 3.079 ms for that traversal versus 0.103 ms using the existing
bitmap-word row enumeration; both preserved occurrence counts and ID checksums.
That comparison excludes output-mask construction and is not a shipped speedup.

These results diagnose this implementation, not unavoidable YA/PT overhead.
The measured CQ Transfer is distinct from the paper's vectorized Bloom pipeline
and from this repository's Sequence lookup-first family. The
[RPT implementation](https://arxiv.org/html/2502.15181v1#S4.SS3) explicitly prunes
unnecessary transfer work; [RPT+](https://people.iiis.tsinghua.edu.cn/~huanchen/publications/rpt+-vldb26.pdf)
adds adaptive construction/probe cancellation. Default here already includes
Masks reductions. Full paper robustness or non-regression claims should not
be attributed to the benchmark's configuration name.

## GPU experiment integration and Mac implementation boundary

The standalone GPU experiment, CQ Mac runtime and MCCL native implementation are
consolidated here without replacing the join-optimized runtime/compiler contracts.
The separate `mini_linq-gpu-index-20261024` and MCCL snapshots are historical
backups, not active work directories. Archived measurements retain their original
provenance. The earlier experiment import passed 149 targeted
storage/Structures/Sequential tests, the five-route RadixRun example (150 exact
answers per route), and real Metal sort/merge/grouping/directory/lookup checks;
these are historical checks, not verification of the current consolidation.

The offline benchmark histories are not themselves an online maintained provider.
The separate [`cq-mac/`](cq-mac/src/lib.rs) runtime now implements selectable
flat compaction and binary carries, pinned immutable snapshots, all-live-run reads,
publication and ownership-based buffer retirement. It also exposes explicitly
selected GPU-built grouped RadixRun with the native CPU reader. The removed legacy
`LeveledDeviceIndex` backend remains unavailable.

See the [implementation plan](../../doc/MAC-IMPLEMENTATION.md) for contracts,
current root-relative commands and the inventory of existing versus proposed
compiler changes. This consolidation adds no new core-compiler behavior.
