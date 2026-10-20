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
   `regression-targets.tsv` to rows `cargo test --workspace` does not cover.
6. **cq-load split.** Move `ColumnAt` and slice impls to cq-storage.

## Next step

Decide whether runtime adaptivity (abandon reductions whose builds keep over
35% of their source) should choose such reductions without advice.

## Datalog (`codex/datalog-v2`)

Rule: features build on the public core API (`languages/`, `passes/`,
`runtime/`, `facade/`, `testkit/`, root manifests). Core changes require user
approval; this integration admits only the four decisions below.

- `features/datalog`: `language` (AST, contract, `Language`), `schedule`
  (strata, semi-naive with declared deltas, recorded insertion, negation keys
  and index layouts), `fork` (ordinary CQ holes), `bridge` (`datalog!`,
  `stratified!`, `seminaive!`; round loop), `oracle` (sequential Ascent,
  naive evaluators, 104 Ascent-generated goldens). Negation is core
  `!R(..)` over completed strata; aggregates and nested blocks are rejected.
  `cq-ext-recursion` is unchanged.
- Indexes: Datalog keeps its relations as grouped-map hosts in the layouts it
  records, updates them after each round, and passes them to one-shot CQ
  holes; the existing compiler probes host indexes natively. Reach chain at
  n = 2,000/4,000/8,000: 0 core index builds, rows enumerated 3n + 1.
- Single-thread vs sequential Ascent, uninstrumented (`e981103`, two runs):
  linear TC chain 1,024 18.7 vs 18.9 ms, chain 4,096 341–344 vs 497–521 ms;
  nonlinear TC chain 512 78.5 vs 108 ms; reach chain 4,096 0.37 vs 0.27 ms;
  reach random 5k 0.41–0.43 vs 0.37–0.41 ms; same-generation chain 4,096
  0.40–0.43 vs 0.25–0.27 ms. Earlier numbers were built with `counters`,
  which selects the per-operator profiled code.
- Core tension (reported, not changed): each rule query deduplicates its own
  answer per call (`__cq_seen`) although Datalog deduplicates again. Even
  with a head returning every body variable, lowering marks the returned
  join canonical because a host read (`Op::Rename`) is never known unique
  (`passes/cq-physical-lowering/src/lib.rs:74-76,315-326`;
  `languages/cq-lang-structures/src/lib.rs:809-816`). Removing it needs a core
  way to declare a host duplicate-free or to hand deduplication to the
  consumer. This is the remaining same-generation gap.
- Workload tests are plain case files (`features/workloads/src/queries`):
  query once, input helpers, golden check; pairwise CQ matrix (329 pairs per
  family, named configurations) plus a 2,400-configuration triangle kernel;
  rejected pipelines are UI fixtures. Workload tests 14,357 → 2,982; cold
  test build 874 → 169 s.
- Integration validation (`codex/integrate-upstream`): root 517, Datalog 57,
  extensions 53, workloads 2,982; 151 byte-exact DuckDB goldens; dependency
  isolation/frontends/consumers; 21 unchanged public-API snapshots; packaged
  external builds/tests for all 11 features and mandatory fixtures; formal
  inventory 155 rules. Datalog needed no API adaptation. Cargo ran serially
  with two build jobs and separate root/feature targets.
- Pre-integration SF 1 (one run): Q19 22.5, D19 12.7, D27 16.3, D72 75.9, H08 17.9,
  H09 106.7, H03 28.1, Q6 16.5, H05 35.6, H21 107.3, H07 44.2, D85 62.8,
  H07_block 72.6 ms; all exact. The H08/D19 slowdown seen on the abandoned
  `codex/datalog` branch is absent, so it came from its core changes.

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

Tooling notes: `tools/check_public_api.py` reads `ROOT/target/doc` regardless
of `CARGO_TARGET_DIR`; the external gate needs `TMPDIR=/private/tmp` on macOS.

Datalog compiler structure (`codex/datalog-redesign`, spec
`doc/formal/datalog.tex`): checked languages Source, Strata, Incremental,
Storage, Round (each `cq_stage::Language`), passes stratify / incremental /
storage / fork / print_rounds plus the `if let` surface pass, wired by a
Datalog-owned named configuration; shared printer library
`cq-datalog-print`; resume by entering any pass macro. Default expansion
byte-identical to `076bb40`.

Admissibility (design step 3, `95acd2d`): contracts admit any correct
choice (coarser strata without internal negation, at-least-once variant
coverage, any layout set whose recorded reads have bound or constant keys);
`check_rewrite` checks real per-IR preservation, so Source / Strata / Storage
rewrite plugins are live (test plugins in `features/datalog/test-passes`).
`cq-datalog-testkit` adds a typed naive reference evaluator with ⊥ as panic.
Open: hole interfaces accept only `pull` printers on the configured
allowlist. Storage/Round now record every former printer decision (providers,
maintenance, phases, publication order, termination, observer placement, key
scope, output conversion; `7360ce8`, `11f7d59`); remaining printer premises:
empty `Default` construction for seen sets and layout hosts, slice-backed
sources for the parallel partition.

Remaining Datalog work: rule-local aggregates; FlowLog comparison;
partial-Rust multi-round refinement.

Parallel Datalog is the extension workspace `features/datalog-parallel`
(partition Round rewrite + Rayon printer, selected by pipeline config; base
has no parallel code). Slice 2 (`cfa8201`, `ed3831f`): full-tuple sharded
`seen` with parallel admission; native layout maps published in parallel,
one exclusive task per layout (fully sharded layouts were measured and
rejected: per-probe shard routing slowed evaluation); adaptive grain
`max(64, n/(4·threads))` fixed the avrora plateau (coarse 1024-row chunks on
a small, expensive driver); recorded inline threshold. 15 threads, seconds
(slice 1 → slice 2): cassandra 1.33 → 0.84, avrora 2.66 → 1.63, lusearch
1.63 → 1.10, xalan 3.74 → 2.57, tomcat 1.74 → 1.14, zxing 2.80 → 1.99.
Residual: reach chain 4096 still 0.74× sequential at 15 threads. Next,
adopted from Ascent after measurement: Fx hasher plugin, hash-once admission,
delta layouts built during admission, heavy-bucket splitting.

On hold until the GPU-side storage work settles, so storage is designed once
for both: lattices and custom providers; DOOP probe efficiency (flat value
storage, batched/prefetched probes on Datalog hosts; probes are 43–46% of DOOP
compute at `8dc918a`); geometric run merging.

Pending core fix (approved to record, not yet designed): let lowering know a
host is duplicate-free, or let a consumer own a query's deduplication, so
Datalog rule queries stop building a per-call `__cq_seen` set (see the core
tension above). Expected to close the same-generation gap to Ascent.

The abandoned design (`codex/datalog`, `codex/datalog-lang`,
`codex/maintained-structures`, `codex/datalog-*`, `codex/tests-reshape`,
`codex/cleanup-folders`, `codex/testkit-backend-identity`) changed core and is
kept only for reference.
