# Status

## Where we are

- Semantics, IRs, checks and the pass protocol are complete. Every decision is
  recorded in IR text, and all features build outside the repository.
- Default pipeline (`mini_linq::passes::plan`): left-deep in written order
  (Written/Coarse); the first clause of each scope streams, later clauses are
  indexed and probed, and no pass commutes operands. `plan(no_reduce)` keeps
  the order but emits no semijoins.
- Reduction (Masks): each indexed clause is reduced by its filtered subtrees of
  the stream-rooted reduction tree (GYO join tree, or GYO ears plus a spanning
  tree of the cyclic core), children first, before the stream starts: the
  bottom-up Yannakakis pass, no top-down pass. Exact atom-to-block cuts on any
  scope. Each block branch is its own scope.
- Predicates: relational disjunctions derive implied per-occurrence domains
  (H07 `n_name ∈ {FRANCE, GERMANY}`) while keeping the exact predicate. Guards
  over allowlisted total std operations, or marked `#[total] if <expr>`, move
  like relational filters; other Rust guards stay where written.
- Aggregates: one required set dedup; constant-group folds use `SingleGroup`;
  grouped containers use the recorded fast hasher.
- Join filters (SQL Server bitmaps): every probe or semijoin into a filtered
  build pushes an exact key set per `i64` key column to the scan its spine
  reaches, through intermediate probes; builds keeping over 35% of their source
  rows construct none. Recorded in Structures; the exact probe still runs.
- Bushy branches: independent dimension branches recorded in VO, built once.
- Probes: Swiss-table `HashIndex`; batched (128) prefetched probes on scan
  spines, scalar elsewhere; no per-row `Rc`.

## Gap to DuckDB (SF 1, single thread)

`cargo xtask bench --tpch --sf 1 --runs 5 --reduction-comparison` from
`features/workloads`, release, one warm-up, median of five. MiniLinq excludes
load; DuckDB includes prepare. All configurations exact vs DuckDB at SF 1.
Raw data: `features/workloads/target/bench/tpch-1-reduction-comparison.{tsv,json}`.
Measured on a different, slower machine than the previous table (whose
DuckDB 1T H05 was 18.1 ms; here 39.5 ms), so compare ratios, not times.

| Query | default ms | no-reduce ms | DuckDB 1T ms | default / DuckDB |
|---|---:|---:|---:|---:|
| tpch_q19 | 35.4 | 35.3 | 155.5 | 0.23 |
| retail_d19 | 21.1 | 65.6 | 30.8 | 0.69 |
| retail_d27 | 17.6 | 17.7 | 24.3 | 0.72 |
| retail_d85 | 29.3 | 2,050.0 | 38.1 | 0.77 |
| retail_h05 | 33.7 | 119.7 | 39.5 | 0.85 |
| retail_h07_block | 35.5 | 302.7 | 38.1 | 0.93 |
| retail_h07 | 36.5 | 303.7 | 37.8 | 0.97 |
| retail_h21_positive | 92.5 | 1,016.5 | 92.4 | 1.00 |
| retail_h09 | 168.8 | 1,294.2 | 163.4 | 1.03 |
| retail_h03 | 39.4 | 121.2 | 34.9 | 1.13 |
| retail_h08 | 33.5 | 72.7 | 24.3 | 1.38 |
| tpch_q06 | 28.0 | 28.9 | 18.9 | 1.48 |
| retail_d72_inner | 205.7 | 157.3 | 107.7 | 1.91 |

no-reduce keeps written (fact-first) order, so without reductions the stream
probes unreduced builds. D72's one reduction (Lineitem ⋉ PartSupp on a
non-selective `ps_availqty > 0`) costs 32 ms of composite probes into the
800k-row PartSupp index and filters nothing.

## Predicate transfer route (`cq_planners::order(transfer)`)

Robust predicate transfer (RPT, SIGMOD'25) as a diverging planner through the
same public pipeline: written order, the Masks forward pass plus the stream's
reductions, then the backward pass from filtered sides, all before the join
phase. `cargo xtask profile --tpch --sf 1 --config default|transfer`,
uninstrumented medians (ms), same session:

| Query | default | transfer |
|---|---:|---:|
| retail_d72_inner | 195.2 | 175.6 |
| retail_d85 | 28.4 | 26.3 |
| retail_h08 | 35.3 | 32.9 |
| retail_d19 | 21.4 | 20.2 |
| retail_h07_block | 39.5 | 46.8 |
| retail_h05 | 33.5 | 43.0 |
| retail_d27 | 18.4 | 27.8 |
| retail_h03 | 39.2 | 51.2 |
| retail_h07 | 35.2 | 54.8 |
| retail_h21_positive | 90.2 | 127.0 |
| retail_h09 | 174.8 | 336.1 |

The backward pass pays off where a build is large and only filtered from the
stream side: D72's PartSupp index shrinks from 800k to 124,846 rows (57 → 35
ms). Elsewhere reducing and retaining the stream costs more than it saves:
the default already filters the stream at its scan with the builds' key sets,
and its probes then only hit, which is the SQL Server argument for omitting
the top-down pass.

