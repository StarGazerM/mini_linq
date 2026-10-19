# GPU execution: current design and evidence

The external `cq_gpu::print` terminal printer in `experiments/gpu` consumes
checked Structures and emits a Rust expression (`Structures → syn::Expr`).
There is no Device IR. The earlier `cq-lang-device` grammar and `via cursor`
encoding are superseded, not pending implementation.

## Implemented contract

- **Plan:** `cq_planners::order(le)` records early non-introducing lookups and
  demand-delayed coarse expansions, respecting written Rust Before boundaries.
- **Representation and evaluation:** `sorted_runs`, `prefix_runs`,
  `build_host_reads`, `retain_lookups` and explicit `DeviceSet` retyping record
  the providers used by the GPU printer. `share` plus `repr … witness` retains
  an eligible lookup until its forcing probe; `stage … expand` records
  segmented expansion. See [the co-design contract](GPU-LE-BACKEND-CODESIGN.md).
- **Control:** CUDA tile streams implement those recorded decisions. The
  supported tile is 256 × 4, with a rank directory and an explicitly configured
  bounded descriptor frontier. Unstaged probes traverse ranges lane-locally;
  cooperative expansion is selected by the recorded staging, not inferred
  from `probe first`. `first` selects a traversal, not a PK/unique-key fact.
- **Storage:** device blocked runs cluster payloads stably and expose key
  ranges. A checked sortedness test selects the already-sorted path; otherwise
  the builder uses counting sort. Rank bitmaps represent presence, including
  holes; an integral host key does not imply a dense, zero-based or hole-free
  domain. Domain widths, row-index limits and allocation budgets still apply.
- **Output:** exact device dedup compares complete keys. Results are owned
  resident device columns; `.to_host()` explicitly transfers tuples. An
  explicit `capacity(Relation)` is an allocation contract, not a uniqueness
  proof. Otherwise bounded grow-and-resume saves unfinished execution state.
  A finite input tile bounds descriptor storage, not expanded cardinality.
  Expansion totals must fit the supported index domain; output growth and
  metadata handling can synchronize with the host.

The pinned cuda-oxide toolchain compiles ordinary Rust kernels. RCCL supplies
host pool allocation, not device sort/scan kernels: the recorded PTX shared
symbol collision prevents that combination. Unsupported constructs are
rejected rather than run on an implicit CPU fallback. These include opaque
Rust, generators, arbitrary computed heads, union/fallback, merge/smallest,
aggregates and recursion. See the [workspace contract and supported domain](../experiments/gpu/README.md#structures-gpu-terminal-printer)
for current codecs, provider restrictions, reproduction commands and resource
audits. The printer is experimental, not a general GPU implementation of all CQ.

## Measurements and provenance

Performance is workload-, plan-, representation- and tuning-specific. Exact
full-tuple validation is distinct from counts-only validation. Resident
execution excludes load/upload/setup unless the cited measurement explicitly
includes it; CUDA-event totals are not automatically end-to-end wall time.
Observed timing differences alone establish neither a causal explanation nor
global optimality. In particular, best-of-tested-order results are not an
optimality claim.

Historical H05 summary (RTX 6000 Ada; the original design's recorded values,
not new measurements):

| System | SF1 ms | SF10 ms |
|---|---:|---:|
| Hand-written Plan A, fact-first | 1.9 | 6.2 |
| Hand-written Plan B, dimension-first | 2.6 | 10.1 |
| srdatalog-python, best of 14 tested orders | 5.2 | 176.8 |
| srdatalog-python, first fact-first baseline | 38.1 | 574–583 |
| srdatalog-python, default plan | 38.3 | 633.5 |
| srdatalog-python, dimension-first order | 2,872 | 58,051 |
| DuckDB 1 thread | 55 | 618 |
| DuckDB 24 threads | 21 | 154 |
| CPU MiniLinq default, other machine | 33.7 | — |

The GPU values exclude one-time input upload and final answer transfer, but
include run builds, fused execution and dedup. srdatalog timings are after
`prepare`, with index builds but without H2D; DuckDB runs full `SELECT DISTINCT`
on dbgen tables. These boundaries and the different CPU machine preclude a
like-for-like end-to-end speedup claim. The table is a historical tuned
hand-written result, not the generated printer's current performance.

Canonical evidence, rather than duplicate specification and tuning narratives:

- [H05 samples and optimization history](../experiments/gpu/compare/gpu_h05.results.json),
  [H05 measurements](../experiments/gpu/h05/README.md), and
  [original environment and commands](../experiments/gpu/README.md#historical-original-measurements-2026-10-16).
- [Generated printer and resident measurements](../experiments/gpu/README.md#structures-gpu-terminal-printer):
  distinguish device execution, setup/load, tuple transfer and counts-only paths.
- [SRDatalog order sweep](../experiments/gpu/README.md#srdatalog-variableclause-order-sweep-2026-10-18):
  rejected wrong-answer configurations are not timing baselines.
- [Triangle experiment](../experiments/gpu/README.md#directed-triangle-lookup-expand-experiment)
  and [samples](../experiments/gpu/compare/gpu_triangles.results.json): directed
  set triangles, with host edge sort/dedup and upload excluded from GPU timing.
  Its count baseline is not a like-for-like end-to-end full-tuple speedup.

## Recursion: design boundary, not an implemented GPU feature

Immutable sorted batches and cursor merges are useful reference mechanisms
from Differential Dataflow, but its logical timestamp compaction is not
physical run merging. Nor is its logical frontier identical to the protected
cuts in [the incremental specification](formal/incremental.tex). Protected cuts
constrain which runs may be merged while preserving the specified snapshots;
a correspondence to DD requires a separate argument.

DD differences are weights: consolidation, multiplicity and retractions mean
they are not generally +1. Insert-only set maintenance can admit each new tuple
once after deduplication; that restricted case does not describe arbitrary DD
batches or join intermediates. Likewise, source evidence for SRDatalog's merge
maintenance supports a merge claim, not an assertion that it re-sorts the
whole relation. GPU recursion, trace witnesses and compaction remain outside
the implemented printer contract; this note does not extend the mathematical
core specification.
