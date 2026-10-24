# GPU execution: current design and evidence

The `cq_gpu_sequence::translate` terminal in `experiments/gpu` consumes checked,
explicitly graded Sequence and emits CUDA-backed Rust. The
`cq_gpu_sequence::machine_grade` adapter validates placements against the GPU
machine; neither adapter chooses a plan. Sequence records tiles, stores,
canonicalization, output protocols, and machine grades before translation.

The former Structures GPU printer and its maintained-index Datalog workspace
have been removed. Maintained frozen/leveled indexes have no Sequence term.
See the [Sequence workspace](../experiments/gpu/README.md) for the active
backend's supported contracts and commands.

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

- [Historical H05 samples](../experiments/gpu/compare/gpu_h05.results.json).
- [Historical triangle samples](../experiments/gpu/compare/gpu_triangles.results.json):
  directed set triangles, excluding host edge sort/dedup and upload from GPU
  timing. These archived samples are not evidence for the current translator.

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
