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
   `regression-targets.tsv` to rows `cargo test --workspace` does not cover;
   cut the e2e 4×4,608 matrix to pairwise + one full kernel.
6. **cq-load split.** Move `ColumnAt` and slice impls to cq-storage.

## Next step

After the runtime work is integrated: staged quotation for plan control, in
[`QUOTATION.md`](QUOTATION.md).

## Isolated incremental/Datalog work

The design is in [`formal/incremental.tex`](formal/incremental.tex). It keeps
the existing bottom-plus-Set refinement contract; total fixpoint equality
does not establish equality of panic domains. Maintained representations
must preserve protected cuts and coordinate publication of all indexes.

| Branch | Implementation commits | Validation / integration gate |
|---|---|---|
| `codex/datalog-lang` | `2292a6d` | Review clear; full workload suite running alone |
| `codex/maintained-structures` | `17cdd52`, `08427da` | Five review findings fixed; root 550 and workloads 14,357 passed |
| `codex/d85-regression` | `46be507` | Root 508 and workloads 14,357 passed; full SF1 comparison exact |

These branches remain isolated; their tests do not validate a merged tree.
Workload builds are serialized with `CARGO_BUILD_JOBS=1` after overlapping
builds exhausted physical memory and accumulated swap. D85 and maintenance
workload builds subsequently terminated successfully; no compiler
nontermination was established.

Remaining incremental work:

1. Complete Datalog's serialized workload gate.
2. Integrate staged calls/maintained state with Datalog-generated rounds;
   the Datalog branch still rebuilds query-local indexes.
3. Specify rule-local aggregate binding and dependency tracking; Datalog
   currently rejects aggregate clauses and nested blocks.
4. Establish the partial-Rust multi-round refinement argument and the
   consumer-owned dedup contract. Structural checks are not those proofs.
5. Check the combined tree and benchmark it before replacing the baseline
   table above. For owned staged inputs, positional recovery is direct;
   generic nonpositional `Rows` hosts may rescan by exact key.
