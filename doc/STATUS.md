# Status

## Where we are

- Semantics, IRs, checks and the pass protocol are complete. Every decision is
  recorded in IR text, and all features build outside the repository.
- Default pipeline (`mini_linq::passes::plan`): left-deep in written order
  (Written/Coarse); the first clause of each scope streams, later clauses are
  indexed and probed, and no pass commutes operands. `plan(no_reduce)` keeps
  the order but emits no semijoins.
- Sequence is the shipped CPU target: `sequence` → `sequential` →
  `translate_pull`, including profile counters. Explicit Structures push
  terminals remain selectable without Sequence phases. The former direct
  Structures pull terminal and its compile APIs are removed; shared
  realization helpers remain for the push printer.
- Reduction (Masks): each indexed clause is reduced by its filtered subtrees of
  the stream-rooted reduction tree (GYO join tree, or GYO ears plus a spanning
  tree of the cyclic core), children first, before the stream starts: the
  bottom-up Yannakakis pass, no top-down pass. Exact atom-to-block cuts on any
  scope. Each block branch is its own scope.
- Predicates: relational disjunctions derive implied per-occurrence domains
  (H07 `n_name ∈ {FRANCE, GERMANY}`) while keeping the exact predicate.
  Comparisons and `&& || !` over query variables are relational and move like
  filters; every other Rust guard is ordinary Rust, type-checked by rustc and
  kept where written (`#[total]` and the allowlist were removed). `!R(args)`
  is core negation, elaborated to an anti-join (`doc/formal/guards.tex`).
- Aggregates: one required set dedup; constant-group folds use `SingleGroup`;
  grouped containers use the recorded fast hasher.
- Join filters (SQL Server bitmaps): every probe or semijoin into a filtered
  build pushes an exact key set per `i64` key column to the scan its spine
  reaches, through intermediate probes; builds keeping over 35% of their source
  rows construct none. Recorded in Structures; the exact probe still runs.
- Bushy branches: independent dimension branches recorded in VO, built once.
- Probes: Swiss-table `HashIndex`; batched (128) prefetched probes on scan
  spines, scalar elsewhere; no per-row `Rc`.
- Back end (`doc/formal/backend.tex`): three recorded layers over one
  Physical program. R, representation: `r repr = tuples | positions {..}
  owned [..]` (data refinement, α fetches columns by lineage). E, evaluation
  unit: `r stage = batch N` (a ⊥-free segment from a `for` producer runs at
  most N rows ahead; opaque Rust ends a stage; probe/merge cursors start
  one). C, control: pull (unfold) or push (fold) of the same stream algebra,
  an emitter choice. Default passes `represent` and `stage` record today's
  decisions only (the probe lookahead is now `stage = batch 128` in text);
  `represent(positions)` and `stage(batch(N))` opt in. Printers bind only
  the columns an operator reads (`cq-print` column environment). Positions
  stop at probes: carrying a `RowId` through an index payload is deferred.

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

