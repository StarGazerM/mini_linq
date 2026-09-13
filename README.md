# MiniLinq query-compilation course project

MiniLinq is a Rust/procedural-macro teaching scaffold for Modern Query Processing. The basic
checkpoint has three student passes and two supplied physical lowerings.

Queries, relational outputs, access plans, and iterator returns end with `;`.
The earlier period terminator is rejected.

The supplied [Rust-expression helper](doc/RUST-EXPRESSION-HELPERS.md)
collects lexical free names from native Syn expressions. Try
`cargo run --example free-variables`.

```text
CQ
  -- Basic HW1 --> RelationalPlan
  -- Basic HW2 --> IndexRequirements { complete RelationalPlan + keys }
  -- Basic HW3 --> staged Rust file containing pull! { RustAccessPlan }
  -- supplied pull --> staged Rust file containing iterator_pipeline! { IteratorPipeline }
  -- supplied Rust lowering --> ordinary Rust with one lazy result-set Iterator
```

Start with the [Homework 1 handout](doc/HW1-START-HERE.md) for the first edge.
The cumulative [query-lowering handout](doc/R1-START-HERE.md) and
[language contract](doc/DSL.md) cover all six boundaries. The
[workload guide](workloads/README.md) defines the queries, datasets, SQL, and
DuckDB oracle.

CQ, RelationalPlan, IndexRequirements, RustAccessPlan, and IteratorPipeline are IR syntax objects.
The surrounding `syn::File` is a **staged Rust program** or **program residual**: ordinary Rust with
local IR holes: one in the basic route, several in the materialized route.

The implemented [Stage I planner](doc/PLANNED-PASS-IMPLEMENTATION.md) supplies
alternative typed passes through these same languages. `planned_query!`
selects graph-based CQ-to-RA planning; `planned_relational_plan!` selects
indexes for the written RA; `materialized!` emits one set-valued `collect_access!` region
per named result. It supports bushy joins, alpha-acyclic semijoin reduction,
cyclic binary fallback, bounded comparisons, and intermediate projection.
Its `build` materializes the DAG; `query` iterates the stored answer.
See the [verification matrix](doc/STAGE-I-VERIFICATION.md) and
[complete boundary artifacts](doc/planned-example/01-cq.rs).

Language-entry macros emit relative `mini_linq::next_stage!` calls. An ordinary
module facade aliased as `mini_linq` can re-export a different next-stage macro.
The [block-access implementation](doc/BLOCK-ACCESS-IMPLEMENTATION.md)
uses this wiring to compile exact integer RA with independently selected input
providers, result materializations, semijoin recipes and join recipes. One query
can borrow a block store, a native hash set and a native sorted set; zone pruning
affects only its selected operand. The existing
materialized backend accepts `access macro_path;` as well. Run the concrete
example with `cargo run -p mini-linq --example blocked-ya --locked`.
The [module and crate design](doc/COMPILER-STRUCTURE.md) separates physical
implementations, local compilation decisions, and data structures. `cq-storage`
contains immutable `BlockStore` storage, independent `BlockMask` bitmaps, summaries
and passive counters. Operational traits specify borrowed block access, selection,
exact membership and conservative ranges; implementations can change layout
without changing recipes that use the same semantics. Query access is exposed
through traits; layout helpers remain private. Generated code owns their
association and query execution;
there is no universal runtime relation API. `physical_plan!` composes provider and
algorithm macros through the existing RA, Access and Pipeline languages. The
compiler checks the composition's bindings; macro expansion and Rust check the
selected operations and types. [The design discussion](doc/CONSTRAINED-STAGING-DESIGN.md)
separates this explicit composition from proposed automatic planning.

CQ arguments, `if` conditions, and `let` values preserve Rust expressions.
The [expression contract](doc/RUST-EXPRESSIONS.md) explains scope, computed
result types, and conservative scheduling through the existing IRs.
The [runnable expression gallery](examples/rust-expressions/README.md) includes
input rows, asserted outputs, join schedules, and evaluation-count checks.

## Compiler boundaries

### Logical plan

Basic HW1 lowers CQ to named relational algebra. Declared relation names are atomic inputs;
each occurrence gets one complete attribute Rename, and later definitions name earlier `rN`
results. Headings are inferred attribute sets rather than serialized result schemas:

```text
r0 = rename R {c0 -> x, c1 -> y};
r1 = rename S {c0 -> y, c1 -> z};
r2 = natural_join r0 with r1;
r3 = rename T {c0 -> z, c1 -> x};
r4 = natural_join r2 with r3;
r5 = project r4 keep {x, y, z};
output r5 as triangle(x, y, z);
```

