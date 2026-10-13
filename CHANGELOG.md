# Changelog

Historical commit entries below are on `codex/all-answers` and passed fmt,
clippy, workspace tests, regressions, dependency isolation, the out-of-tree
gate, the public-API check, the formal check, goldens, fuzz, and TPC-derived
kernels exact against DuckDB at SF 0.1 and SF 1.

## Unreleased

- The default facade now performs checked Hypergraph semijoin reduction between
  Written/Coarse ordering and elaboration: a tree reducer only on acyclic scopes,
  but exact atom-to-block cuts on any scope. Each block branch is analyzed
  independently. Opaque Rust guards remain in place.
- Select `plan = mini_linq::passes::plan_written(no_reduce);` in `compile!`'s
  existing pipeline configuration to recover the previous unreduced plan.
  Reduction correspondence is checked before elaboration in both modes.
- Split-witness RA correspondence recognizes local semijoin/projection
  identities while retaining predicate ownership and hidden-column correlation
  checks, so reduced plans can retain their written guard prefixes.
- Both the facade and external Hypergraph planner apply
  `cq_elaborate::simplify_reductions`, a checked RA rewrite which absorbs a
  redundant reduced support only when the probe's other input already supplies
  its required key. This preserves native host reads and the unchanged
  ordered-only-key fixture without introducing a hash requirement. Negative
  operands and unrelated supports remain intact.
- Replayed positional union inputs construct an owned predicate per traversal,
  rather than moving one shared predicate into several index builds. This fixes
  the H07-block compilation exposed by absorption without adding `Copy` bounds.
- Reduction scope is explicit and tested: cyclic bodies keep exact block cuts
  but no tree reductions; cyclic block-free bodies have no reductions. Acyclic
  branches under cyclic roots may receive their own tree reducers.
- Current on/off benchmark results and environment limitations are recorded in
  `doc/STATUS.md`; the historical gate claim above does not apply to this entry.

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
