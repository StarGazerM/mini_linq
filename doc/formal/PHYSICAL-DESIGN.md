# YA-aware physical administration

The architecture mapping is in [architecture_placement.tex](architecture_placement.tex)
and the arena backend design is in [ya_physical.tex](ya_physical.tex), built through
[physical_design.tex](physical_design.tex) into
[build/physical_design.pdf](build/physical_design.pdf). The main manuscript
remains within its 32-page limit and connects its exact backend to this
supplement in [physical.tex](physical.tex). The supplement uses the same
11-point, one-inch-margin layout and is separate from that page budget.

## Contract and decisions

The existing architecture allows replacement of an expression, an owning Rust
context, or a larger compiler segment. Exact hash and bitmap access can use the
same Access language with distinct native expressions. Build-time representation
selection can be an ordinary runtime Rust branch. Conservative transfer owns a
whole region; delayed expansion owns its group handles and intermediate semantics.
Zone-map I/O savings require control of the input/scan context before payload reads.
The default macros have fixed continuations; alternative routing requires wiring
an entry or composing the exported typed passes.

Named RA and its exact semijoin meaning stay intact. Within one arena backend
family, a proposed private overlay selects providers and access operations.
It is not a universal trait or a mandatory IR for other implementations.
Exact mode preserves every RA version;
Transfer mode is limited to a checked YA region and certifies final answers.
The production compiler has not acquired either overlay entry in this change.

- Exact membership: hash members, hash groups, or an injectively encoded dense bitmap.
- Conservative filtering: whole-tuple Bloom filters and coordinate bounds, with explicit source emptiness.
- Block access: zone metadata can prove disjointness; Bloom endpoint probes cannot prove block absence.
- Storage: immutable arenas and fresh survivor selections. Projection deduplicates into a new arena.
- Resource identity: source version, paired key maps, layout, equality/encoding domain, and provider configuration.
- Assembly: exact group lookup, separately specified expansion, and set collection. Per-join handles do not implement whole-subtree factorization.
- Policy: deterministic hash fallback; optional filters cannot weaken capability checks. Runtime data stays runtime data.

## Rule to algorithm to code

[physical_rules.tsv](physical_rules.tsv) lists every proposed rule separately,
its numbered algorithm, proposed handler name, and any semantic-model function.
A dash means the model does not implement that rule. All 20 new rules are
explicitly **abstract/proposed** in the combined [rules.tsv](rules.tsv).
The paper's compilation table identifies existing Rust extension points:

| Boundary | Current code | Proposed responsibility |
| --- | --- | --- |
| Index demand | [relational_to_indexes.rs](../../crates/cq-compiler/src/relational_to_indexes.rs) `compile`, `columns` | Aggregate roles and retain both key-position maps before provider selection. |
| Data construction | [indexes_to_rust.rs](../../crates/cq-compiler/src/indexes_to_rust.rs) `compile`, `build_index` | Create arenas, selections, versioned provider handles and construction-derived coverage evidence. |
| Actual access | [indexes_to_rust.rs](../../crates/cq-compiler/src/indexes_to_rust.rs) `local_plan` | Emit exact Reduce or certified Transfer, and exact Lookup/Expand/Exists operations. |
| Independent check | [physical.rs](../../crates/cq-compiler/src/pass_contracts/physical.rs) `materialized`, `check_indexes`, `check_materialized_clause` | Validate overlay transitions, source versions, evidence, temporary consumption and emission templates. |
| Pass entry | [passes.rs](../../crates/cq-compiler/src/passes.rs) `compile_materialized` | Keep the current entry as baseline; the arena implementation can use a separately checked private overlay feeding existing local Access/Pipeline quotation. |

The supplement spells out the state fields, provider records, missing Rust
command variants, rule premises and updates, two algorithm blocks, the full
`r0`–`r12` physical trace, preservation arguments, ownership, costing and
production validation obligations. Alias transitions invalidate logical-name
resource caches while preserving already emitted immutable handles.

## Research basis

The bibliography is [ya_references.tex](ya_references.tex). Primary sources:

- [SQL Server bitmap filters, CIDR 2026](https://www.vldb.org/cidrdb/papers/2026/p29-zhao.pdf): optimizer placement and runtime representation choice belong to different stages. Its instance-optimality argument assumes exact filtering.
- [Diamond Hardened Joins, PVLDB 2024](https://www.vldb.org/pvldb/vol17/p3215-birler.pdf): exact Lookup/Expand decomposition separates operator scheduling from table representation. A probabilistic replacement does not inherit every exact rewrite.
- [Predicate Transfer, CIDR 2024](https://www.vldb.org/cidrdb/papers/2024/p22-yang.pdf): propagated approximate filtering and rebuilding filters when separator keys change.
- [Engineering YA in Column Stores, PVLDB 2025](https://www.vldb.org/pvldb/vol18/p2413-vansummeren.pdf): nested lookup/expansion and representation-aware execution. Its bag-projection properties are not imported into MiniLinq's set projection.
- [Robust Predicate Transfer, 2025](https://arxiv.org/abs/2502.15181): distinct scheduling algorithms whose performance guarantees are not implied by Bloom installation alone.
- [DuckDB dynamic join bounds](https://duckdb.org/2024/11/14/optimizers) and [zone maps](https://duckdb.org/docs/current/guides/performance/indexing): separate build-key summaries and storage-block metadata.
- [Bloom, CACM 1970](https://www.cs.princeton.edu/courses/archive/spr05/cos598E/bib/p422-bloom.pdf): approximate membership and space/error tradeoffs.

## Validation and practical limits

```sh
make -C doc/formal
make -C doc/formal model
make -C doc/formal example
```

[check.py](check.py) checks the 159-rule combined inventory, all 20 physical
rule-to-algorithm mappings, available model function names, and the existing
32 production source fingerprints. It detects inventory drift, not semantic
proofs or complete compiler correspondence.

[physical_model.py](physical_model.py) checks 4,096 exhaustive binary-domain
chain instances in both modes: 16,384 exact reductions and 16,384 conservative
transfers. It compares final full joins and projected sets with an independent
whole-row product oracle, exact frozen views with full-join projections, and
candidates with their inclusion invariant after every edge. Targeted cases
cover empty/disconnected inputs, composite correlation, forced false positives,
version misuse, immutable masks, zone rejection and projection deduplication.

[access_representations.rs](example/src/bin/access_representations.rs) separately
checks the existing `pull!` macro over native hash and exact bitmap membership,
runtime selection from observed key range, and a collision-prone prefilter followed
by exact membership. All 264 cases agree with an independent input-membership
reference; 1,672 false-positive probes are removed. The range cutoff is illustrative.
This is a local macro-composition witness, not an implementation of the overlay,
a production Bloom filter, or a performance experiment.

This model is a bounded runtime-semantics prototype. It does not implement the
entire administrative command/evidence checker, Rust provider code, dense
bitmap backend, persistent block storage, or performance measurements. Those
are implementation/validation obligations of the completed design, not claims
about the current production compiler. Existing source fingerprints remain
unchanged; no full-workspace validation is implied by the focused examples.
