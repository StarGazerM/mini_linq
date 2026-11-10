# Per-batch GPU baseline, 2026-11-10 (compression batch)

Current `04776a2f` (GPU base-column layouts, STORAGE §§12.5–12.6) versus the
previous batch main `20f1f04e`. Raw samples, binaries, build times and summaries:
[baseline-2026-11-10-compression.results.json](baseline-2026-11-10-compression.results.json).

## Scope

This batch changes only `experiments/gpu` (device-columns, compress, tpch),
`experiments/storage-bench/gpu_base_codec.py`, evidence files and docs. No
root-workspace source changed (`git diff --name-only 20f1f04e 04776a2f`), so
CPU TPC-H and CPU DOOP binaries are unchanged and no CPU baseline is run. The
GPU baseline covers GPU TPC-H SF1 advised (26 queries) and `doop-gpu` xalan
(26 outputs). `srd-doop-gpu` field_flow was not run: its sources and the GPU
Datalog sources are unchanged.

## Generated code, reviewed before timing

Fresh builds of both revisions:

- `queries.rs` differs only in H05's SF1/SF10 load setup:
  `gpu_compress::{Source::load_manifest, Catalog::compress}` became
  `crate::loader::{load_source, compress_catalog}`. That load runs before the
  benchmark loop and outside query timers. The comparison normalizes build
  paths and path-derived kernel-module hashes.
- `plans.json` is equal except for `generation_seconds`.
- The default `for` layout calls the same `EncodedColumn::from_host(.., Codec::For)` as before.
- `doop`, `datalog` and `datalog-runtime` sources are unchanged. The `doop-gpu`
  binary differs only through the additive device-columns/compress code it links.

## Method

Local RTX 6000 Ada with sm_89 materialized cubins, Rust 1.99.1 and CUDA 13.0.

- **Pairs.** Each workload has one untimed warmup process per revision, then
  five interleaved fresh-process pairs (reference first in even pairs) holding
  only the GPU lock. Five CPU-exclusive confirmation pairs follow; each sample
  takes the GPU lock, then the timing gate, then the exclusive timing lock.
- **Contamination.** A sample with a foreign GPU tenant, or with 1-minute load
  above 8 before or after it, would be discarded and repeated. None were;
  observed load was 1.19–5.84.
- **TPC-H metric.** Each process reports the median of five `total_ms`. The
  first measured sample is compared fully against DuckDB and later samples by
  bench-check sampling; this is validated, not all-samples-full.
- **DOOP metric.** `GPU_DATALOG_PHASES` fixpoint seconds. Every process compares
  all 26 complete output tuple sets against doop-mini.
- **Statistics.** Per-item exhaustive 5^5 paired bootstrap; suite geomean
  resampled 20,000 times, seed 20261106.
- **Time.** Driver wall 910.86 s. Builds took 190.42 s (reference) and 145.30 s
  (current). Total measured lock wait was 0.002 s.

## Results

| Workload | GPU-only current/reference [95% CI] | CPU-exclusive current/reference [95% CI] |
|---|---:|---:|
| TPC-H SF1 advised geomean (26) | **0.999 [0.997, 1.001]** | **0.999 [0.997, 1.001]** |
| doop-gpu xalan fixpoint (0.612 → 0.594 s) | 0.980 [0.964, 0.997] | 1.006 [0.999, 1.014] |

Neither suite interval excludes one. DOOP's GPU-only interval lies below one
while its confirmation interval lies above it, so there is no confirmed DOOP
effect.

Every per-query point estimate lies within 1.5% of one. Some per-query CIs
exclude one in a single mode, never in both:

| Query | GPU-only | CPU-exclusive |
|---|---:|---:|
| H05 | 1.003 [1.000, 1.005] | 1.000 [0.995, 1.005] |
| H20 | 1.007 [1.001, 1.015] | 1.000 [0.998, 1.002] |
| H13 | 0.998 [0.990, 1.006] | 1.009 [1.005, 1.014] |
| D19, D72-inner, H02, H04, H07, H18, H21-positive | include 1 | 0.990–0.999, upper bound < 1 |

These are unconfirmed small effects, consistent with identical query code. No
regression is attributed to this batch.

## Gates for the batch

- **GPU gate.** `bash tools/gpu_gate.sh` **PASS**, 630.59 s wall: crate suites
  338 s, radix counters 32 s, H05 SF10 ceiling 68 s, DOOP builds 99 s and
  Cassandra 26/26 93 s; H05 SF10 8.285 ms ≤ 10.35 ms. It ran on the branch at
  `fadeecc5` + compression.
- **Crate tests after rebasing onto `20f1f04e`.** The three new main commits
  touch only `passes/cq-sequence-plan`, its fixtures, formal docs and a
  dependency check. After the rebase, `gpu-device-columns` and `cq-gpu-tpch`
  passed again: 50 + 18 tests.
