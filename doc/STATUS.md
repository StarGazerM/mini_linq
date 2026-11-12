# Status

Audit baseline: `7e187597` (2026-11-02). Numerical measurements and execution
reports below are historical records from this document at that commit unless
a dated result artifact is linked; they are not new runs of the current tree.
Where raw artifacts are absent, the recorded numbers are not independently
reproducible. [PLANNING-REDESIGN Progress/P6b](PLANNING-REDESIGN.md) is the
newer evidence source; proposals under “Left work” and “Decisions needed”
are historical, not a replacement for that plan.

**Batch gates (2026-11-12, compression Phase 1a: GPU TPC-H key domains
`7abd4f14` vs `e4e08c99`):** keys become u32 codes in shared load-recorded
Offset/Rank domains (CHANGELOG, STORAGE). Only `experiments/gpu`, `testkit`,
`tools` and docs change; no IR grammar (`check_ir_scope` PASS for both IR
crates). Independent review "merge after fixes"; fixes applied.
- GPU gate on RunPod RTX 6000 Ada **PASS** (crates, radix-counters, H05 SF10
  8.478 ms ≤ 10.35, Cassandra 26/26).
- Paired query time, load excluded (5 alternating pairs, every sample exact
  under the independent checker;
  [raw](../experiments/gpu/compare/comp-tpch-domains-2026-11-12.pod-bench.json)):
  SF10 geomean **0.753 [0.741, 0.785]**, SF1 **0.842 [0.793, 0.894]**.
  h13 and h04_not_exists (0.13–0.21×) include a boundary change: their
  host-built inputs are now uploaded at load (`host_input_upload_ms`), not in
  `execute`. Without them: SF10 **0.856 [0.841, 0.897]**, SF1
  **0.945 [0.910, 1.009]**.
- Slower with CI above 1: h02 SF10 1.189 [1.169, 1.210] (grouped host
  download/decode boundary), h05 SF1 1.065, d85 SF1 1.096. Terminal packed
  publication and device-resident nullable grouped publication are the
  identified follow-ups.
- SF10 process wall 41.3 s vs 64.6 s (host H05 catalog removed).
- doop-gpu xalan (DOOP unchanged except shared radix runtime): fixpoint
  1.049 [1.036, 1.123], setup 1.052 [0.932, 1.107], output 1.046
  [0.982, 1.085]; provisional, not yet attributed.
- DOOP per-domain codes (`codex/comp-doop-domains`) are not landed: its
  fixpoint is 1.42× on xalan with output 0.48× (whole GPU evaluation 1.005×);
  the remaining cost needs a clean-GPU kernel profile.
- The pinned dataset revision is now `e29e21fd` (tools/README); the checker
  reads DuckDB-quoted TPC-H oracles (`tsv-quoted`).

**Batch gates (2026-11-11, P5 scan-contract salvage `227310ce` vs `61b73315`):**
three single-owner commits (Structures `a25bc3e8`, Sequence `f84b77e7`,
workload/runtime `227310ce`), independent review "merge".
- Expanded gate: only `tpch_q06` changes generated code (32 hunks: dead
  summary checks removed, explicit scan geometry, `row-par` route).
- `cargo xtask compile-tpch`: all 29 CPU kernels build.
- `tools/gpu_gate.sh` **PASS** (838 s; H05 SF10 8.392 ms; Cassandra 26/26).
- Q06 same-route query time vs baseline (9 pairs,
  [evidence](../experiments/storage-bench/results/p5-salvage-isolation.json)):
  Bytes serial 1.039 [0.982, 1.097]; Bytes Tasks-24 0.999; row serial 1.002.
  Plain serial is 1.058 at default link order and 0.995 [0.969, 1.013] with
  the same source relinked, so it is code placement.
- No route is promoted: `default`/`par` stay row-backed.
- The per-batch CPU TPC-H baseline is pending.

Also landed (`0464ff89`): the Q06 `aggregate!` build failure introduced by
`fadeecc5` is fixed, and the real Q06 dispatch is in the default `catalog` test.

**Per-batch GPU baseline (2026-11-10, P6 C GPU-first `2184de85` vs
`ef6ca71a`):** [raw results](../experiments/gpu/compare/p6c-2026-11-10.results.json),
STORAGE §12.7 and the P6 C tile paragraph. Only `experiments/gpu` and docs
changed, so no CPU baseline was run. Generated TPC-H code changes only in h06,
h11 and h15 (FOR-native tiles); all 52 plans are unchanged. Every run was
fresh-process, paired and validated, with no foreign GPU work in the samples.
Query time (load excluded):
- GPU TPC-H SF1 advised geomean **0.991 [0.989, 0.993]**: h06 0.766×, every
  query with unchanged code within [0.997, 1.007];
