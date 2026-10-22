# GPU execution: current design and evidence

The `cq_gpu_sequence::translate` terminal in `experiments/gpu` consumes checked,
explicitly graded Sequence and emits CUDA-backed Rust. The
`cq_gpu_sequence::machine_grade` adapter validates placements against the GPU
machine; neither adapter chooses a plan. Sequence records tiles, stores,
canonicalization, output protocols, and machine grades before translation.

The former Structures implementation remains isolated in
`experiments/gpu/legacy-datalog` for maintained-index Datalog and handwritten
baselines. Maintained frozen/leveled indexes have no Sequence term yet.
The following contract and measurements describe that legacy implementation,
not additional capabilities of the Sequence translator.

## Legacy Structures contract

- **Plan:** `cq_planners::order(le)` records early non-introducing lookups and
  demand-delayed coarse expansions, respecting written Rust Before boundaries.
- **Representation and evaluation:** `sorted_indexes` records which sorted
  layouts serve each built result's reads (minimum chain cover of read keys,
  levels in binding order, `Rel => [a][b]` overrides; EXT-SortedIndexes).
  `sorted_runs`, `prefix_runs`, `build_host_reads`, `retain_lookups` and
  explicit `DeviceSet` retyping record the providers used by the GPU printer. `share` plus `repr … witness` retains
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
and aggregates. GPU recursion is handled by a separate checked Datalog bridge,
not a new CQ operator. See the [legacy workspace contract](../experiments/gpu/legacy-datalog/README.md)
for its codecs, provider restrictions and historical commands; the
[Sequence workspace](../experiments/gpu/README.md) documents the active backend.

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
  [legacy H05 measurements](../experiments/gpu/legacy-datalog/h05/README.md), and
  [original environment and commands](../experiments/gpu/legacy-datalog/README.md#historical-original-measurements-2026-10-16).
- [Generated printer and resident measurements](../experiments/gpu/legacy-datalog/README.md#structures-gpu-terminal-printer):
  distinguish device execution, setup/load, tuple transfer and counts-only paths.
- [SRDatalog order sweep](../experiments/gpu/legacy-datalog/README.md#srdatalog-variableclause-order-sweep-2026-10-18):
  rejected wrong-answer configurations are not timing baselines.
- [Triangle experiment](../experiments/gpu/legacy-datalog/README.md#directed-triangle-lookup-expand-experiment)
  and [samples](../experiments/gpu/compare/gpu_triangles.results.json): directed
  set triangles, with host edge sort/dedup and upload excluded from GPU timing.
  Its count baseline is not a like-for-like end-to-end full-tuple speedup.

## Resident Datalog and the remaining maintenance boundary

`cq_gpu_datalog::datalog!` reuses the existing source, SCC, semi-naive and fork
checks with explicit device admission/negative-key policies. Supported CQ holes
use the existing GPU printer. Candidates are deduplicated and subtracted from
the accumulated relation on device; every SCC successor is prepared before
publication. Full/delta outputs remain immutable resident snapshots. Small
checked metadata readbacks control convergence and allocation.

The legacy bridge maintains explicit frozen and leveled index snapshots; this
is the boundary not represented by the Sequence cutover. Supported syntax,
dictionary requirements, oracle cases and historical measurements remain under
[Resident Datalog](../experiments/gpu/legacy-datalog/README.md#resident-datalog).

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
whole relation. Trace witnesses and compaction remain outside the implemented
backend; this note does not add Differential Dataflow timestamp semantics to
the mathematical core specification.
