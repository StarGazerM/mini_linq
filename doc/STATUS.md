# Status

## Where we are

- Semantics, IRs, checks and the pass protocol are complete. Every decision is
  recorded in IR text, and all features build outside the repository.
- Semijoins no longer copy rows: reduced relations are selections over their
  host, filters run inside the scan, and zone-mapped hosts skip blocks.
- Hashing uses a recorded fast hasher and a compact index layout.
- The facade defaults to Written/Coarse with checked Hypergraph reduction.
  A tree reducer applies only on acyclic scopes; exact atom-to-block cuts apply
  on any scope. Each block branch is analyzed independently.
  `plan = mini_linq::passes::plan_written(no_reduce);` in the
  existing `compile!` pipeline restores the old unreduced plan.

## Default reduction comparison (SF 1)

Measured with `cargo xtask bench --tpch --sf 1 --runs 5 --reduction-comparison`
from `features/workloads`: release profile, one warm-up, median of five runs.
`default` enables acyclic tree reduction and exact block cuts; `no-reduce` selects
`mini_linq::passes::plan_written(no_reduce)`. All other stages and the timing
boundaries are identical. Ratio is default / no-reduce; lower is faster.
Measurements include redundant-support absorption and positional union replay
ownership; every worker finished compilation before timing. H07_block was
remeasured after confirming restoration of its exact block cuts.

| Query | no-reduce ms | default ms | Ratio | Exact vs DuckDB |
|---|---:|---:|---:|---|
| retail_h03 | 107.255 | 75.895 | 0.708 | Pass, both |
| retail_h05 | 268.438 | 269.450 | 1.004 | Pass, both |
| retail_h07 | 760.441 | 879.987 | 1.157 | Pass, both |
| retail_h07_block | 629.782 | 103.127 | 0.164 | Pass, both |
| retail_h08 | 71.367 | 43.419 | 0.608 | Pass, both |
| retail_h09 | 946.543 | 2169.816 | 2.292 | Pass, both |
| retail_h21_positive | 671.349 | 290.010 | 0.432 | Pass, both |
| retail_d19 | 91.078 | 61.438 | 0.675 | Pass, both |
| retail_d27 | 57.469 | 33.724 | 0.587 | Pass, both |
| retail_d72_inner | 107.134 | 108.536 | 1.013 | Pass, both |
| retail_d85 | 793.470 | 46.329 | 0.058 | Pass, both |
| tpch_q06 | 26.090 | 24.648 | 0.945 | Pass, both |
| tpch_q19 | 56.476 | 57.764 | 1.023 | Pass, both |

The runner generated missing local SF1 and SF0.1 dbgen data using DuckDB
v1.5.5. Both configurations passed exact answer comparison at both scales for
all 13 available kernels. Q6 and Q19 are full implemented TPC-H queries; the
other entries are derived retail kernels, not claims of full TPC-H coverage.
Reduction is not universally faster: H09 regresses 2.292× and H07 1.157×;
Q19 is near parity at 1.023×. The opt-out is useful beyond reproducing old text.
Raw full-catalog measurements are in
`features/workloads/target/bench/tpch-1-reduction-comparison.{tsv,json}`; the
updated H07_block row comes from
`features/workloads/target/bench/tpch-1-retail_h07_block-reduction-comparison.{tsv,json}`.

### Tree and block-cut scope

Tree reduction applies only to acyclic scopes. Exact semijoins by finite
Rust-free block relations apply to cyclic and acyclic scopes. Each branch is
analyzed independently, so an acyclic branch under a cyclic root may still
receive its tree reducer. Cyclic block-free scopes such as H05 have no reductions.
All 13 benchmark queries, plus Q6/Q19 aggregate-input plans, are byte-identical
to the plans preceding the discarded strict-no-cuts policy at VO, RA and Physical
for both default and opt-out. H07_block again has four RA/Physical semijoins.
The H07_block-only SF1 rerun used five measurements after one warm-up and
RuntimeDiag timing clearance: default 103.127 ms versus opt-out 629.782 ms
(6.11× faster), both exact against DuckDB with 5,501 rows. The SF0.1 correctness
gate also passed both configurations with 601 rows. Other queries were not
remeasured because their plans are unchanged.

### Validation and native-host compatibility

`cargo test --workspace --locked` passes: **471 tests across 98 suites**.
The unchanged ordered-only-key dispatch fixture compiles and passes under the
reduced default.
Coverage includes cyclic bodies with block cuts but no tree steps, cyclic
block-free bodies with zero reductions, and independently reduced acyclic
branches under cyclic roots.

The initial full reducer materialized `R ⋉ L` for both `L ⋉ (R ⋉ L)` and
the final probe, imposing an unnecessary `Hash` requirement on an ordered host.
The checked `cq_elaborate::simplify_reductions` RA pass proves the input-key
containment, rewrites these consumers to use native `R`, then removes the dead
reduced copy. Both the facade and external Hypergraph planner use this one
rewrite. Necessary reductions, independent consumers, opaque Rust chains and
negative operands retain their obligations. The reducer and independent checker
enforce the same scope-local tree and exact block-cut policy.

The full suite also covers the previously rejected reduced prefix-guard plan.
The RA checker recognizes local semijoin identities while preserving independent
witnesses, predicate ownership and hidden shared columns. New counterexamples
reject absorption when the consumer lacks the required support key.
Replayed positional union scans construct their owned predicates per traversal,
avoiding repeated moves of one predicate across several index builds.
`python3 tools/check_public_api.py --update` regenerated 22 reviewed snapshots;
the new public entry is `cq_elaborate::simplify_reductions(&ra::Program)`.

A separate throwaway acyclic-query comparison passed: the actual `no_reduce`
macro's emitted RA exactly matches the previous normalize/order/elaborate
pipeline. The scratch test was deleted; permanent pipeline regressions retain
the corresponding default-versus-opt-out checks.

## Historical gap to DuckDB (SF 1, single thread, query time)

| Kernel, plan | MiniLinq | DuckDB |
|---|---:|---:|
| D85 hypergraph | 73 ms | ~40 ms |
| H03 hypergraph | 76 ms | ~36 ms |
| H03 unreduced (former default) | ~350 ms | ~36 ms |
| D85 unreduced (former default) | 1,512 ms | ~40 ms |
| H09 unreduced (former default) | 2,020 ms | ~160 ms |
| Q19 unreduced (former default) | 145 ms | ~155 ms |

These pre-cutover measurements compared the former unreduced default with the
external Hypergraph planner. They are not measurements of the new default.
The former unreduced plans' hottest symbol was `HashIndex::lookup`: one probe
per row.

## Left work

1. **Measure batching.** Profile whether batched lookups and row-at-a-time
   iteration explain the remaining reduced-plan gap before changing the runtime.
2. **Plan shape.** Implied selections from guards, e.g. H07's
   `n_name ∈ {FRANCE, GERMANY}`, are not derived. Opaque Rust guards (H09's
   `contains`) stay where written by design.
3. **Positional default.** Selections need the caller's `positional(..)` advice,
   because the compiler cannot see host Rust types. Defaults copy.
4. **Approximate filters** (Bloom, coarse bitmaps) need a covering exact check
   in the IR; not started.
5. **Known limits.** No keys or constraints on data; no query minimization;
   no parallel execution; block fix can form Cartesian intermediates in rare
   cases; D27 hypergraph 4% slower after the hashing change.

## Next step

Profile H09 and D85 with reduction at SF 1 with batched-lookup experiments, then decide
whether a batch interface in `cq-storage` (recorded in Structures) is justified.
