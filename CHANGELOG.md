# Changelog

Historical commit entries below are on `codex/all-answers` and passed fmt,
clippy, workspace tests, regressions, dependency isolation, the out-of-tree
gate, the public-API check, the formal check, goldens, fuzz, and TPC-derived
kernels exact against DuckDB at SF 0.1 and SF 1.

## Unreleased

- Plan advice (rule R-Advice): `plan(reduce(T with S), skip(T with S))`
  records an author's cardinality knowledge as VO reduce steps on top-level
  atoms sharing a variable: `reduce` adds `T ⋉ S` after the Masks reductions,
  `skip` drops a Masks reduction (always sound). Kernels may declare
  `advise plan(..)`, adding an `advised` configuration. D72 is advised
  `skip(Lineitem with PartSupp), reduce(PartSupp with Lineitem)`
  (`ps_availqty > 0` keeps every row): 191–195 → 141–145 ms at SF 1
  (PartSupp index 800k → 124,846 rows; DuckDB 1T 107.7 ms).

- `cq_planners::order(transfer)`: robust predicate transfer (SIGMOD'25) as a
  diverging planner through the public pipeline (rules Plan-Transfer,
  CK-Transfer). On the written order it adds, to the Masks forward pass, the
  stream's reductions by its filtered children and a backward pass from
  filtered sides, parents first; blocks are never extra supports. Every
  retail kernel gains a `transfer` configuration. Against the default it wins
  on D72 (195 → 176 ms; PartSupp index 800k → 125k rows), D85, H08, D19 and
  loses on H03, H05, H07, H07-block, H21+, D27, H09 (175 → 336 ms); see
  `doc/STATUS.md`. Transfers follow join predicates only; scopes with opaque
  Rust keep Masks, since the structural RA check rejects some sound transfer
  schedules there (fuzz seeds 91, 97). `cq_reduce::semijoin` is public for
  external schedules; e2e and fuzz index planners by name/`Planner::ALL`.

- Default plan is left-deep in written order (Written/Coarse): the first
  clause of each scope streams and no pass commutes join operands. Reductions
  (`Reductions::Masks`, rules H-SpanningCore, R-Masks, CK-Masks) reduce each
  indexed clause by its filtered subtrees of the stream-rooted reduction tree,
  children first, before the stream starts, on acyclic and cyclic scopes; the
  stream itself is never reduced or stored. Removed: static build orientation
  (`orient`, L-Orient, CK-Orient), `plan(stream(..))` advice and its `advised`
  kernel configuration, the unused join-filter advice configurations,
  redundant-support absorption in the default chain, and `doc/QUOTATION.md`.
  TPC-derived kernels are written fact-first (H03, H05, H07, H07-block, Q19
  Lineitem; H21+ the other Lineitem; D85 PartSupp; H09, D72 Orders).
- Join filters follow SQL Server's bitmap pushdown: every eager probe or
  semijoin into a statically filtered hash build pushes an exact key set to the
  scan its left spine reaches, through intermediate probes and semijoins, one
  filter per `i64` key column (composite keys filter by each projected column);
  a scan may carry several. A key bound only by an intermediate build is not a
  scan column and is never filtered. Builds over unions qualify. Each covering
  exact operator still runs. At runtime a build whose distinct keys exceed 35%
  of its source occurrence's rows constructs no filter (the RPT+ creation
  threshold); the abandoned filter passes every key.
- `BlockRead::read` shares `read_owned`'s loop; the adapter chain it replaced
  ran twice as slow when pulled row by row (6M rows: 18.9 → 10.7 ms). Dense
  `AdaptiveKeySet` membership inlines into scan loops.
- SF 1 bench (median of five, this machine) vs DuckDB 1 thread: 0.23–1.91×;
  at or below DuckDB on Q19, D19, D27, D85, H05, H07-block, H07, H21+;
  H09 1.03, H03 1.13, H08 1.38, Q06 1.48, D72 1.91. Against the old
  Filtered/orientation default on the same machine: H05 52.8 → 33.7,
  H07-block 106.9 → 35.5, H07 54.4 → 36.5, H21+ 156.2 → 92.5,
  H09 182.3 → 168.8, D85 145.3 → 29.3; D72 141.0 → 205.7 regressed (see
  `doc/STATUS.md`). All answers exact against DuckDB.

- `cq_storage::Filter`: filtering as a representation operation with the law
  `α(s.filter(c, p)) = { o ∈ α(s) | p(c(o)) }`; every read agrees with it and
  filtering twice is filtering by the conjunction. A semijoin is a filter by
  `Membership`. Realizations make filtered tuples unreadable by construction:
  positional hosts filter to a `Selection` bitmap (its only state; every read
  consults it), selections narrow, and `Selection::build` compacts survivors.
  Exhaustive law tests. The `filter_microbench` example (1.5M rows, 2M probes)
  measured the compact rebuild faster than an all-row index whose lookups skip
  cleared ids at every retained fraction, even with that index prebuilt (1%:
  0.5 ms build + 6.5 ms probes vs 124.5 ms probes; 100%: 83.4 + 116.9 vs
  220.8 ms); no masked index type is provided.

- Exact dense key-set construction stages keys in one contiguous buffer, then
  sets bitmap bits directly; it no longer hashes and allocates a shared cell
  per distinct key before discarding that hash table. Sparse and partial-codec
  inputs retain the exact hash/sorted-key fallback. The temporary buffer holds
  input occurrences (including duplicates), and its capacity is accounted.
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
- SF 1 vs DuckDB 1 thread: 0.96–3.8× (Q19 0.20×), from 2.5–48×; D19 at parity;
  H07 880 → 44 ms, H09 2,170 → 107 ms, H05 269 → 41 ms. D85 regressed to
  61 ms in the integrated tree (open item). Details in `doc/STATUS.md`.

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
