# Status

## Where we are

- Semantics, IRs, checks and the pass protocol are complete. Every decision is
  recorded in IR text, and all features build outside the repository.
- Default pipeline (`mini_linq::passes::plan`): static Filtered/Coarse ordering,
  checked Hypergraph reduction, redundant-support absorption, then static build
  orientation recorded in RA before IX. `plan(no_reduce)` keeps the ordering and
  orientation but emits no semijoins.
- Reduction: tree reducer only on acyclic scopes, and only for edges whose
  support carries a filter from outside the target subtree; exact atom-to-block
  cuts on any scope. Each block branch is its own scope.
- Predicates: relational disjunctions derive implied per-occurrence domains
  (H07 `n_name ∈ {FRANCE, GERMANY}`) while keeping the exact predicate. Guards
  over allowlisted total std operations, or marked `#[total] if <expr>`, move
  like relational filters; other Rust guards stay where written.
- Aggregates: one required set dedup; constant-group folds use `SingleGroup`;
  grouped containers use the recorded fast hasher.
- Join filters: a small build side's keys filter the streamed relation's scan
  (exact key set, or Bloom covered by the exact probe), recorded in Structures.
- Bushy branches: independent dimension branches recorded in VO, built once.
- Probes: Swiss-table `HashIndex`; batched (128) prefetched probes on scan
  spines, scalar elsewhere; no per-row `Rc`.

## Gap to DuckDB (SF 1, single thread)

`cargo xtask bench --tpch --sf 1 --runs 5 --reduction-comparison` from
`features/workloads`, release, one warm-up, median of five. MiniLinq excludes
load; DuckDB v1.5.5 includes prepare. All configurations exact vs DuckDB at
SF 0.1 and SF 1. Raw data: `features/workloads/target/bench/tpch-1-reduction-comparison.{tsv,json}`.

| Query | default ms | no-reduce ms | DuckDB 1T ms | default / DuckDB |
|---|---:|---:|---:|---:|
| tpch_q19 | 21.7 | 49.2 | 106.0 | 0.20 |
| retail_d19 | 13.7 | 13.8 | 14.3 | 0.96 |
| retail_d27 | 15.4 | 15.7 | 12.7 | 1.22 |
| retail_h08 | 18.6 | 18.9 | 12.7 | 1.47 |
| retail_d72_inner | 78.1 | 78.2 | 52.9 | 1.48 |
| retail_h09 | 106.5 | 110.3 | 70.4 | 1.51 |
| tpch_q06 | 16.7 | 16.9 | 8.5 | 1.96 |
| retail_h03 | 34.4 | 32.9 | 16.5 | 2.08 |
| retail_h05 | 40.7 | 41.0 | 18.1 | 2.24 |
| retail_h21_positive | 112.4 | 164.2 | 47.1 | 2.38 |
| retail_h07 | 44.1 | 105.2 | 18.5 | 2.39 |
| retail_d85 | 61.4 | 61.8 | 18.7 | 3.28 |
| retail_h07_block | 71.5 | 90.9 | 18.8 | 3.81 |

Before this work (1a6460c) the same defaults were 2.5–48× slower than DuckDB
(H07 880 ms, H09 2,170 ms, H05 269 ms).

## Left work

1. **D85 regression.** The integrated baseline remains 61 ms. On isolated
   `codex/d85-regression`, deferring a weak composite bushy branch behind a
   stronger disjoint-key filter measured 19.7 ms. All 13 queries remained
   exact; the branch's full workload suite passed. The fix is not integrated.
2. **H07/H07_block/H05 build side.** Orders and Lineitem are still indexed where
   DuckDB streams them against small accumulated branches.
3. **H21.** Both Lineitem aliases are indexed.
4. **Scans.** Scalar row-at-a-time over 48-byte rows (Q6 2×); columnar scans.
5. **Positional default.** Selections need the caller's `positional(..)` advice.
6. **Known limits.** Static heuristics only (no statistics); no keys or
   constraints on data; no parallel execution.

### Decisions needed

1. **Check policy.** Checks that recompute the same decision (`orient`,
   `join_filter_candidate` + literal budgets, cq-order rank, branch equality,
   block_contract) vs obligation checks (cq-reduce). Decide per pass.
