# Preserved merge investigation

The original merge measurements in this section were collected on 2026-10-15 on
the local M5 Pro (16 GPU cores, 24 GB unified memory). No experiment data was overwritten when
moving to the set-difference investigation. The production merge kernel remains
the original tiled implementation; neither direct stores nor cached run heads
showed a consistent improvement.

| File | Meaning |
|---|---|
| `m5-pro-2026-10-15.csv` | Initial MLX/Rust completed-call timings; includes submission/allocation/waits, not just GPU time. |
| `m5-pro-gpu-diagnosis-2026-10-15.txt` | Unchanged shader dispatched directly through Metal; GPU command-buffer timestamps and pipeline resource limits. |
| `limiter-investigation-2026-10-15.json` | Early mixed merge/copy counter recording. Do not attribute its aggregate stall signals to merge alone. |
| `isolated-limiters-2026-10-15.json` | Separate merge-only/copy-only recordings; middle-60%-span filtering, counter statistics and caveats. |
| `cached-heads-2026-10-15.txt` | Paired baseline/cached-head GPU timings. Candidate rejected; original restored. |
| `merge-profile-raw-2026-10-15.tar.gz` | Preserved source traces, capture, verified counter template, diagnostic harnesses and AIR. Kept outside `build/` so cleaning build output does not delete this evidence. |
| `SHA256SUMS` | Integrity checksums for those six artifacts. Run `shasum -a 256 -c benchmarks/SHA256SUMS` from the MCCL crate directory. |

The raw archive includes the mixed counter trace, the isolated merge/copy traces,
the merge/copy GPU capture, `PerformanceLimiters.tracetemplate`, the paired timing
Swift harness, the isolated C++ workload, and baseline/cached optimized AIR. Its
paths retain their original `build/profile/` or `build/profiling/` prefix. Extract
into a separate directory when inspecting; do not overwrite active builds.

The `.trace` recordings reported a logging-stream error. Their GPU counter tables
were nevertheless exported successfully. Counter configuration was established
through the supported Instruments Performance Limiters menu, not guessed from
undocumented profile IDs. Counter influence metrics are not fractions of time
stalled. Samples remain device-wide even when selected by target GPU intervals.

The archive includes local process/device metadata; review it before public
sharing. It is retained locally and as an asset of the private GitHub release
`merge-checkpoint-2026-10-15` in `StarGazerM/mccl`, not in Git history.
The original artifacts under `build/` also remain intact. The archive passed
`gzip -t`; GitHub's uploaded SHA-256 digest matches the local checksum.

Retrieve it into a fresh checkout with:

```sh
gh release download merge-checkpoint-2026-10-15 --repo StarGazerM/mccl \
  --pattern merge-profile-raw-2026-10-15.tar.gz --dir benchmarks
shasum -a 256 -c benchmarks/SHA256SUMS
```

## MCCL lifecycle integration, 2026-10-25

New measurements use the local **M3 Max, 40 GPU cores, 128 GiB**, macOS 27.0.1.
The original M5 Pro files and their checksum manifest remain unchanged.

| File | Meaning |
|---|---|
| `m3-max-array-ready-2026-10-25.csv` | Existing matched merge/copy benchmark after the Rust array-ready completion cutover; equal input lengths, nine samples, three warmups. |
| `m3-max-lifecycle-2026-10-25.json` | Cache-cleared/pooled output and whole-round cases, direct/extra-copy input preparation, four patterns at 64K/1M/16M/64M output keys, nine samples and two warmups; wired budget zero. |
| `m3-max-lifecycle-wired-2026-10-25.json` | Same lifecycle cases at 1M/16M with an explicit 1 GiB wired budget; previous settings restored. |
| `m3-max-lifecycle-sync-2026-10-25.json` | 1M-key lifecycle cases with five samples and a separately measured additional whole-stream synchronization. |

Lifecycle inputs split total keys one-third/two-thirds. Each result is checked
exactly outside operation timers, and bounded retained-output checks verify old
borrowed slices while distinct fresh results are produced. Raw JSON includes first
calls before warmups, every measured sample, graph/eval/read phases, preparation
and release phases, CPU process faults, MLX active/cache/peak accounting and
configuration/restoration records.

