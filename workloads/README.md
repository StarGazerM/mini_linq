# Workloads and the DuckDB oracle

This directory contains a correctness corpus for the MiniLinq compiler. It has
20 complete, positive conjunctive queries: ten small canonical join shapes and
ten independently named retail kernels derived from selected TPC-H and TPC-DS
join topologies. The corpus is deliberately varied; it does not obtain its
count by making progressively larger cliques.

Each leaf under `src/queries/{classic,retail}/*.rs` is authored Rust source and
the only source of truth for that query's syntax and semantics. Every leaf
directly invokes `::mini_linq::workload_query!` exactly once. That procedural
macro emits the typed CQ factory used by the catalog; it does not choose a
physical policy or generate an executable query API. The catalog retains stable
ordering and teaching metadata; it does not carry a second copy of the query.
Generated data, oracle SQL, and golden results are derived artifacts.
Generation reads the typed CQ factory but never rewrites a query module.

`workload_query!` is not another query language or IR parser. Its input is the
same `cq::Program` grammar accepted by `mini_linq!`; its workload-only role is to
emit catalog metadata. Logical compilation is inspected separately through
`cargo xtask explain`.

The workload tools generate deterministic headerless `i32` CSV inputs,
mechanically translate each authored CQ to visible SQL, evaluate that SQL in
an independent in-memory DuckDB engine, and compare the canonical result with
tracked golden CSVs. Generated-query differential execution is unavailable
until a physical policy for the workload hosts and a host input/output contract
are supplied. The corpus oracle is confined to the separate root `xtask`
workspace. Ordinary workspace builds do not compile DuckDB. The corpus
implementation uses DuckDB's
[Rust client][duckdb-rust] and [Appender API][duckdb-appender] for bulk input.

## The ten classic queries

Open a name below to read the whole authored Rust leaf: its `struct`, every
typed `relation Name(c0: Type, c1: ...);` declaration, and its rule inside one direct
`::mini_linq::workload_query!` invocation.
The corresponding Rust
[`classic` catalog](src/catalog/classic.rs) registers those files alongside
stable order and teaching metadata.

| Command-line name | Distinct purpose |
|---|---|
| [`intersection`](src/queries/classic/intersection.rs) | Unary equality join ending in full-key membership. |
| [`cartesian-product`](src/queries/classic/cartesian_product.rs) | A disconnected body; the second occurrence has a disjoint heading and forms a Cartesian product. |
| [`oriented-chain`](src/queries/classic/oriented_chain.rs) | One self-joined relation has demands on leading and non-leading columns; the head also projects away join variables. |
| [`claw`](src/queries/classic/claw.rs) | The variable-graph star `K_1,3`, with three branches sharing one center. |
| [`fact-star`](src/queries/classic/fact_star.rs) | A relation-join-graph fact/dimension star, distinct from the variable-graph claw. |
| [`triangle`](src/queries/classic/triangle.rs) | The smallest cyclic CQ: root enumeration, partial lookup, then full-key cycle closure. |
| [`cycle-4`](src/queries/classic/cycle_4.rs) | A chordless four-cycle rather than a larger clique. |
| [`spade`](src/queries/classic/spade.rs) | A triangle plus a one-edge handle, testing a branch resumed from an earlier binding. |
| [`barbell`](src/queries/classic/barbell.rs) | Two cyclic regions connected by one edge and a deeper continuation. |
| [`loomis-whitney-4`](src/queries/classic/loomis_whitney_4.rs) | The four-dimensional Loomis-Whitney query: every ternary atom omits a different variable. |

`spade` is a course-local alias, not a claim of standard terminology. The same
triangle-with-one-edge-handle query is called **Lollipop** in the
[EmptyHeaded paper][emptyheaded]; graph theory also calls the underlying
uncolored graph a [paw][paw]. EmptyHeaded is also the source for the Barbell
shape.
The Loomis-Whitney family is discussed as a consequence of the fractional
cover bound in the [NPRR worst-case-optimal join paper][nprr].