## Index building vs join processing (SF 1, one thread, planning excluded)

DuckDB 1.5.5 with timers in `PhysicalHashJoin` (sink, combine, pointer-table
init, finalize, Bloom push = build; everything else = non-build), NDEBUG
build, prepared once. MiniLinq: profiled run, exclusive construction of
indexes, key sets and join filters (inputs drained first) = build. Same data;
MiniLinq total is the uninstrumented median. Machine had background load, so
treat differences under ~5% as noise.

| Query | ML total | ML build | ML non-build | DB total | DB build | DB non-build |
|---|---:|---:|---:|---:|---:|---:|
| H03 | 39.9 | 19.1 | 27.4 | 39.0 | 8.9 | 30.2 |
| H05 | 34.7 | 5.0 | 31.0 | 41.5 | 3.5 | 37.9 |
| H07 | 35.1 | 11.3 | 30.0 | 44.0 | 9.7 | 34.4 |
| H07-block | 35.1 | 11.2 | 29.8 | 45.6 | 11.0 | 34.5 |
| H08 | 32.8 | 10.6 | 25.1 | 27.0 | 3.8 | 23.2 |
| H09 | 169.8 | 37.3 | 165.5 | 199.9 | 100.5 | 99.0 |
| H21+ | 91.0 | 14.3 | 80.8 | 94.7 | 7.9 | 86.7 |
| D19 | 20.9 | 8.2 | 16.3 | 30.7 | 0.9 | 29.8 |
| D27 | 18.3 | 1.2 | 17.5 | 23.9 | 0.7 | 23.2 |
| D72 | 192.4 | 61.3 | 118.1 | 135.4 | 72.9 | 62.6 |
| D85 | 29.4 | 11.4 | 20.8 | 37.6 | 1.1 | 36.5 |

DuckDB builds far smaller tables (cost-chosen build sides); our written
order indexes whole dimensions (D19 Part, D85 Customer). DuckDB's non-build
work is lower on H09 and D72.

## Left work

1. **D72.** Its decisive choice is data-dependent: `ps_availqty > 0` keeps
   every PartSupp row. No written order avoids an unreduced large build
   (PartSupp-first indexes all 1.5M Orders: 229–238 ms). The kernel's advice
   `skip(Lineitem with PartSupp), reduce(PartSupp with Lineitem)` gives
   141–145 ms (`advised` configuration) vs 191–195 default and DuckDB 107.7.
   Choosing this automatically needs statistics or a runtime check.
2. **Scans.** Scalar row-at-a-time over 48-byte rows (Q6 2×); columnar scans.
3. **Positional default.** Selections need the caller's `positional(..)` advice.
4. **Zone maps.** Block min/max exists only in the `hypergraph_zone`
   configuration; scan key sets skip blocks only there.
5. **Known limits.** Static heuristics only (no statistics); no keys or
   constraints on data; no parallel execution.

### Decisions needed

1. **Check policy.** Checks that recompute the same decision
   (`join_filter_candidates`, cq-order rank, CK-Masks, branch equality,
   block_contract) vs obligation checks (cq-reduce Tree/Hypergraph). Decide per
   pass.
2. **Derived facts in Structures text** (`Origin`/lineage, `constants`,
   membership `build/key`, fixed `join_filter` fields): drop or keep.
3. **Knob freeze.** No new out-of-band knobs (`plan(..)` options, batch size,
   Bloom size) until plan control is designed.
4. **Pass ownership.** Move `simplify_reductions` to an RA rewrite home; expose
   the default plan chain as a library fn so planners/fixtures stop restating
   it.
5. **Artifacts.** Regenerate `features/workloads/data` (26.7k CSV lines) at test
   time unless handouts need browsable CSV; stop committing feature
   `Cargo.lock`s (ignored by `check_external.py`); drop or narrow API snapshots
   (filter `kw::*`/auto-trait/tuple-macro rows); shrink
   `regression-targets.tsv` to rows `cargo test --workspace` does not cover;
   cut the e2e 4×4,608 matrix to pairwise + one full kernel.
6. **cq-load split.** Move `ColumnAt` and slice impls to cq-storage.

## Next step

Decide whether runtime adaptivity (abandon reductions whose builds keep over
35% of their source) should choose such reductions without advice.