- h06 resident **0.771× [0.743, 0.796]** (SF1) and **0.809× [0.808, 0.811]**
  (SF10, CPU-exclusive);
- doop-gpu xalan fixpoint **1.012 [0.994, 1.026]** GPU-only and
  **1.014 [0.990, 1.045]** CPU-exclusive, so no confirmed effect.

Load time (separate cold-start metric, never counted as query time), from the
device cold load: h06 process wall **0.591×** (SF1) and **0.140×** (SF10);
suite process wall 24.3 → 18.3 s.

`tools/gpu_gate.sh` **PASS** (722 s; H05 SF10 8.347 ms; Cassandra 26/26).

**Per-batch GPU baseline (2026-11-10, compression batch `04776a2f` vs
`20f1f04e`):** [report](../experiments/gpu/compare/baseline-2026-11-10-compression.md),
[raw results](../experiments/gpu/compare/baseline-2026-11-10-compression.results.json).
No root-workspace source changed, so no CPU baseline was run. Generated TPC-H
code differs only in H05's untimed load calls. Five interleaved pairs plus five
CPU-exclusive confirmation pairs, all validated:
- GPU TPC-H SF1 advised geomean **0.999 [0.997, 1.001]** in both modes;
- doop-gpu xalan fixpoint **0.980 [0.964, 0.997]** GPU-only and
  **1.006 [0.999, 1.014]** CPU-exclusive, so no confirmed effect.

`tools/gpu_gate.sh` **PASS** (630.59 s; H05 SF10 8.285 ms; Cassandra 26/26).

**Per-batch CPU/GPU baseline (2026-11-06, `b23e6bb2` vs `96f32991`):**
[report](../experiments/gpu/compare/baseline-2026-11-06.md),
[raw results](../experiments/gpu/compare/baseline-2026-11-06.results.json).
Five interleaved pairs, all 29 TPC-H SF1 kernels: default geomean
**0.998 [0.989, 1.006]**, par **0.992 [0.977, 1.047]**.
DOOP xalan mini/mini-par: 26-output query **1.015 [0.949, 1.188]**,
field_flow **1.018 [0.944, 1.071]**. Bench-check exactness is full-first,
sampled-later; DOOP validation runs after collection, with no resident checker
during timing. The report records expanded plan movements, same-source linker
layout controls, per-command Gate times, and a native checker full-witness
memory defect exposed by field_flow's 103.7 million output rows.
**GPU completed:** 26 advised SF1 queries and both xalan GPU workloads, five
interleaved pairs plus five CPU-exclusive confirmation pairs. Confirmed TPC-H
geomean **1.004 [1.002, 1.008]**; H07 **1.102 [1.093, 1.108]** and
D72-inner **1.045 [1.039, 1.049]** are slower. Both DOOP fixpoint CIs include one.
All validation passes (26-output full tuples; field_flow 43-output full-first,
sampled-later). Main `525673c5` GPU batch gate **PASS**, **1066.22 s**:
H05 SF10 **8.375264 ms ≤ 10.35 ms**, Cassandra **26/26** complete output sets.
Evidence: `/data/mini_linq-wt/gpu-batch-1106/target/gpu-batch-1106/gpu-gate.log`
and `target/gpu-gate-h05-sf10.json` in that worktree.