Every graph-shaped entry is still a colored CQ/homomorphism query. It does not
silently add vertex inequalities or non-edge predicates, so it is not induced
subgraph matching.

## The ten retail-derived kernels

The optional `retail` suite transfers the same compiler machinery to longer
fact/dimension, role-playing, composite-key, and cyclic shapes. Each linked
name is the complete authored program. The Rust
[`retail` catalog](src/catalog/retail.rs) supplies order, purpose, and scope
notes.

| Command-line name | Positive join core | Inspiration and intentional omissions |
|---|---|---|
| [`retail_h03`](src/queries/retail/retail_h03.rs) | Customer -> Orders -> Lineitem, projecting order and customer keys | TPC-H Q3 topology; omits filters, revenue aggregation, ordering, and limit. |
| [`retail_h05`](src/queries/retail/retail_h05.rs) | Customer -> Orders -> Lineitem -> Supplier, closing on Nation and ending at Region | TPC-H Q5 topology; omits filters, arithmetic, aggregation, and ordering. |
| [`retail_h07`](src/queries/retail/retail_h07.rs) | Supplier -> Lineitem -> Orders -> Customer, with two Nation roles | TPC-H Q7 topology; omits nation-pair and date predicates, arithmetic, aggregation, and ordering. |
| [`retail_h08`](src/queries/retail/retail_h08.rs) | Lineitem rooted snowflake with Part, Supplier-Nation, and Orders-Customer-Nation-Region branches | TPC-H Q8 topology; omits filters, arithmetic, conditional aggregation, and ordering. |
| [`retail_h09`](src/queries/retail/retail_h09.rs) | Lineitem joined to composite-key PartSupp plus Part, Supplier, Orders, and Nation | TPC-H Q9 topology; omits filters, profit arithmetic and aggregation, year extraction, and ordering. |
| [`retail_h21_positive`](src/queries/retail/retail_h21_positive.rs) | Supplier -> waiting Lineitem -> Orders -> Nation plus another same-order Lineitem | Only the positive witness joins from TPC-H Q21; deliberately omits inequality, `NOT EXISTS`, filters, aggregation, ordering, and limit. |
| [`retail_d19`](src/queries/retail/retail_d19.rs) | StoreSales with Date, Item, Store, and Customer -> Address branches | TPC-DS Q19 topology; omits filters, inequality, attributes, aggregation, ordering, and limit. |
| [`retail_d27`](src/queries/retail/retail_d27.rs) | StoreSales joined to four dimensions | TPC-DS Q27 topology; omits filters, measure aggregation, grouped/union construction, ordering, and limit. |
| [`retail_d72_inner`](src/queries/retail/retail_d72_inner.rs) | CatalogSales -> Inventory correlated through Item and Date week, with three Date roles and other dimensions | Only the inner-join spine of TPC-DS Q72; omits outer joins, filters, `CASE`, aggregation, grouping, ordering, and limit. |
| [`retail_d85`](src/queries/retail/retail_d85.rs) | Composite order/item WebSales-WebReturns join with two CustomerDemo roles and four dimensions | TPC-DS Q85 topology; omits filters, arithmetic, aggregation, grouping, ordering, and limit. |

These are independent integer CQs over synthetic data. They are **not** TPC-H
or TPC-DS queries, do not use the TPC schemas or generators, and must not be
reported as compliant benchmark results. Their names retain a short provenance
hint only so students can inspect the full query from which each join topology
was abstracted. This conservative naming and reporting boundary follows the
TPC's rules for benchmark standards and public result use; see the current
[TPC Policies][tpc-policies].

## Scenarios, scale, and result semantics

Generation has two orthogonal controls. A **scenario** chooses the semantic
shape that a dataset must exercise; a **scale** chooses its volume. Generation
is deterministic for the catalog case, scenario, scale, and recorded seed.

| Scenario | Input/result shape |
|---|---|
| `coverage` | Every input is nonempty and the result is nonempty. Planted rows exercise successful proofs and access-specific edge cases. |
| `empty-input` | Exactly one required input relation is empty, so the result is empty. Other inputs remain populated. |
| `no-match` | Every input is nonempty, but the result is empty because join values do not match. |