Exact-semijoin adaptation of RPT (SIGMOD'25), through the same public pipeline:
connected growth within written atom runs, Masks forward pass plus the stream's
reductions, then the backward pass from filtered sides, before the join phase.
Predicate, binding and block boundaries stay fixed. This is not the paper's
vectorized Bloom pipeline or RPT+'s adaptive execution.

The implementation now applies checked reduction absorption after elaboration,
retains positional snapshots only at recorded retained boundaries, enumerates
surviving bitmap positions during narrowing, and shares equivalent scan filters
after validating every covering witness. Default and transfer matched cached
DuckDB witnesses for predicate-form H05, H07-block, D27 and H08 at SF0.1 and SF1.

The full 29-case catalog now exposes transfer, including every stage of the
12 standalone retail runners. All 29 transfer cases matched DuckDB at SF0.1 and
SF1. The full-suite check exposed H09's disconnected Part × Orders prefix:
at SF0.1 it created 161,250,000 pairs before a Lineitem probe reduced them to
32,160. Connected atom scheduling removes that prefix; SF1 transfer completed
in 487.8 ms (pooled median of ten), versus the prior 120-second worker timeout.
The explicit written-order H09 baseline is unchanged and its SF1 timeout was
not rerun. Left-side absorption also removes H21-positive's 356,794 duplicate
membership checks, with support pruning performed first to preserve H03/D85
scan filters. [Full evidence](../experiments/gpu/compare/rpt-repair/summary.json)
and [29-query matrix](../experiments/gpu/compare/rpt-repair/matrix.tsv).

Historical measurements below predate these repairs; they do not characterize
the repaired path. `cargo xtask profile --tpch --sf 1 --config default|transfer`,
uninstrumented medians (ms), same historical session:

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

The historical D72 PartSupp build shrank from 800k to 124,846 rows (57 → 35 ms).
The other historical regressions included avoidable retained supports, bitmap
snapshots/full-base enumeration and duplicate filter construction. They are not
evidence that these costs are inherent to predicate transfer. Four repaired
derived kernels do not establish global non-regression or full Yannakakis coverage.

## Lookup-expand route (`cq_planners::order(le)`)

The external planner records existing VO subatoms: a no-new-name lookup on
the currently bound join key, followed by one coarse full correlated row.
Ready lookups follow the GYO ear order (then the Masks cyclic-core tree);
the outer stream and Rust chain remain Written/Coarse. Expansions wait for
column demand, a written Rust Before boundary, or scope exit. Opaque Rust
retains its whole lexical environment through `FreeVariables::dependencies`.
Blocks recurse without changing their interface or fallback ancestry.

Coarse expansion is deliberate: CK-Le checks exactly one full expansion per
non-stream occurrence, rather than claiming Fine's one-variable obligation.
It independently checks the non-introducing lookup before that expansion and
written column ownership; WF-VO checks first uses, complete rows and Rust
boundaries, Masks checks reductions, and the final CQ→RA′ check remains active.
No RA′, VO, Physical grammar, runtime feature, or core compiler dependency is
added. `physical = [cq_rewrites::smallest_intersections]` remains the public
intersection hook; this coarse planner does not manufacture Fine ranges or
claim a worst-case-optimal join bound.

The retail runtime registers `le` and `le_runs`. The e2e corpus carries both
as planner axes (538 new accepted configurations; the two D85 `merge_joins`
+ HashMap witnesses are excluded by the existing accepted-domain policy and
pinned as negative UI fixtures); the corpus gate passes (3,440 tests).

`le_runs` explicitly applies `sorted_runs`; core default selection is
unchanged. Sorted-run retyping is limited to one-level HashIndex payload
builds with a single stored borrowed i32/i64 key, selecting the integral
codec eligible for the rank directory. Multi-column, non-integral and
computed/owned keys stay hashed; the rewrite never selects NoDense
binary-search runs. An integral key with an over-budget span can still fall
back to binary search at runtime.

Cursor retention uses `share` / `repr … witness` / `stage … expand`
(ST-Share, STR-Witness, ST-Expand; `cq_rewrites::retain_lookups`), the R/E
encoding in [the co-design note](GPU-LE-BACKEND-CODESIGN.md). `le_runs` =
`order(le)` + `sorted_runs` + `retain_lookups`. Historical SF1 uninstrumented
medians (`cargo xtask bench --tpch --sf 1 --runs 5 --config le_runs`,
2026-10-18, all 13 exact vs DuckDB):

| Query | default | le_runs | ratio |
|---|---:|---:|---:|
| retail_h03 | 33.5 | 27.7 | 0.83 |
| retail_h07 | 37.0 | 29.5 | 0.80 |
| retail_h07_block | 34.8 | 28.4 | 0.82 |
| retail_h08 | 31.6 | 27.1 | 0.86 |
| retail_d19 / d27 / d85 / q06 / q19 | 21.2 / 17.2 / 28.9 / 25.6 / 36.3 | 20.8 / 17.5 / 29.4 / 26.0 / 35.2 | 0.97–1.02 |
| retail_d72_inner | 182.5 | 193.6 | 1.06 |
| retail_h09 | 164.2 | 186.6 | 1.14 |
| retail_h21_positive | 89.6 | 105.6 | 1.18 |
| retail_h05 | 32.6 | 41.0 | 1.26 |

These measurements combine planning, representation and sharing changes;
they do not isolate the causal contribution of cursor retention. Eligibility
and non-filtering lookups help describe the different plans, but do not prove
the cause of each timing difference. The following historical table measures
the superseded hidden-cursor implementation, not the current encoding.

Re-measured 2026-10-16 from `features/workloads`, sequentially with
`cargo xtask profile --tpch --sf 1 --config default` then
`cargo xtask profile --tpch --sf 1 --config le_runs`, with no concurrent
checks or benchmark jobs. These are uninstrumented medians of 11 timed runs
after warm-up, in ms; ratios are `le_runs/default` (lower is better).
All 13 kernels were exact against DuckDB in both configurations, including
the separate counter and timing workers; all independent build-count checks
passed. The existing SF1 database and Parquet projections were reused.

| Query | default | le_runs | Ratio | Change |
|---|---:|---:|---:|---:|
| retail_h03 | 39.813 | 31.195 | 0.784x | -21.6% |
| retail_h05 | 33.053 | 42.664 | 1.291x | +29.1% |
| retail_h07 | 36.098 | 30.200 | 0.837x | -16.3% |
| retail_h07_block | 37.248 | 28.932 | 0.777x | -22.3% |
| retail_h08 | 33.269 | 27.913 | 0.839x | -16.1% |
| retail_h09 | 164.862 | 189.177 | 1.147x | +14.7% |
| retail_h21_positive | 92.148 | 113.095 | 1.227x | +22.7% |
| retail_d19 | 20.684 | 21.471 | 1.038x | +3.8% |
| retail_d27 | 17.980 | 18.158 | 1.010x | +1.0% |
| retail_d72_inner | 193.369 | 177.665 | 0.919x | -8.1% |
| retail_d85 | 27.023 | 30.283 | 1.121x | +12.1% |
| tpch_q06 | 28.419 | 31.262 | 1.100x | +10.0% |
| tpch_q19 | 35.758 | 35.991 | 1.007x | +0.7% |

The earlier NoDense runs measured H09 at 326.040 ms and D72-inner at
329.832 ms. Their composite PartSupp keys now stay HashIndex. The H09
profile actually builds 42,656 filtered PartSupp rows, not 800,000;
D72-inner builds all 800,000. This fixes the unintended binary-search
replacement, but does not make every LE schedule faster than default.

Every remaining regression above 10% was inspected in
`/data/codex/opprofile-evidence/<query>.{default,le_runs}.ours.txt` and the
matching `.validation.json`:

- **H05 (+29.1%):** Supplier is a host fallback, not an eligible retained
  build: `r12` still tests the 10,000-key HashSet
  `r11.fallback.supplier_key`, and `r20` probes the separate composite
  HashIndex `r11.fallback.nation_key_supplier_key`. All 184,082 rows pass
  the added supplier/order/customer/nation/region semijoins before the final
  supplier probe retains 7,243. Residual-key support therefore does not remove
  this particular second lookup. Order/customer built payload indexes use
  integral-codec SortedRuns; order join-filter insertions still double from
  46,008 to 92,016. The answer span grows 23.424 → 36.154 ms despite exclusive
  build time falling 5.851 → 3.011 ms.
- **H09 (+14.7%):** `r10` and `r17` both probe
  `r3.index.part_key_supplier_key`, now a HashIndex rather than a NoDense
  SortedRun. Both emit all 319,404 rows; default has one such expansion.
  Supplier's host HashIndex still has an added non-filtering semi, and
  Nation is read twice. The order-key payload build `r5` remains an
  integral-codec SortedRun; its join-filter insertions double
  287,490 → 574,980. The answer span is 120.231 → 153.650 ms. Instrumented
  totals are nevertheless 207.327 → 203.368 ms because default instrumentation
  overhead is +25.76%, versus +7.50%; these clocks cannot precisely apportion
  the uninstrumented 24.315 ms regression.
- **H21-positive (+22.7%):** `r9.index.order_key` is an integral-codec
  SortedRun rather than default's HashIndex, while `r8.index.order_key`
  remains the 729,413-key membership provider. Added `r11/r12` order semis
  preserve all 356,794 outer rows, and `r14/r16` supplier/nation semis
  preserve all 378,274 expanded rows. The `r9` join-filter insertions double
  72,110 → 144,220. Exclusive build time falls 14.630 → 8.588 ms, but the
  answer span grows 50.182 → 76.703 ms: saved build work does not pay for
  the lookup/expansion schedule. Integral-codec eligibility alone does not
  guarantee that a runtime key span fits the rank directory.
- **D85 (+12.1%):** the composite Lineitem build
  `r8.index.part_key_supplier_key` remains HashIndex. `r13` tests it before
  `r16` expands it; the hash provider cannot retain a Copy run cursor.
  `r12` and `r26` also access the 200,000-row Part host HashIndex twice,
  versus one Part probe in default; early `r12` touches 24,238 rows before
  composite-key filtering leaves 8,913. Both composite join filters double
  insertions 8,913 → 17,826. Part's fallback build costs 8.372 ms exclusive
  (default 7.807); the answer span grows 13.773 → 16.761 ms.
- **Q06 (+10.0017%, rounded to +10.0% above):** there is no keyed provider
  to retype or cursor to retain. Both profiles have the same three range
  scans (6,001,215 → 909,455 → 248,078 → 114,160 rows), binds, and HashSet
  dedup. The scan/bind span is 20.551 → 20.626 ms and the answer/dedup span
  28.828 → 29.694 ms; instrumented totals differ only +3.0%.
  [INFERENCE] The larger plain-run difference is measurement/code-layout
  variability, not an observed representation regression; the operator
  evidence does not establish its exact cause.

Operator stream spans are inclusive and overlap; they are not additive
exclusive costs. LE plus sorted runs remains an explicit CPU experiment,
not a universal speedup or a saved-descriptor GPU implementation.

### GPU integration

The external `cq_gpu_sequence::translate` terminal consumes checked, graded
Sequence; `machine_grade` validates the GPU machine model. There is no Device
IR. `probe first` is not a uniqueness fact,
and integral host keys need not be dense or hole-free. Finite input batches
do not bound total expansion or eliminate host synchronization.

See [the current design](GPU-DESIGN.md) and
[the workspace evidence](../experiments/gpu/README.md)
for supported constructs, exactness checks, resident/load/transfer boundaries
and historical tuning results. Counts-only measurements are not full-tuple
measurements. The triangle GPU baseline excludes host edge sort/dedup and
upload, so it cannot establish a like-for-like end-to-end speedup.

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
2. **Scans.** Q6 `default` 28.0 ms vs DuckDB 1T 19.1 (execute) on this
   machine. Measured (perf, plain release worker, query-only by 5/45-run
   delta; `/data/codex/q06-perf`): 191 M instructions and 127 M cycles per
   run, IPC 1.50, 31.8 instructions per scanned row, 5.3 M L1d misses
   (≈ the 4.5 M cache lines of the 288 MB row array). Attribution:
   `BlockRead::read_owned` over the shipdate column 61.5% (17.2 ms);
   discount/quantity filters, binds and project 16.1% (4.5 ms); the `canon`
   dedup `HashSet<(i128,i32,&i64,&i64)>` 17.2% (4.8 ms, of which
   `reserve_rehash` growth 12.7%). A plain `for` loop over the same 48-byte
   rows evaluating the whole predicate and sum takes 12.5–13.7 ms, and the
   shipdate filter alone 9.8 ms (29 GB/s), so the row layout is not the
   gap: the iterator-driven read costs 1.3–2× the plain loop (its inner
   loop re-tests the `Bound` discriminants and stores the position per row,
   and returns through `next()` per survivor), and the dedup set is pure
   overhead on a key-unique relation. Staging alone (`staged`
   configuration, one fused selection pass per chunk) does not change this:
   29.8 vs 29.8 ms. Columnar storage would cut the shipdate pass from
   9.8 to ~1.2 ms (measured on a 4-byte column) but is the smaller of the
   three effects.
3. **Positional default.** Selections need the caller's `positional(..)` advice.
4. **Zone maps.** Block min/max exists only in the `hypergraph_zone`
   configuration; scan key sets skip blocks only there.
5. **Known limits.** Static heuristics only (no statistics); no keys or
   constraints on data; no parallel execution in the default sequential target
   (`sequence-tasks` and `datalog-parallel` provide Rayon realizations).

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
   `regression-targets.tsv` to rows `cargo test --workspace` does not cover.
6. **cq-load split.** Move `ColumnAt` and slice impls to cq-storage.

## Next step

Decide whether runtime adaptivity (abandon reductions whose builds keep over
35% of their source) should choose such reductions without advice.

## Datalog (`features/datalog`, `features/datalog-parallel`)

Rule: features build on the public core API (`languages/`, `passes/`,
`runtime/`, `facade/`, `testkit/`, root manifests); core changes need user
approval.

- Structure (spec `doc/formal/datalog.tex`): checked languages Source, Strata,
  Incremental, Storage, Round (each a `cq_stage::Language`); passes `if let`
  surface, stratify, incremental (semi-naive only, by decision), storage,
  fork, print_rounds; a Datalog-owned named configuration; shared printer
  library `cq-datalog-print`; resume by entering any pass macro. Contracts
  admit any correct choice, and Round records every former printer decision
  (providers, maintenance, phases, publication order, termination, observer,
  key scope, outputs). `cq-datalog-testkit` executes each IR's meaning.
- Holes are ordinary CQ programs (default tail `sequence → sequential →
  translate_pull`); typed hole interfaces (pull/push) are checked against a
  terminal registry; Round rewrites may change one hole's CQ sub-pipeline.
- Dedup ownership: Storage proves hosts canonical and Round records `admit R
  from [holes]`, discharging `host_set .. discharged` and `consumer canonical`
  (`doc/formal/context.tex`); rule holes build no local `__cq_seen`.
- Negation is core `!R(..)` (anti-join, `doc/formal/guards.tex`) over key sets
  built once per stratum; zero per-call index builds.
- Fx hashing is the default; keyed `cq_storage` providers via
  `hasher(keyed)`/`hasher_round(keyed)` (`mini-keyed` engines).
- Compile time (base DOOP crate): macro expansion 139 → 3.4 s (holes declare
  only what they read, one check per pass, linear source checks, optimized
  proc macros); codegen partitioned per component/hole; one crate per engine.
- Parallel extension: partition Round rewrite + Rayon printer, sharded
  admission, per-layout exclusive publication, `self_dedup per_task` through
  the checked push-terminal rewrite (Structures `print_push`, because the
  Sequence back end has no supplied-consumer entry, `cq-sequence-pull`
  `lib.rs:654-711`). DOOP at 15 threads ≈ 0.8 s (cassandra), 2.3 s (xalan);
  all six apps exact vs Ascent sequentially and at 15 threads.
- Schedule: Round records per-hole read sets, wave partitions, per-hole
  policy, derived publication placement and a progress mode
  (`WF-DL-Schedule`, `WF-DL-Progress`, `DL-WaveCommute`,
  `DL-PublishAfterReaders`, `DL-AsyncCovering`, `CK-DL-Schedule`,
  `P-DL-Waves`, `P-DL-Relational`, `DL-RelationalScheduler`;
  `doc/formal/datalog.tex` "Schedule records", "Relational progress"). Waves
  are rayon scopes on CPU and will be stream/graph branches on GPU; admission
  is the only per-head join. `relational` progress drops the step-round
  barrier: per-member batches, tiled delta windows, scheduler-gated
  isolation (no lock ever waits). DOOP cassandra at 15 threads is exact under
  every mode; relational is ~8% slower (1.84–1.88 s vs 1.67–1.76 s) because
  its dense points-to SCC has holes reading most members, so commits drain
  readers. Sharded native layouts (`shard_storage(layouts(sharded))`,
  `DL-ShardedLayout`) cut publication from 42% to 20% of step time: DOOP
  cassandra 15 threads 1.28 s compute, exact; evaluation (65%) now dominates.
- Open: reach chain at 15 threads below sequential; nested CQ code in one
  codegen unit; rule-local aggregates; FlowLog comparison; partial-Rust
  multi-round refinement; surface sugar (disjunction, facts, rule macros).
- On hold (GPU-side storage design first): lattices and custom providers;
  DOOP probe efficiency; geometric run merging.
- Not context facts (see context.tex): declared keys (need a provenance
  model), first-witness consumption, guard totality, terminal canonical
  return.
- Core boundary blockers are repaired: Sequence rewrites and shared selection
  helpers are core-owned; target-dependent conformance and workload tests live
  in `features/sequence-conformance`. `cq-testkit` has no optional compiler or
  feature backedge. Dependency isolation checks the actual closed core.
- `translate_pull` codegen still triggers `clippy::unused_unit` in workloads
  (GPU-side note).
- Sequence conformance: 31 crates (one per classic/retail case, pairwise
  triangle matrix per planner, 310 kernel rows instead of 2,848). Tasks code
  reaches Rayon only through non-generic `cq_storage::parallel` helpers. A full
  rebuild after a compiler change takes 143 s wall (1,355 s CPU, 3.7 GB peak
  RSS); running all 31 takes seconds.
- Archived: abandoned v1-design branches are tags `archive/<name>`; the parked
  pre-redesign refactor is `../mini_linq-archive/datalog-par-factor.patch`.

Approved core integration decisions:

1. Completed folder cleanup: HW4 is `tests/hw4_optional.rs`; YA is
   `passes/cq-reduce/tests/ya_schedule.rs`, with owning manifests and references.
2. Completed std-only `cq-testkit/golden`; DuckDB still enables it, while the
   Ascent oracle no longer enables bundled DuckDB.
3. Completed `runtime_rustc` backend selection by linked type identity, with
   zero/ambiguous-match diagnostics and regression coverage.
4. Evaluated but omitted `codex/d85-regression` (`46be507`). Paired SF1
   median-of-three default runs: D85 15.725 ms without vs 16.159 ms with;
   all 13 queries exact in both runs, with identical operator counters across
   all 26 query/configuration pairs. The changed branch chooser is used only
   by Filtered/Coarse, not the new Written/Coarse default, so it cannot improve
   that plan. Raw paired results are
   `features/workloads/target/bench/tpch-1-{before,with}-d85.{tsv,json}`.

Final integration SF1 comparison: release, one warm-up and median of five;
all 13 queries exact against DuckDB in both default and no-reduce configurations,
also checked at SF0.1. Times exclude MiniLinq loading. DuckDB 1T includes prepare.
Raw data: `features/workloads/target/bench/tpch-1-reduction-comparison.{tsv,json}`.

| Query | Rows | Default ms | No-reduce ms | DuckDB 1T ms |
|---|---:|---:|---:|---:|
| retail_h03 | 11,620 | 22.283 | 60.337 | 16.743 |
| retail_h05 | 7,242 | 21.146 | 74.753 | 18.230 |
| retail_h07 | 5,501 | 23.318 | 175.807 | 18.802 |
| retail_h07_block | 5,501 | 21.208 | 181.382 | 18.752 |
| retail_h08 | 2,603 | 19.034 | 39.450 | 12.769 |
| retail_h09 | 319,402 | 83.516 | 794.141 | 75.880 |
| retail_h21_positive | 302,284 | 48.979 | 505.158 | 48.532 |
| retail_d19 | 8,962 | 13.708 | 36.431 | 14.288 |
| retail_d27 | 357 | 11.902 | 10.416 | 13.187 |
| retail_d72_inner | 5,344 | 93.692 | 67.177 | 53.664 |
| retail_d85 | 8,962 | 15.529 | 1,225.156 | 19.103 |
| tpch_q06 | 1 | 17.136 | 17.099 | 8.961 |
| tpch_q19 | 1 | 12.828 | 12.804 | 108.081 |

Tooling notes: the external gate needs `TMPDIR=/private/tmp` on macOS; run
long suites under a hard timeout.
