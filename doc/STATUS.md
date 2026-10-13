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

## Gap to DuckDB (SF 1, single thread)

`cargo xtask bench --tpch --sf 1 --runs 5 --reduction-comparison` from
`features/workloads`, release, one warm-up, median of five. MiniLinq excludes
load; DuckDB v1.5.5 includes prepare. All configurations exact vs DuckDB at
SF 0.1 and SF 1. Raw data: `features/workloads/target/bench/tpch-1-reduction-comparison.{tsv,json}`.

| Query | default ms | no-reduce ms | DuckDB 1T ms | default / DuckDB |
|---|---:|---:|---:|---:|
| tpch_q19 | 21.7 | 42.1 | 105.1 | 0.21 |
| retail_h08 | 19.0 | 22.1 | 12.4 | 1.54 |
| retail_d27 | 23.8 | 22.5 | 12.6 | 1.88 |
| tpch_q06 | 17.3 | 16.5 | 8.4 | 2.05 |
| retail_d72_inner | 113.8 | 113.3 | 53.6 | 2.12 |
| retail_h03 | 36.0 | 35.1 | 16.5 | 2.19 |
| retail_d85 | 43.0 | 43.2 | 18.8 | 2.28 |
| retail_h07 | 43.6 | 95.7 | 18.7 | 2.33 |
| retail_h09 | 181.4 | 179.1 | 70.4 | 2.58 |
| retail_d19 | 36.8 | 36.9 | 13.8 | 2.67 |
| retail_h21_positive | 136.5 | 178.6 | 47.1 | 2.90 |
| retail_h07_block | 69.9 | 97.3 | 18.5 | 3.78 |
| retail_h05 | 67.8 | 65.7 | 17.9 | 3.79 |

Before this work (1a6460c) the same defaults were 2.5–48× slower than DuckDB
(H07 880 ms, H09 2,170 ms, H05 269 ms).

## Left work

Measured causes of the remaining 1.5–3.8× (operator-level comparison against
DuckDB `EXPLAIN ANALYZE`, xctrace profiles):

1. **Join filters on scans.** DuckDB pushes key filters from small build sides
   into the Lineitem scan (D85, H03, D19, D27). Needs an approximate filter
   covered by the exact join check in the IR.
2. **Bushy plans.** Plans are linear chains; DuckDB joins small dimension
   branches (Supplier⋈Nation, Part/PartSupp) first (D27, D72, H09).
3. **Per-row cost.** Probes are 70–86% of CPU in D27/H08/D19; scans are
   scalar over 48-byte rows (Q6). Batched probes and columnar scans.
4. **Positional default.** Selections need the caller's `positional(..)` advice.
5. **Known limits.** Static heuristics only (no statistics); no keys or
   constraints on data; no parallel execution.