These are separate datasets rather than a positive dataset with a few
unreachable negative rows. Across the 20 catalog cases, all three scenarios
apply except `cartesian-product`/`no-match`: a Cartesian product of two
nonempty inputs cannot be empty. The checked-in tiny corpus therefore contains
exactly 59 applicable datasets and 59 golden files: 20 `coverage`, 20
`empty-input`, and 19 `no-match`.
With `--scenario all`, the task reports and skips that one unavailable pair.
Requesting `cartesian-product --scenario no-match` explicitly is an error, so
a command cannot succeed after checking zero datasets.

The `coverage` generator plants the following evidence before filling each
relation to its scale target with domain-separated noise:

- Every declared input has a raw duplicate of a row that participates in a
  successful proof. This exercises relevant duplicate input, rather than only
  duplicating unreachable noise.
- When a body variable is projected away, two successful derivations project
  to the same head row.
- Every partial lookup, including lookups deep in the body, reaches a
  controlled multi-candidate bucket and a key with at least two surviving
  complement rows. Every feasible partial lookup also reaches a missing
  bucket; a few self-join prefixes structurally guarantee a hit and are not
  advertised as misses.
- Every full-key occurrence has an isolated leave-one-out assignment: all
  other occurrence rows are present and the target membership row is absent.
- For every column of every composite lookup key, a planted decoy differs only
  at that component.

Not all of those facts are claims about distinct final rows. Active raw
duplicates, missing partial buckets, bucket fanout, and surviving complements
are structural execution guarantees checked directly against the generated
inputs and successful assignments. Projection collisions check result-set
deduplication. Full-key omissions and component-wise composite-key decoys are
stronger: deliberately dropping the corresponding membership or equality
test changes the final result set. Noise rows make inputs substantial without
creating an unrelated cyclic result.

| Scale | Connected-case rows per relation | Complete witnesses, up to | Repository policy |
|---|---:|---:|---|
| `tiny` | 96 distinct rows | 12 | Inputs and golden results are checked in. |
| `medium` | 25,000 distinct rows | 512 | Generated locally and ignored by Git. |
| `large` | 1,000,000 distinct rows | 4,096 | Explicit stress scale; generated locally and ignored by Git. |

Disconnected CQs use a lower per-relation target so their necessary Cartesian
product remains bounded. At tiny scale, `cartesian-product` is capped at 64
distinct rows per relation, hence at most 4,096 result pairs. The manifest
beside each dataset records the actual raw and distinct cardinalities,
scenario, scale, seed, and generator version; those values are checked against
the selected catalog case. The tiny artifacts are also regenerated in memory
and compared exactly before an oracle run, so a valid-looking but stale CSV or
seed cannot silently become the checked-in test. Medium and large primarily
stress import, normalization, and index construction; their deliberately
bounded outputs are not a selectivity study.

The raw relation CSV files retain duplicates. The SQL oracle gives each raw
DuckDB table its own `SELECT DISTINCT` CTE, uses one alias per body occurrence,
equality predicates for shared variables, a final `SELECT DISTINCT`, and an
explicit `ORDER BY` in head-column order. Golden CSVs are therefore strictly
lexicographically ordered, distinct materialized result sets.

Future workload execution must preserve the CQ's set semantics through its
selected physical operators and structures. The retired backend's
input-normalization and sorted-vector API are not imposed on the new chain. No generated-query comparison currently runs for this corpus.

Files are organized as follows:

```text
workloads/src/queries/mod.rs
workloads/src/queries/classic/<case_module>.rs
workloads/src/queries/retail/<case_module>.rs
workloads/src/catalog/<suite>.rs
workloads/data/<suite>/<case>/<scenario>/<scale>/manifest.txt
workloads/data/<suite>/<case>/<scenario>/<scale>/inputN.csv
workloads/sql/<suite>/<case>.sql
workloads/golden/<suite>/<case>/<scenario>/<scale>.csv
```