**Historical paired measurement (2026-11-02/03, `44bf82bc` vs `c1b75718`;
[summary](../experiments/gpu/compare/bench-2026-11-02.md), raw samples in
[`bench-2026-11-02.results.json`](../experiments/gpu/compare/bench-2026-11-02.results.json)).**
Five interleaved fresh-process pairs (GPU TPC-H SF10: four). Historical TPC-H
samples used complete answer comparisons; DOOP timing samples checked all
relation counts after a separate full-tuple check, not full tuples in every
sample. Current timing policy uses a full first measured sample per
binary/configuration and exact counts plus seeded actual-tuple membership checks
thereafter; see
[`tools/README.md`](../tools/README.md#timing-sample-answer-validation).
Geometric-mean
current/reference ratios: CPU TPC-H SF1 `default` **0.908 [0.902, 0.917]** and `par`
**0.820 [0.800, 0.840]** (28 kernels). `retail_h07_block` does not compile on
`44bf82bc` (`sharded::Index::from_positions`, E0599). Apparent CPU slowdowns:
`default` `retail_h06` 1.026 [1.017, 1.043] and `retail_h20` 1.024
[1.015, 1.035]. Both fit the existing same-source bidirectional layout-control
envelope (h06 [0.9401, 1.1137]; h20 reverse [0.9794, 1.0451]; evidence in
the linked summary), so neither is a confirmed regression; this does not
establish the historical cause. CPU DOOP xalan: `mini` 8.97 s, 0.975 [0.969, 0.982]; `mini-par`
(24 threads) 2.04 s, 0.913 [0.891, 0.969]. GPU TPC-H advised (RunPod RTX 6000
Ada, every query exact): SF1 **1.085 [1.033, 1.120]**; SF10 **1.023 [1.000, 1.054]**
(4 pairs; all SF10 effects provisional/exploratory, below the five-pair minimum).
Observed ratios with CIs above 1 include h09 2.52/2.34, h21_positive
2.07/2.09, h10 1.55/1.61 and h03 1.26/1.22; apparent best h12 0.456/0.423 and h11
SF10 0.416. SF1 h08 1.048 and d72_inner 1.025 lack same-source layout controls
and are unconfirmed apparent slowdowns. The 2026-11-02 GPU TPC-H current
timing includes final answer download whereas reference excludes it: a known
boundary bug fixed separately, biasing cur/ref ratios against current.
DOOP GPU `doop-gpu` fixpoint (pod, 26/26 exact): xalan 0.239
[0.150, 0.599], batik 0.724 [0.482, 0.836]. These are low confidence:
fresh-process spread reached 10x, cause not established. No live SRDatalog
comparison is possible on the pod.
The GPU TPC-H ratios above are boundary-confounded, not confirmed execution
regressions: current charges terminal downloads to build/total and reference
does not. Separate download accounting restores a comparable boundary; the
historical samples lack the download field needed for arithmetic correction.

**Corrected GPU TPC-H checkpoint (2026-11-03):** separate terminal
`download_ms` accounting restores the reference boundary; intermediate transfers
stay timed. The local RTX 6000 Ada SF1 comparison versus `c1b75718` is
**0.948 [0.946, 0.949]** across 26 queries. This is exploratory evidence from
three exact pairs, not five-pair acceptance and not a corrected SF10 result.
H03 remains **1.141 [1.140, 1.142]**; its publication-write layout is under
investigation separately. [Boundary evidence](../experiments/gpu/compare/gpu-tpch-boundary.results.json)
supersedes the confounded SF1 headline, without rewriting those historical samples.
The DOOP pod also lacks both the live SRD toolchain and the prepared corpus for
`srd-doop-gpu`; its results cannot replace historical SRDatalog fullclock evidence.

## Where we are

- Structural planner step 1 implements I1–I3 checking on every executable
  VO and I2 drainage in shared ordering and the separate L&E producer.
  Elimination is connected-first; Transfer retains its declared written-binding
  preference through `WrittenConnected`. The final census matches all original
  124 I2 repairs plus 22 elimination repairs, and the complete authored-route
  supplement has zero unpredicted changes. Root, five-package, planners, quick,
  formal, API and all-29 compile-TPC-H gates pass. All 66 changed CPU routes
  pass SF1 DuckDB exactness and real untimed counters; all 232 expanded hunks
  are classified, with H15 default/par byte-identical.
  CPU xalan passes 26-relation full-tuple checks. Final GPU workspace/all-target
  compilation passes, including doop-gpu and the legal H07 advice mirror.
  GPU runtime acceptance subsequently passed on main `525673c5` (2026-11-06):
  all GPU crate suites, separate radix-counters suite, H05 SF10 ceiling, both
  DOOP builds and Cassandra 26/26 complete tuple sets; see dated baseline above.
  Evidence: `/data/design-notes/planner-step1-census.md` and the worktree's
  `target/planner-step1-gates.json`. No performance comparison is claimed.
  Revert pending merge (`codex/unleak-order`): all step-1 edits outside the VO
  layer are undone and the `after [..]` provenance is removed; D85 (transfer) fails
  correspondence, H05/D72/H02/H20 realizations change, one planners test fails.
  `/data/design-notes/planner-step1-revert.md`.
  Datalog seed/delta, driver and permutation are recorded facts; optional
  stratum demand is a no-op. Main explicitly retained Incremental-prefix
  Storage and delta permutation in this step; single CQ order ownership and
  VO-derived Storage are step 1b after a separate census.

- P7.2 merged in `514ef6af`, advice-only: Structures records
  `Ordinary` versus `Statistics(BuildDemand)` construction, checked statistic
  publications and borrowed/attached filters. Ordinary builds retain their
  no-statistic-work path; automatic activation remains P7.3 design work.
- Development uses three workspaces (formerly 28): root for portable core,
  features, formal examples and CPU benchmarks; `experiments/gpu` for oxide;
  `experiments/mac` for Metal dependencies. Standalone packaging fixtures remain
  intentionally isolated. Root default members exclude five counter-enabled
  packages; explicit regression rows cover them.
- The RunPod lane in `tools/remote_gpu` builds bundles locally, seeds a persistent
  volume from a pinned HF revision, executes on disposable GPU pods and pulls
  results with a 60-minute deletion watchdog. DuckDB/HF dependencies travel in
  the bundle; session pruning keeps old bundles from filling the volume.
  See [tools](../tools/README.md) for prerequisites and lock protocol: shared
  build/test locks per command, GPU-only timing locks unless confirming a
  near-neutral claim under CPU exclusion.
- Contract 7: `if`/`let`/`for` embed unchanged opaque Rust, pure and total by
  premise, placed by free variables only. Explicit `where` owns relational
  comparisons, membership, ranges and boolean predicates; closed Rust operands
  are evaluated once at host. Types are opaque host syntax, and GPU Rust
  eligibility uses recorded identity representations plus closed captures.
  Formal rules, inventory and conformance records describe this cutover;
  updated conformance rows are explicitly unverified pending integration.
- The checked IRs and pass protocol are implemented. Recorded-decision and
  independent-checker gaps remain tracked in `PLANNING-REDESIGN.md`; this is
  not a claim that every contract or external-consumer gate is complete.
- Default pipeline (`mini_linq::passes::plan`): left-deep connected growth
  (Connected/Coarse), not written-order scheduling. The first scheduled clause
  of each scope streams; later clauses are indexed and probed.
  `plan(no_reduce)` keeps that scheduling policy but emits no semijoins.
- Sequence is the shipped CPU target: `sequence` → `sequential` →
  `translate_pull`, including profile counters. Explicit Structures push
  terminals remain selectable without Sequence phases. The former direct
  Structures pull terminal and its compile APIs are removed; shared
  realization helpers remain for the push printer.
- Reduction (Masks): each indexed clause is reduced by its filtered subtrees of
  the stream-rooted reduction tree (GYO join tree, or GYO ears plus a spanning
  tree of the cyclic core), children first, before the stream starts: the
  bottom-up Yannakakis pass restricted to filter-carrying subtrees, no
  top-down pass. Unfiltered (key/foreign-key) edges are not reduced, so it is a
  full reducer only when those edges remove nothing; `Tree` is the
  unconditional full reducer. Exact atom-to-block cuts on any scope. Each block
  branch is its own scope.
- Predicates: relational disjunctions derive implied per-occurrence domains
  (H07 `n_name ∈ {FRANCE, GERMANY}`) while keeping the exact predicate.
  Explicit `where` comparisons and `&& || !` are relational filters.
  Opaque Rust guards are type-checked by rustc and placed by free-variable
  dependencies, not kept where written (`#[total]` and the allowlist were removed).
  `!R(args)` is core negation, elaborated to an anti-join (`doc/formal/guards.tex`).
- Aggregates: one required set dedup; constant-group folds use `SingleGroup`;
  grouped containers use the recorded fast hasher.
- Join filters (SQL Server bitmaps): eligible probes or semijoins into filtered
  builds push exact key sets to scans reached through their probe spines.
  Structures records filter candidates and one activation policy per filter:
  `policy abandon P of B keys key_count` (default `35` for a positional lineage
  base) or `policy always`. The checker validates the percentage, base and key
  source; the runtime applies that recorded threshold to exact distinct keys
  versus base occurrences. `always` filters never abandon. The exact probe
  still runs.
- Probes: Swiss-table `HashIndex`; batched (128) prefetched probes on scan
  spines, scalar elsewhere; no per-row `Rc`.
- Back end (`doc/formal/backend.tex`): three recorded layers over one
  Physical program. R, representation: `r repr = tuples | positions {..}
  owned [..]` (data refinement, α fetches columns by lineage). E, evaluation
  unit: `r stage = batch N` (a representable indexable segment runs at most
  N rows ahead; unsupported batch operators end a stage, not a Rust-totality
  category). C, control: pull (unfold) or push (fold) of the same stream algebra,
  an emitter choice. Default passes `represent` and `stage` record today's
  decisions only (the probe lookahead is now `stage = batch 128` in text);
  `represent(positions)` and `stage(batch(N))` opt in. Printers bind only
  the columns an operator reads (`cq-print` column environment). Positions
  stop at probes: carrying a `RowId` through an index payload is deferred.
- General provider optimization contracts are recorded as
  `supports [retain, expand, keys]`, not inferred from implementation names.
  Native emission checks cursor/key traits; selection and replay replace support
  with the provider and preserve live obligations. Target switches reject
  decisions a terminal cannot realize rather than silently discarding them.
- Host reads and builds use direct calls selected by per-reader Structures
  facts, not autoref tiers, `build!`, or `AutoDense`. Declared native trait
  mismatches are rustc errors; undeclared reads use recorded lazy fallbacks.
  Captures are plain owned values cloned per consuming move closure, not
  compiler-generated `Arc` handles. The written reference belongs to
  `cq_order::compile_written`, not `cq-relational-lowering`.
- Mac: standalone Metal build/merge experiments are integrated with the optimized
  native RadixRun tree. The [Mac implementation plan](MAC-IMPLEMENTATION.md)
  includes flat and LSM-like maintenance, snapshot/publication obligations and
  compiler boundaries. Production Metal providers, online maintained runs and
  a Metal Sequence backend remain planned, not implemented.

## Gap to DuckDB (SF 1, single thread)

`cargo xtask bench --tpch --sf 1 --runs 5 --reduction-comparison` from
`features/workloads`, release, one warm-up, median of five. MiniLinq excludes
load; DuckDB includes prepare. All configurations exact vs DuckDB at SF 1.
The raw artifacts for this historical table are not present in this
checkout, so it is not independently reproducible from checked-in evidence.
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

In this historical comparison, no-reduce kept written (fact-first) order, so without reductions the stream
probes unreduced builds. D72's one reduction (Lineitem ⋉ PartSupp on a
non-selective `ps_availqty > 0`) costs 32 ms of composite probes into the
800k-row PartSupp index and filters nothing.