Basic HW2 derives exact first-use equality-index keys by intersecting the left heading with each
right Rename and translating its target attributes back to declared source-column positions.
`IndexRequirements` preserves the **entire RelationalPlan** and appends those keys, so join order
remains inspectable before storage decisions.
The general algebra can be bushy; the basic physical route accepts the canonical left-deep chain
documented in the [R1 handout](doc/R1-START-HERE.md#basic-hw2-relationalplan-to-indexrequirements).

### Physical plan and lazy pull execution

Basic HW3 returns a complete `syn::File` with the `Program` and generated owning-storage API,
concrete Rust containers and types, final access expressions, a lazy distinct query, and sorted
materialization.
Its only residual region is one expression-position `pull!` parsed as a `rust_access_plan::Plan`;
access leaves are already concrete Rust. See the
[full triangle residual](doc/DSL.md#complete-triangle-expansion).

The supplied pull pass emits named lazy streams—`scan`, binary `join`/`filter`, `project`, then
`distinct`—in IteratorPipeline. Thus `rN` names a logical set relation and `iterN` a physical
lazy stream. The final supplied pass structurally maps each `iterN` to one Rust local using iterator
adapters; code generation does not rediscover the plan.

- The generated storage's `query()` constructs the root iterator but requests no row.
- `once_with(...).flatten()` starts traversal on the first `next()`.
- `materialize()` drains distinct owned results and sorts them; `Program::run` is
  `build(...).materialize()`.

Traversal borrows stored tuples: scans yield `&Row`, proper index lookups yield `&Complement`, and
membership constructs an owned lookup key. Generated expressions clone only fields needed for
owned keys and results—not stored rows or payloads wholesale. Each first-seen owned result is also
cloned into the lazy distinctness set.

`Program::build` normalizes each used input once into `relationN: HashSet<Row>`. Extra builders
borrow that set, one final consumer moves it, and a full-key index reuses it rather than collecting
again, preserving input set semantics.
This supports self-joins and multiple indexes while dropping unused inputs without iteration.
Names follow `inputN`, `relationN`, `rowsN`, and `indexN`; absolute standard-library paths prevent
caller capture.

Relation declarations contain concrete Rust column types. Those types determine trailing-tuple rows,
keys, payloads, and results; generated operations impose their natural `Clone`, `Eq`, `Hash`, and
`Ord` requirements. The
[typed relation contract](doc/OPTIONAL-PROJECT-TRACKS.md#baseline-typed-relation-contract) states
the exact ownership and trait policy.

This is a conventional demand-driven
[Volcano](https://people.eecs.berkeley.edu/~prabal/teaching/resources/eecs582/graefe94volcano.pdf)
route: lazy intermediate streams remain explicit without materializing full intermediate
relations. The protocol permits native multiway operators, but R1 deliberately uses a left-deep
binary decomposition. Rust or LLVM may inline the adapters later; that optimization is not part of
pull semantics. Storage construction and sorting are blocking, while aggregation and distinctness
may retain state. Finite in-memory iteration needs no async runtime; `pull` and `push` are sibling
lowering strategies.

## Student ownership and contracts

| Student pass | Exact result |
|---|---|
| `compile_cq(&cq::Module)` | `syn::Result<relational_plan::Module>` with source plus named algebra |
| `compile_relational_plan(&relational_plan::Module)` | `syn::Result<index_requirements::Module>` with the preserved plan plus exact keys |
| `compile_index_requirements(&index_requirements::Module)` | `syn::Result<syn::File>` with the sequential API and exactly one `pull!` expression |

The [student pass bodies](crates/cq-compiler/src/passes.rs) share one file. Staff supplies parsers,
contracts, macro routing, the two physical lowerings, and structural/runtime tests.

| Proc-macro entry | Typed function |
|---|---|
| `mini_linq` | `compile_cq` |
| `relational_plan` | `compile_relational_plan` |
| `index_requirements` | `compile_index_requirements` |
| `pull` | `compile_pull` |
| `iterator_pipeline` | `compile_iterator_pipeline` |

Each entry parses one typed source and directly calls its function. Grammar errors become span-aware
`compile_error!` output. In authored workload leaves specifically, `workload_query!` directly emits
the typed-CQ factory; it does not use a `macro_rules!` transport wrapper, nested `mini_linq!`, or an
`include_str!` copy. Workload feature `compiled-queries` forwards `mini-linq/compiled-workloads`,
which also runs the completed passes and emits `Program`.

Semantic contracts are switchable:

- macro entries do not separately call source `contract::check`;
- `debug_requires` is the typed function's source precondition, enabled by debug assertions;
- optimized builds may skip it, making invalid unchecked input outside the function contract;
- nominal pass targets own debug postconditions; `syn::File`/`syn::Expr` carry Rust grammar.

For each nominal IR, `contract::check` remains the explicit span-aware diagnostic API and
`contract::well_formed` the Boolean predicate. Edge contracts relate exact source/target structure;
tests inspect generated Rust structurally and with rustc. Details are in the
[R1 contract rules](doc/R1-START-HERE.md#what-students-write).

## Optional routes

[Homework 4](doc/HW4-OPTIONAL.md) cumulatively adds negation and aggregation to CQ and relational
algebra; it keeps `mini_linq!`, while `hw4` gates its tests. The
[project tracks](doc/OPTIONAL-PROJECT-TRACKS.md) cover push/Tokio, Rayon, and exact cross-query
index sharing; the same document also records the concrete typed baseline. Backend alternatives get
explicit macros and typed functions. Incompatible choices use separate checkpoint branches unless
a combined route is supplied and tested. Grading policy is deferred.

## Inspect one query one pass at a time

```text
cargo xtask explain triangle
cargo xtask explain triangle --through relational-plan
```

The stages are `cq`, `relational-plan`, `index-requirements`, `rust-access-plan`,
`iterator-pipeline`, and `final-rust` (the default). Complete formatted snapshots go to
`target/mini-linq-explain/<case>/` as `01-cq.rs` through `06-final-rust.rs`, plus `README.md`.
On failure, earlier current-run artifacts remain, stale later files are removed, and `ERROR.md` is
written.

In VS Code, choose **Tasks: Run Task -> MiniLinq: Explain query**, then compare adjacent files. The
[step-by-step workflow](doc/R1-START-HERE.md#inspect-one-query-one-pass-at-a-time) explains the
editor task and debugging. These ignored snapshots are not Cargo modules, so Rust Analyzer does not
type-check them in call-site context. Its recursive expansion shows final Rust, not intermediate
typed passes; virtual `[EXPANSION].rs` is an output view and cannot be stepped or re-expanded.

## Commands and workloads

Starter-safe inspection and artifact generation:

```text
cargo run --example cq-inspection
cargo run --example pull-execution
cargo xtask list
cargo xtask explain triangle --through cq
cargo xtask generate all tiny --scenario all
```

After completing the three student passes:

```text
cargo run --example triangle-duckdb --features duckdb-example
cargo test -p cq-compiler --test pass_contracts cq_to_relational_plan_contract
cargo test -p cq-compiler --test pass_contracts relational_plan_to_index_requirements_contract
cargo test -p cq-compiler --test pass_contracts index_requirements_to_staged_rust_contract
cargo test -p cq-compiler --test documented_triangle
cargo test -p cq-compiler --test pull_lowering
cargo test -p mini-linq --test endpoint
cargo test --workspace
cargo check -p cq-workloads --features compiled-queries --locked
cargo xtask differential all tiny --scenario all
```

The [project examples](examples/README.md) are named by the boundary they
demonstrate. `triangle-duckdb` keeps a small set of input rows, the MiniLinq CQ,
the equivalent SQL, both executions, and their equality assertion in one
readable program. The workload corpus remains the broader correctness path.

The 20 authored Rust leaves—ten classic join shapes and ten retail-derived positive CQ
kernels—call `workload_query!` directly. Their catalog factories work before the passes are
complete; the `compiled-queries` feature exercises the completed path.

`list`, `generate`, and `explain` do not build DuckDB. `golden` and `differential` transparently
enable the `oracle` feature; the first run may compile bundled DuckDB. Golden checking is read-only;
only staff should rewrite answers:

```text
cargo xtask golden all tiny --scenario all
cargo xtask golden all tiny --scenario all --write
```

Deterministic data, visible SQL, scenarios, set semantics, scale policy, artifact paths, and the
TPC-derived suite's non-compliance boundary are documented in the
[workload guide](workloads/README.md). `generate` consumes authored typed-CQ factories and never
rewrites query modules.

The checked-in toolchain pins Rust 1.97.1 with `rust-src`, `rustfmt`, and `clippy`; workspace
`rust-version = "1.85"` is only the minimum compiler. VS Code should use the Rust Analyzer bundled
with its extension rather than overriding `rust-analyzer.server.path` to a rustup component.
AI-assisted submissions must complete [AI-USE.md](AI-USE.md).