The query leaf and its direct `::mini_linq::workload_query!` invocation are
authored query input. `src/queries/mod.rs` only registers the leaf modules, and
the catalog is authored metadata that points to the factory emitted by each
procedural-macro invocation. The data, SQL, and golden paths are derived.
`inputN.csv` follows declaration position. A declaration supplies the real
Rust column types and schema; the corresponding CSV/host argument supplies
the relation instance. The manifest retains the source relation name and
arity; positional filenames avoid collisions between legal identifiers such
as `R` and `r` on case-insensitive filesystems.

## Commands

List the stable names and teaching purpose of all 20 cases:

```text
cargo xtask list
```

Generate data and visible oracle SQL. A selection is `all`, `classic`,
`retail`, or one exact case name; the scale is `tiny`, `medium`, or `large`.
`--scenario` accepts `coverage`, `empty-input`, `no-match`, or `all` and
defaults to `coverage`. The positional arguments default to `all tiny`:

```text
cargo xtask generate all tiny --scenario all
cargo xtask generate triangle medium --scenario coverage
```

Generation constructs each selected typed CQ from its authored Rust query leaf
and reads its catalog metadata. It replaces the case/scenario/scale data
directory, writes its manifest and positional input CSVs, and rewrites the
visible derived SQL. SQL is scenario-independent because all scenarios
instantiate the same query. Generation never writes or reformats a query
module: Rust query source is authored input, not a generated artifact.

Check tracked golden rows against a fresh in-memory DuckDB evaluation:

```text
cargo xtask golden all tiny --scenario all
```

`golden` is read-only. Regenerating expected answers is an explicit staff
operation:

```text
cargo xtask golden all tiny --scenario all --write
```

`workload_query!` emits catalog metadata and typed CQ factories. Inspect any
authored catalog case at the currently supported logical boundaries with:

```text
cargo xtask explain triangle --through index-requirements
cargo xtask explain retail_h21_positive
```

`--through` accepts `cq`, `relational-plan`, or `index-requirements`; its default
is `index-requirements`. The command calls the typed CQ → RA → IX passes and
checks their local contracts. IX records named demands over exact RA results:
each demand is the key set a plan applies a result at.
Physical derives demand uses from its operands.

Complete typed payloads are written under `target/mini-linq-explain/<case>/`:

| File | Boundary |
|---|---|
| `01-cq.tokens` | Authored complete CQ program. |
| `02-relational-plan.tokens` | Planned complete named RA program. |
| `03-index-requirements.tokens` | Complete IX program: RA plus named demands. |

These files are typed pass outputs, not captured individual macro expansions or
Rust Analyzer type-checked workspace modules. A new run clears stale later
artifacts. If a pass fails, successful earlier files remain and `ERROR.md`
identifies the failing boundary. Post-IX stage requests reject with a diagnostic
explaining the missing physical choices and host contract.

`cargo xtask differential` is currently unavailable. It immediately reports that
the old whole-query backend was removed and that a physical policy for the
workload hosts and the new host input/output contract are required. It does not
compile DuckDB or execute a substitute backend.

`generate` and the DuckDB-only `golden` check use the query catalog without
invoking compilation. `explain`, `list`, and `generate` use the lightweight
default tool build and do not compile DuckDB. `golden` prints that it is enabling
the oracle and re-runs Cargo with the optional `oracle` feature; users do not
add a feature flag, though the first oracle command may take longer while
bundled DuckDB builds.

[duckdb-rust]: https://duckdb.org/docs/current/clients/rust
[duckdb-appender]: https://duckdb.org/docs/current/data/appender
[emptyheaded]: https://pmc.ncbi.nlm.nih.gov/articles/PMC5221635/
[paw]: https://arxiv.org/abs/1309.1312
[nprr]: https://arxiv.org/abs/1203.1952
[tpc-policies]: https://www.tpc.org/TPC_Documents_Current_Versions/pdf/TPC-Policies_v6.19.pdf
