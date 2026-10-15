# Changelog

Historical commit entries below are on `codex/all-answers` and passed fmt,
clippy, workspace tests, regressions, dependency isolation, the out-of-tree
gate, the public-API check, the formal check, goldens, fuzz, and TPC-derived
kernels exact against DuckDB at SF 0.1 and SF 1.

## Unreleased

- Default facade pass `mini_linq::passes::plan` (renamed from `plan_written`,
  no alias): static Filtered/Coarse ordering, checked Hypergraph reduction,
  redundant-support absorption (`cq_elaborate::simplify_reductions`), and
  static build orientation recorded in RA before IX. `plan(no_reduce)` emits no
  semijoins. Tree reduction only on acyclic scopes and only along
  filter-supported edges; exact block cuts on any scope.
- Relational disjunctions derive implied per-occurrence domains, keeping the
  exact predicate. Guards over allowlisted total std operations, or marked
  `#[total] if <expr>`, move like relational filters; checkers recompute
  domains and reject invented totality.
- Aggregates: redundant dedup layers removed, `SingleGroup` for constant-group
  folds, fast hasher for grouped containers; `AdaptiveKeySet` no longer
  allocates per duplicate key.
- Fixes: reduced prefix-guard RA correspondence, positional predicate reuse in
  replayed unions, correlated-block provenance after reordering, partial
  absorption of shared supports, planner-checker branch erasure.
- Join filters: Structures `join_filter` entries (exact key set or Bloom, with
  the exact probe recorded as covering check) filter streamed scans.
- Bushy plans: checked independent dimension branches in VO, built once.
- `HashIndex` uses hashbrown's Swiss table; `IndexRead::probe` batches 128
  prefetched probes on scan spines; no per-row `Rc`; `HashIndex::keys()`.
- Plan for staged quotation (plan control without touching upper stages) in
  `doc/QUOTATION.md`.
- Formal incremental/Datalog design in `doc/formal/incremental.tex`, with seven
  abstract rules. Distinguishes snapshot refinement from total fixpoint
  equality, candidate dedup from freshness, and representation laws from
  shared maintenance/version contracts. Signed changes remain out of scope.
- SF 1 vs DuckDB 1 thread: 0.96–3.8× (Q19 0.20×), from 2.5–48×; D19 at parity;
  H07 880 → 44 ms, H09 2,170 → 107 ms, H05 269 → 41 ms. D85 regressed to
  61 ms in the integrated tree (open item). Details in `doc/STATUS.md`.

### Datalog branch (`codex/datalog-v2`, not merged)

- `features/datalog`: Datalog on the unmodified compiler. Checked stratified,
  semi-naive stages resumable from text; core `!R(..)` negation; Datalog-owned
  indexed hosts give 0 core index builds per round. Sequential Ascent
  differential tests and Ascent-generated goldens.
- Workload tests are plain case files with pairwise configuration coverage
  plus a full triangle kernel; 14,357 → 2,982 tests, cold build 874 → 169 s.
- Core changes awaiting approval are listed in `doc/STATUS.md`.

## Execution model (exact semijoin pushdown, hashing)

- `bf5206a` Fast hashing as a recorded type: default Structures types carry an
  ahash `HashState`; printers print exactly the recorded type; `retype` can
  still choose `std`. Compact `HashIndex` layout. SF 1: H09 default
  3,907 → 2,020 ms, D85 default 2,609 → 1,512 ms, D85 hypergraph 131 → 73 ms.
- `8d8f759` Push printer realizes selections, key sets and fused reads.
  Workloads advise `positional` for `Vec`/slice hosts.
- `8bbc0ba` Structures `selection`/`keyset`/`read` entries with lineage checks;
  the selection pass decides, users advise with `select(positional(..))`; pull
  printer fuses filters into scans. Physical unchanged on 180 cases. SF 1 D85
  hypergraph rows copied 863,457 → 53,082.
- `c5bd1dc` `cq-load::ZoneRows`: per-block min/max; block skipping through
  `BlockRead`. Reproduces the measured D85 skips (46% of 2,048-row blocks).
- `5615239` `cq-storage` contracts: `PositionalRows`, `BlockRead`,
  `ReadPredicate`, exact `AdaptiveKeySet` (dense bitmap or hash + sorted keys),
  immutable `Selection`, block/row/copy counters.

## Language and planning

- `3b21e5d` Blocks no longer disable relational hoisting (H07 block: Lineitem
  date filter before its build).
- `241f37c` Disjunctive blocks: `{..} | {..}` (union) and `{..} else {..}`
  (per-row fallback); multi-rule heads are sugar for `|`; `union`/`anti` in
  RA′, IX, Physical.
- `b4740da` Filtered build-side index: `foo("a", b)` builds one index keyed by
  `b` over matching rows, without the constant column.
- `81d1a77` Correspondence checks are structural contracts over fixed
  occurrences (`R@clause`); both homomorphism engines deleted (1,752 lines).

## Documentation

- `7cb9da6`, `c25ffdd` Formal paper rewritten: one section per IR, one per
  pass, every rule tied to code (`doc/formal`).