## Predicate transfer route (`cq_planners::order(transfer)`)

Exact-semijoin adaptation of RPT (SIGMOD'25), through the same public pipeline:
connected growth within written atom runs, Masks forward pass plus the stream's
reductions, then the backward pass from filtered sides, before the join phase.
Predicate, binding and block boundaries stay fixed. A top-level VO `Guard`,
`Let` or `For` makes `transfer` return the Masks plan unchanged because the RA
correspondence checker rejects some otherwise sound transfer schedules; inner
block scopes also retain Masks reductions. This is not the paper's vectorized
Bloom pipeline or RPT+'s adaptive execution.

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
the outer stream remains explicit. Expansions wait for column demand or scope
exit. Rust placement requires free variables, and lexical opacity retains the
whole environment through `FreeVariables::dependencies`.
Blocks recurse without changing their interface or fallback ancestry.

Coarse expansion is deliberate: CK-Le checks exactly one full expansion per
non-stream occurrence, rather than claiming Fine's one-variable obligation.
It independently checks the non-introducing lookup before that expansion and
written column ownership; WF-VO checks first uses, complete rows and free-variable
bindings, Masks checks reductions, and the CQ-to-RA correspondence remains active.
No RA′, VO, Physical grammar, runtime feature, or core compiler dependency is
added. `physical = [cq_rewrites::smallest_intersections]` remains the public
intersection hook; this coarse planner does not manufacture Fine ranges or
claim a worst-case-optimal join bound.

The default planner also accepts hand-authored step advice:
`plan(order(Seed, R@1(x), S@2(x), R@1, S@2))`. Parentheses list the exact
retained query columns (all bound columns must remain); bare selectors expand
the full occurrence. A bound-only partial read is a lookup; an introducing
partial read is Fine and may bind one variable. For example
`order(Seed, R@1(a,x), S@2(a,x), R@1, S@2)` exposes an intersection on `x`.
Named partial occurrences require an explicit complete read; unnamed
occurrences retain written priority. The existing CK-Le checker now lives in
`cq-order`, shared by both entry points: selected lookup occurrences require
one lookup before one full expansion. Only automatic LE requires written column
ownership and the written outer stream; advised schedules choose their first
read as the stream, with column introductions checked by WF-VO. WF-VO checks
every read and dependency. Without advice the default path is unchanged;
advice does not infer Rust types or expressions.

The retail runtime registers `le` and `le_runs`. The e2e corpus carries both
as planner axes (538 new accepted configurations; the two D85 `merge_joins`
+ HashMap witnesses are excluded by the existing accepted-domain policy and
pinned as negative UI fixtures); the corpus gate passes (3,440 tests).

`le_runs` explicitly applies `sorted_runs`; core default selection is
unchanged. Sorted-run retyping selects `SortedRun<_, _, I64Codec>` for eligible
one-level HashIndex payload builds with one stored key, without inspecting
Rust key types. The provider explicitly spells the codec; rustc enforces its
trait bounds rather than selecting another codec. An over-budget span can
still fall back to binary search within that recorded codec at runtime.

The Mac experiment also exposes checked `cq_rewrites::radix_runs`, selecting
native `cq_storage::RadixRun<_,_>` for eligible singleton i32/i64/u32/u64 payload
builds, including owned/computed keys. This is radix12/error32 spline lookup,
not `SortedRun` binary/rank lookup. It composes with retained cursors and
explicit segmented expansion; Sequential capability is Exact/Range/Membership,
not Ordered or GPU. Default selection and the existing `le_runs` preset are
unchanged. See [native execution and measurements](../experiments/mac/README.md#native-backend-radixspline-provider-and-processing-advice).

The CPU TPC-H harness offers `--index-comparison` for all 29 current kernels.
Ordinary radix advice can leave host fallbacks unchanged; matched
`built_hash`/`built_radix` configurations make explicit materialization common
to both arms on supported kernels. H09's authored order now avoids its independent
Part/Orders prefix, using local SF1 cardinalities and an index-width comparison.
The Part substring guard remains first. Earlier H09 timings below and in the
GPU report are historical, not timings for this revised CPU order.

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

0. **CPU H20 `par`: large single-threaded partial-group merge (2026-11-10).**
   H02 `par`'s 1.53× regression from the canon merge is fixed by
   `PL-GroupPartial` (partial groups only where Physical records no canon on
   the group's input; H02 back to group-key routing, 0.54× of the regressed
   time). H20 `par` keeps the partial schedule: its group needs no canon, has
   about 1.7 rows per key (909,455 rows, 543,210 keys), and the merge runs on
   one thread. Sequence plans record no row or key counts, so the choice needs
   the M7 cost model or a parallel merge. Evidence:
   `design-notes/par-regression-report.md`.

1. **D72.** Its decisive choice is data-dependent: `ps_availqty > 0` keeps
   every PartSupp row. The current `advised` recipe in
   `features/workloads/src/queries/retail/tpch/retail_d72_inner.rs` is
   `plan(no_reduce, order(Nation, Supplier, Lineitem, Orders, Customer, PartSupp, Part))`.
   The historical 141–145 ms result used the former
   `skip(Lineitem with PartSupp), reduce(PartSupp with Lineitem)` recipe,
   not this current order. Choosing such advice automatically needs statistics
   or a runtime check.
2. **Scans.** Historical Q6 profiling measured 28.0 ms versus DuckDB 1T
   19.1 ms, with costs attributed to the old `from_fn` positional scan and a
   canon `HashSet`. These are not measurements of the current implementation.
   `BlockRead::read_owned` now constructs the named `BlockScan` iterator
   (`runtime/cq-storage/src/positional.rs`), with offset state and a dedicated
   `fold` loop. Sequence `Emitter::canon`
   (`passes/cq-sequence-pull/src/lib.rs`) realizes the recorded canonical
   provider rather than prescribing that historical HashSet. The remaining
   scan/canonicalization gap needs fresh measurements; the old attribution
   does not establish current bottlenecks or the benefit of columnar storage.
3. **Positional default.** Selections need the caller's `positional(..)` advice.
4. **Zone maps.** The historical `hypergraph_zone`-only limitation is superseded
   by P5 S2's columnar `ScanTable` loaders and block summaries. Runtime support
   is not a claim of default compiler scan selection; see `DECODE-AUDIT.md`.
5. **Known limits.** The default sequential target is not parallel
   (`sequence-tasks` and `datalog-parallel` provide Rayon realizations).
   Explicit keys/counts and evidence-driven choices now exist; their coverage
   is described in `PLANNING-REDESIGN.md`, rather than “no statistics/keys”.

### Decisions needed

1. **Check policy.** Checks that recompute the same decision
   (`join_filter_candidates`, cq-order rank, CK-Masks, branch equality,
   block_contract) vs obligation checks (cq-reduce Tree/Hypergraph). Decide per
   pass.
2. **Derived facts in Structures text** (`Origin`/lineage, `constants`,
   membership `build/key`, fixed `join_filter` fields): drop or keep.
3. **Knob freeze.** No new out-of-band knobs (`plan(..)` options, batch size,
   Bloom size) until plan control is designed.
4. **Pass ownership.** Expose the default plan chain as a library fn so
   planners/fixtures stop restating it. (The checked RA rewrites
   `simplify_reductions`/`simplify_expansions` now live in `cq-ra-rewrites`.)
5. **Artifacts.** Regenerate `features/workloads/data` (26.7k CSV lines) at test
   time unless handouts need browsable CSV; stop committing feature
   `Cargo.lock`s (ignored by `check_external.py`); drop or narrow API snapshots
   (filter `kw::*`/auto-trait/tuple-macro rows); shrink
   `regression-targets.tsv` to rows `cargo test --workspace` does not cover.
6. **cq-load split.** Move `ColumnAt` and slice impls to cq-storage.

## Next step

Follow the current steps in `PLANNING-REDESIGN.md`. The earlier proposal to
abandon reductions by selectivity is distinct from the implemented, recorded
join-filter abandonment policy described above.

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
- Dedup ownership: Storage admits full-row sets before publishing CQ inputs;
  Round records `admit R from [holes]`, authorizing `consumer canonical`
  (`doc/formal/context.tex`). Rule holes build no local `__cq_seen`.
- Negation is core `!R(..)` (anti-join, `doc/formal/guards.tex`) over key sets
  built once per stratum; zero per-call index builds.
- Fx hashing is the default; keyed `cq_storage` providers, negation key sets
  included, via `hasher(keyed)` (`mini-keyed` engines).
  Storage's `Negative` record owns each negation provider and hasher;
  there is no `hasher_round` rewrite.
- Compile time (base DOOP crate): macro expansion 139 → 3.4 s (holes declare
  only what they read, one check per pass, linear source checks, optimized
  proc macros); codegen partitioned per component/hole; one crate per engine.
- Parallel extension: partition Round rewrite + Rayon printer, sharded
  admission, per-layout exclusive publication, `self_dedup per_task` through
  the checked push-terminal rewrite (Structures `print_push`, because the
  Sequence back end has no supplied-consumer entry; see `translate` and
  `Emitter::finish` in `passes/cq-sequence-pull/src/lib.rs`). Historical: DOOP at 15 threads ≈ 0.8 s (cassandra), 2.3 s (xalan)
  (superseded for xalan by the 2026-11-02 paired record above: `mini-par` 2.04 s at 24 threads);
  all six apps exact vs Ascent sequentially and at 15 threads.
- Schedule: Round records per-hole read sets, wave partitions, per-hole
  policy, derived publication placement and a progress mode
  (`WF-DL-Schedule`, `WF-DL-Progress`, `DL-WaveCommute`,
  `DL-PublishAfterReaders`, `DL-AsyncCovering`, `CK-DL-Schedule`,
  `P-DL-Waves`, `P-DL-Relational`, `DL-RelationalScheduler`;
  `doc/formal/datalog.tex` "Schedule records", "Relational progress"). Waves
  are Rayon scopes on CPU and recorded stream lanes on GPU (`DL-WaveLanes`);
  admission joins lane work before publication. `relational` progress drops
  the CPU step-round barrier: per-member batches, tiled delta windows, scheduler-gated
  isolation (no lock ever waits). DOOP cassandra at 15 threads is exact under
  every mode; relational is ~8% slower (1.84–1.88 s vs 1.67–1.76 s) because
  its dense points-to SCC has holes reading most members, so commits drain
  readers. Sharded native layouts (`shard_storage(layouts(sharded))`,
  `DL-ShardedLayout`) cut publication from 42% to 20% of step time: DOOP
  cassandra 15 threads 1.28 s compute, exact; evaluation (65%) now dominates.
  Sorted-run layouts (`layouts(runs)`; `DL-RunLayout`) provide the CPU run-layout
  reference for GPU Datalog storage; `seen(runs)` is rejected and there is no
  separate `DL-RunAdmission` rule. The historical cassandra run measured 3.11 s
  (not a CPU default).
- GPU Datalog (`experiments/gpu/datalog`, `cq-gpu-datalog`): `device_storage`
  (`CK-DL-DeviceStorage`) + `batch` hole terminal `translate_arena` over
  radix relation inputs (`GPU-Runs`, `GPU-Arena`) + `print_rounds` (`P-DL-Gpu`)
  (`DL-DevicePacking`, `DL-DeviceRuns`). Historical acceptance:
  the 13-program oracle was exact on the GPU (416 comparisons). Random graph,
  2048 nodes, ~4.1M answers: TC 0.36–0.49 s vs 1.58–1.79 s mini /
  1.83–2.18 s mini-par; same generation 0.41 s vs 3.58 s / 3.07 s.
  That checkpoint issued waves on one stream. P4-GPU slice D now uses
  ordinary full-key `DeviceRadixRun<W, u64, ()>` state and `RadixLayout` views:
  read-only `fresh` returns a `SortedBatch`, and `append_sorted` publishes it.
  Device records independently check offset, construction, prefix reads,
  directory policy and geometry. Slice-D gates/timings are not claimed here.
  Scheduling can record multiple lanes; see the dated P6b evidence.
  Historical DOOP cassandra acceptance: all 26 GPU outputs equaled `mini` exactly:
  strings stay in the dataset (`method_descriptor` interned at load, symbol
  columns as `SymbolCodec` u32 codes, closed constants captured on the host),
  wide inputs use multi-lane words. 11.8 s vs 4.27 s `mini` / 1.26 s `mini-par`.
  nsys: kernels 7.7 s (one hole 4.07 s: 306 × 13.3 ms), PTX module JIT 5.6 s
  (158 × 35 ms); readback transfers 21 ms, the rest of D2H time is waiting.
  Subsequent historical checkpoint: AOT sm_89 cubins (module loading 0.12 s) and delta-first occurrence
  order (`driver(delta_first)`, `DL-OccurrenceOrder`): GPU DOOP 3.67 s warm,
  kernels 1.90 s; largest remaining kernel is the field-load delta variant.
  Host overhead cut (no arena fences, `reserve_previous`, known-unique sorts):
  3.14 s; ~1.0 s of inter-kernel host time is CPU work outside CUDA calls.
  Before AOT: cold first run after a rebuild ~25 s (driver PTX JIT cache empty). The GPU
  workspace pins pushed forks (cuda-oxide and RCCL `mini-linq/main`), Rust
  1.99.1, CUDA 13.0.
  Fresh local SRD comparison (2026-11-03, unmodified `dbe4c2cb`):
  xalan's same 43-IDB field-flow fixpoint is **0.880 s** in `srd-doop-gpu`
  versus **1.187 s** in live SRDatalog `fab849f`, paired ratio
  **0.741 [0.598, 0.943]** (five rotating rounds, exact outputs).
  Batik has only one completed observation (**1.929 / 1.844 s**): the user
  stopped further runs, so no Batik ratio/CI is claimed. These replace the
  historical Oct 26 default-query ranges as the latest live observations,
  not as a five-pair Batik result. End-to-end boundaries differ; the report
  shows phases separately and does not claim an end-to-end speedup.
  The distinct 26-output `doop-gpu` workload is reported without any SRD ratio.
  See [local SRD report](../experiments/gpu/compare/srd-local-2026-11-03.md)
  for raw evidence, load/lock accounting and the late root-bound membership
  finding in the default planner's array-store delta variants.
- Open: reach chain at 15 threads below sequential; nested CQ code in one
  codegen unit; rule-local aggregates; FlowLog comparison; surface sugar
  (disjunction, facts, rule macros). Pure-total Rust remains a semantic premise.
- On hold: lattices and custom providers; DOOP probe efficiency.
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
  triangle matrix per planner, 310 kernel rows instead of the current 1,424-row
  executable product after removal of the ordered-dedup axis). Tasks code
  reaches Rayon only through non-generic `cq_storage::parallel` helpers. A full
  rebuild after a compiler change takes 143 s wall (1,355 s CPU, 3.7 GB peak
  RSS); running all 31 takes seconds.
- Archived: abandoned v1-design branches were recorded as tags `archive/<name>`.
  No checked-in evidence artifact for the abandoned parallel-factor patch
  is available in this checkout.

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
   by Filtered/Coarse, not the current Connected/Coarse default, so it cannot
   improve that plan. Raw paired results were reported as
   `features/workloads/target/bench/tpch-1-{before,with}-d85.{tsv,json}`.

Historical final integration SF1 comparison: release, one warm-up and median
of five; reported all 13 queries exact against DuckDB in both default and
no-reduce configurations, also checked at SF0.1. Times exclude MiniLinq loading.
DuckDB 1T includes prepare. Raw data for this historical comparison is
absent from this checkout; these numbers do not measure the current planner.

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