Timing is **CPU wall time**, never a GPU timestamp or hardware bandwidth counter.
“Resident” case names mean retained, GPU-touched input handles, not a guarantee
of continuous physical residency. Cache clearing affects only unused MLX cache
entries; it does not reset hardware caches, pipelines or device performance state.
The optional wired budget is independent of the cache budget and defaults to zero.
The first CPU scan includes array-ready access and a checksum, not a host-vector
copy. Full exact comparisons occur separately.

The current wrapper uploads once into MLX-owned shared storage, exposes borrowed
immutable CPU slices, and uses explicit stream barriers only when requested.
The extra-copy control is intentionally labeled: it restores the extra data
movement, not all synchronization details of the previous Rust wrapper. The older
CSV's `cpu_wall_completed_call` and the new `cpu_wall_array_ready_call` therefore
have different completion boundaries and must not be silently combined.

## Index construction investigation, 2026-10-26

M3 Max, 40 GPU cores, 128 GiB, macOS 27.0.1. These files record the
**pre-shared-relation revision**, including per-build CPU population.
The native observer delegated to the then-current pair-index primitives.
They are historical evidence, not current constructor measurements.

| Files | Scope |
|---|---|
| `index-timing-m3max-20261026.csv` | Initial continuously reused 1M/16M/64M combined/split evaluation probe; native encoding and actual GPU timestamps. |
| `index-scheduling-m3max-20261026.csv` | Paired clear/retain boundaries across sizes; adds CPU-driver scheduling, GPU span and CPU fault/system-time fields. Retained boundaries are not guaranteed fully CPU-touched storage. |
| `index-scheduling-idle-m3max-20261026.csv` | Same paired policy at 64M, adding 250 ms idle outside measured phases. |
| `index-steady-active-a-m3max-20261026.csv`, `index-steady-idle-m3max-20261026.csv`, `index-steady-active-b-m3max-20261026.csv` | Active/250-ms-idle/active sequence at 64M with continuously pooled storage; measured CPU population faults were zero. |
| `index-steady-cleared-m3max-20261026.csv` | 64M inactive-cache clearing before every sample; separates CPU first-touch from deferred driver scheduling. |
| `index-timing-smoke-20261026.csv`, `index-scheduling-smoke-20261026.csv` | Small-input validation of the respective diagnostic versions, not crossover evidence. |

Five measured samples per variant, plus separate first observations and two
warmup rounds. Every row and directory entry is checked outside timing.
Native population generates directly into shared storage, so it is not a host
copy benchmark. GPU sums/spans, driver scheduling and CPU wall intervals have
different scopes and need not add. No physical DRAM counters or clock telemetry
were collected. Previous cache limits were restored.

The mini_linq repository retains the 240-configuration construction matrix and
matched raw-Metal/MCCL comparator JSON. See [methods, raw links, measured causes and limits](https://github.com/StarGazerM/mini_linq/blob/codex/all-answers/doc/MAC-IMPLEMENTATION.md#2026-10-26-controlled-construction-investigation).


## Shared-relation construction cutover, 2026-10-26

`shared-source-index-timing-m3max-20261026.csv` measures the new fused shared-source
builder at 1M/16M/64M rows, with five samples per retained/cleared cache policy.
A single-column relation is ingested once per size and retained across every
build; GPU projection generates original row IDs. Source bytes are `4*N`, not
the previous packed-pair `8*N`.

CSV ingestion allocation/population/fault fields repeat that **one-time event**
as context. No population occurs per build. Timed construction includes GPU
projection/sort/directory, output/scratch allocation, scheduling and scalar
count/error publication. Exact output/directory/source validation is separate.
The data include actual command-buffer and CPU-driver scheduling timestamps;
these are not interchangeable with wall time or hardware DRAM counters.

The [shared-source cutover](https://github.com/StarGazerM/mini_linq/blob/codex/all-answers/doc/MAC-IMPLEMENTATION.md#shared-relation-input-cutover)
links the updated source-reuse matrix, two-column/multi-index cases and complete
H03/H09 execution. Do not treat old per-build population times as inherent GPU
construction costs, or compare the new boundary without including ingestion
when the workload actually needs it.