2. **Derived facts in Structures text** (`Origin`/lineage, `constants`,
   membership `build/key`, fixed `join_filter` fields): drop now or with Q5.
3. **Knob freeze.** No new out-of-band knobs (`plan(..)` options, batch size,
   Bloom size) until quotation Q1/Q5; do Q2 (public `cq_stage::Input`) before
   Q1 since `SurfaceInput` rejects a `quotes {}` slot.
4. **Pass ownership.** Move `orient`/`simplify_reductions` to an RA rewrite
   home; expose the default plan chain as a library fn so planners/fixtures
   stop restating it.
5. **Artifacts.** Regenerate `features/workloads/data` (26.7k CSV lines) at test
   time unless handouts need browsable CSV; stop committing feature
   `Cargo.lock`s (ignored by `check_external.py`); drop or narrow API snapshots
   (filter `kw::*`/auto-trait/tuple-macro rows); shrink
   `regression-targets.tsv` to rows `cargo test --workspace` does not cover.
6. **cq-load split.** Move `ColumnAt` and slice impls to cq-storage.

## Next step

After the runtime work is integrated: staged quotation for plan control, in
[`QUOTATION.md`](QUOTATION.md).

## Datalog (`codex/datalog-v2`)

Rule: features build on the unmodified core (`languages/`, `passes/`,
`runtime/`, `facade/`, `testkit/`, root manifests). A needed core change is
reported with evidence first. On this branch the core diff against
`codex/all-answers` is empty.

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
- Single-thread vs sequential Ascent (`a7b4a7c`, one run): reach chain 4,096
  0.71 vs 0.38 ms; reach random 5k 0.57 vs 0.40 ms; linear TC chain 4,096
  618 vs 515 ms; nonlinear TC chain 512 91 vs 106 ms; same-generation 4,096
  0.65 vs 0.25 ms.
- Workload tests are plain case files (`features/workloads/src/queries`):
  query once, input helpers, golden check; pairwise CQ matrix (329 pairs per
  family, named configurations) plus a 2,400-configuration triangle kernel;
  rejected pipelines are UI fixtures. Workload tests 14,357 → 2,982; cold
  test build 874 → 169 s.
- Validation: root 507, datalog 57, extensions 53, workloads 2,982 (on
  `7695d00`), DuckDB goldens 151, public API, dependency, formal checks.
- SF 1 (this branch, one run): Q19 22.5, D19 12.7, D27 16.3, D72 75.9, H08 17.9,
  H09 106.7, H03 28.1, Q6 16.5, H05 35.6, H21 107.3, H07 44.2, D85 62.8,
  H07_block 72.6 ms; all exact. The H08/D19 slowdown seen on the abandoned
  `codex/datalog` branch is absent, so it came from its core changes.

Waiting on user decisions (core changes, not made):

1. Folder cleanup: move `course/hw4_optional.rs` and `teaching/ya/ya_schedule.rs`
   into crate test directories (touches root `Cargo.toml`,
   `passes/cq-reduce`, comments in `cq-lang-ra`).
2. `cq-testkit` golden helpers are gated behind `duckdb`
   (`testkit/cq-testkit/src/lib.rs:122-123`), so the Ascent oracle builds
   bundled DuckDB.
3. `cq-testkit::runtime_rustc` picks backend rlibs by recency and one API probe
   (`lib.rs:154-197,219-222`); it fails when root and a feature with another
   `hashbrown` share a target directory. Separate target directories avoid it.
4. `codex/d85-regression` (`46be507`) changes `passes/cq-order`; D85
   62 → 20 ms.

Tooling notes: `tools/check_public_api.py` reads `ROOT/target/doc` regardless
of `CARGO_TARGET_DIR`; the external gate needs `TMPDIR=/private/tmp` on macOS.

Remaining Datalog work: profile linear TC and same-generation gaps; rule-local
aggregates; Ascent lattices and custom providers as explicit contracts;
FlowLog comparison; partial-Rust multi-round refinement argument.

The abandoned design (`codex/datalog`, `codex/datalog-lang`,
`codex/maintained-structures`, `codex/datalog-*`, `codex/tests-reshape`,
`codex/cleanup-folders`, `codex/testkit-backend-identity`) changed core and is
kept only for reference.
