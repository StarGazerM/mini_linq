# Next work: cleanup

From the per-module and architecture review at `ddb0bf9`. Start after the
in-flight work lands; re-verify each item against the tree first (line numbers
are as of `ddb0bf9`). Estimates are reviewer estimates.

## C1 Pure deletions (≈1k lines, no design change)

- `runtime/cq-state-providers` (no references; repoint the mutation fixture in
  `tools/check_dependencies.py:188`).
- `cq-syntax`: `predicate.rs` (+ re-export `cq-lang-cq/src/lib.rs:45`, test
  `tests/rust_expressions.rs:94-106`), `rust::primitive_i32`, `rust::literal`.
- `cq-lang-cq`: `normal.rs:355-558` `Kernel`/`Renaming` (+ tests/normal.rs),
  hypergraph `conformal`/`chordal`/`width`, `hypergraph::check`.
- `cq-lang-ra`: `Schedule::union`/`union_over`; unreachable check `lib.rs:729-740`.
- `cq-lang-physical`: `Analysis.point_reads`; no-op `map_err` `lib.rs:558-560`.
- `cq-order`: `Ranking::Auto`. `cq-reduce`: `Pushdown`/`Options.guards`.
- `cq-elaborate`: `block_hoisting` field.
- `cq-physical-lowering`: `Reads.merged`/`own_merge`; `pub use check_rewrite`
  (also in `cq-structure-selection`).
- `cq-print`: `materialize` `_canon` param; `relational_condition` wrapper.
- `cq-stage`: `stage()`.
- `cq-storage`: `dispatch.rs:131-279` `Range`/`RangeIndex`/`RangeCursor`;
  `Canonical` + `require::canonical` (+ dead `set` branch in cq-print);
  `require::candidates`/`keyed`; merge `RangeLookup` into `SeekRangeLookup`.
- `cq-load`: `parquet_hash`/`parquet_btree`; tests-only `ZoneRows::from_batches`.
- `mini-linq`: unused dev-deps (`cq-stage`, `cq-state-providers`, `syn_derive`,
  `contracts`, `prettyplease`); `with_<head>` alias; redundant `plan` unit test.
- `tools/check_features.py`, `tools/check_expansions.py`.
- Root `Cargo.toml`: unused deps of `mini-linq-course`; move
  `hw4_optional.rs` out of `cq-lang-cq/tests` to stop the double target.
- Workloads: `rerun_with_oracle`, `compare_push`/`Mode::Push`, unused xtask
  deps, `execution::load`, `assert_non_fan_growth`.

## C2 Language-owned analyses (≈600 lines)

DESIGN: a language contract returns the facts its consumers use.

- `ix::demands(&ra, &analysis)`; cq-index-policy compile/check and
  diverged-ra call it (unforbid cq-index-policy in diverged-ra `check.py`).
- Physical key inheritance (5 copies) and smallest candidate prefixes (5 copies)
  in `cq-lang-physical`.
- `blocks::bound_before` in cq-lang-cq (7 hand-rolled loops); one scope walker
  shared by `cq-syntax/rust.rs` and `cq-lang-cq/sugar.rs`.
- One literal-disjointness fold for RA and Physical `constants.rs`; shared
  heading/type checks in `cq_syntax::algebra`.
- One `rewrites` module in cq-testkit used by `tools/fixtures/pass-macros`,
  `stacked-ir/test-passes`, diverged-ra `theta-passes`.
- One SplitMix64 (testkit, workloads generate/adversarial, xtask fuzz).
- `Context::positional_host` and shared `group` in cq-print.
- Within crates: cq-elaborate `structural.rs` through the `Body` path;
  cq-order `check_steps` reuse `filtered_rank`; cq-reduce `Options::effective`;
  VO block facts cached; `join_branches` single atom map / hypergraph.

## C3 Test duplication (≈2k lines)

- stacked-ir `tests/reference.rs` ↔ `tests/stacked.rs`: one parameterised body.
- `workloads/e2e/tests/extension_laws.rs` + `ui/extensions.py` duplicate
  `extensions/conformance`: keep one.
- own-dialect `tests/positional.rs` ↔ facade `tests/positional.rs`:
  printer-parameterised fixtures in testkit; delete own-dialect allow-lists.
- rustc run harness ×3 → testkit helper; `wcoj.rs` into `physical_query.rs`.
- composition/inspection: drop re-tests of core; 18 duplicate planner fixtures;
  rewrites examples.

## C4 Decisions needed

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
   `regression-targets.tsv` to rows `cargo test --workspace` does not cover;
   cut the e2e 4×4,608 matrix to pairwise + one full kernel.
6. **cq-load split.** Move `ColumnAt` and slice impls to cq-storage.

## Process

- Run `cargo fmt --all --check` before committing; `9a949f8` landed unformatted
  and `ddb0bf9` was ~90% fmt churn.
