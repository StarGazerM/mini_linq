# Mac (Apple silicon) calibration and cost model

Branch `mac-cost-model` (worktree of `mini_linq`). Nothing here touches the main
workspace. Two pieces:

- `calib/` — `maccal`: measured primitive costs on this machine (CPU, GPU, and
  CPU+GPU concurrently). Rust + Metal; see `calib/README.md` for every primitive.
- `model/` — trace-driven cost model over those primitives:
  `model.py` (structures × policies × placements × workload) and `analyze.py`
  (best structure per placement over a scenario grid).

Machine: Apple M5 Pro, 16-core GPU, **15 logical CPUs** (5 super + 10 perf),
24 GB unified memory, macOS 27.0.1, Metal 4. Results dated 2026-10-23.

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

## Model results (`analyze.py`, best structure per placement, ms)

Workload: semi-naive rounds with δ rows each, ρ probes per δ row, rows absorbed
into the index every round; level-0 capacity 64 K rows; 8 B rows unless noted.

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
| bulk build 15 M + 60 M probes | hash **8** | leveled 48 | leveled 11 | 6.0× |
| TPC-H SF10 chain, 5 builds probed by 60 M | hash **36** | leveled 241 | leveled 51 | 6.6× |
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
deltas live in L2/SLC) and shrank the GPU payoff: 2–7× where rounds are few and big
(bulk joins, TC/SG), ≤ 1× once the round count passes a few hundred, regardless of
structure, because the 128 µs command-buffer floor dominates.
