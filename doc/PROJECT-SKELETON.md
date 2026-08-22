# MiniLinq project skeleton

MiniLinq is a sequence of typed compiler languages. The basic checkpoint keeps logical and physical
decisions inspectable while staff supplies parsing, procedural-macro transport, and runtime scaffolding.

```text
CQ
  -- compile_cq (HW1) --> RelationalPlan
  -- compile_relational_plan (HW2) --> IndexRequirements { plan + keys }
  -- compile_index_requirements (HW3) --> syn::File with pull! { RustAccessPlan }
embedded RustAccessPlan
  -- compile_pull (supplied) --> IteratorPipeline
  -- compile_iterator_pipeline (supplied) --> syn::Expr
syn::Expr -- inserted into the staged syn::File --> final syn::File
  -- ToTokens --> emitted Rust
```

Use the [six complete Triangle residuals](DSL.md#complete-triangle-expansion) as the concrete map and
the [R1 handout](R1-START-HERE.md) for implementation order.

## Workspace map

```text
crates/
  cq-ir/src/
    cq.rs                  CQ syntax and contract
    relational_plan.rs     named relational algebra
    index_requirements.rs  complete plan plus index overlay
    rust_access_plan.rs    occurrence-annotated Rust accesses
    iterator_pipeline.rs   named lazy physical operators
  cq-compiler/
    src/passes.rs          three student pass bodies
    src/staff_r1.rs        answers-branch implementations of HW1--3
    src/pass_contracts.rs  exact source-target predicates
    tests/                 structural, compile, runtime, and error checks
  mini-linq/src/lib.rs     proc-macro parsing and typed routing
workloads/
  src/queries/{classic,retail}/*.rs
                           one authored Rust macro invocation per query
  data/                    generated relation inputs
  sql/                     derived DuckDB SQL
  golden/                  derived expected results

examples/                  small programs named by the boundary they demonstrate
xtask/                     query stepper, data generation, and DuckDB oracle
.vscode/tasks.json         interactive query-stepper task
doc/DSL.md                 authoritative languages and residuals
doc/HW1-START-HERE.md      first student transformation
doc/R1-START-HERE.md       student implementation handout
doc/HW4-OPTIONAL.md        guided cumulative extension
```

## Representation ownership

| Representation | Owns | Does not decide |
|---|---|---|
| CQ | typed relations, source-order atoms, head | joins, indexes, Rust |
| RelationalPlan | atomic relation inputs, `unit`/`rename`/`natural_join`/`project`, `rN` edges, inferred attribute-set headings, and separate output metadata | storage or execution |
| IndexRequirements | unchanged plan plus canonical equality keys | Rust collections and builders |
| staged `syn::File` | source-visible API, storage, builders, and the selected `pull!` residual | occurrence accesses or cursor operators |
| RustAccessPlan | source-ordered Rust scans, lookups, membership tests, and output | cursor operators or combinator spelling |
| IteratorPipeline | named `iterN` streams, physical operators, bindings | concrete Rust adapters |
| `syn::Expr` | ordinary Rust locals, iterator combinators, closures, and root deferral | no semantic choice remains |

This table is the architecture test: preserve an earlier fact instead of rediscovering it later. If a
pass makes two independent kinds of decision, introduce a boundary instead of hiding one in printing.

Logical and physical names deliberately differ. `rN` denotes a set-valued algebra result; `iterN`
denotes a resumable stream, not a materialized relation. The namespaces expose the execution boundary.

## Generated API and storage

Basic HW3 returns one complete Rust file:

```text
<vis> struct <Program>;
<vis> struct <Program>Storage { row stores and indexes }

impl <Program> {
    <vis> fn build(inputs) -> <Program>Storage
    <vis> fn run(inputs) -> Vec<Row>
}
impl <Program>Storage {
    <vis> fn query(&self) -> impl Iterator<Item = Row> + '_
    <vis> fn materialize(&self) -> Vec<Row>
}
```

`<vis>` is the visibility of the source program; generated items are not unconditionally public.

`build` eagerly consumes used inputs once, normalizes set semantics, and builds storage outside the
result iterator. Creating lazy `query` reads zero rows; `next()` drives the whole chain. Its expression
changes from `pull!` to `iterator_pipeline!` to Rust across the supplied passes. `materialize` drains
and sorts distinct rows; `run` is `build(...).materialize()`.

Relation declarations determine always-trailing Rust tuple types: `(T,)`, `(T, U,)`, and so on.

| Requirement | Physical form |
|---|---|
| unbound enumeration | `Vec<Row>`, or an already-required full-key `HashSet<Row>` |
| proper equality lookup | `HashMap<Key, Vec<Complement>>` |
| full-row membership | normalized `HashSet<Row>` moved directly |

Every used relation is normalized once into `HashSet<Row>`. Extra representations borrow it and clone
only fields their owned keys or complements require; one final consumer moves it. Query traversal
borrows stored rows and payloads. It creates owned lookup keys and output rows, while lazy distinctness
retains the first copy of each result. The generated operations impose the corresponding Rust
`Clone`, `Eq`, `Hash`, and output `Ord` obligations. Identical `(relation, key columns)` requirements
share one index, including across self-joins. [DSL.md](DSL.md) gives the exact rules.

## Student and staff boundary

| Component | Basic-checkpoint owner |
|---|---|
| IR syntax, parsers, local contracts, and edge predicates | staff |
| `compile_cq` | student, HW1 |
| `compile_relational_plan` | student, HW2 |
| `compile_index_requirements` | student, HW3 |
| `compile_pull` and `compile_iterator_pipeline` | staff |
| proc-macro routing, fixtures, structural/rustc/runtime harnesses | staff |
| classic and retail-derived workloads, data, SQL, and oracle | staff infrastructure |

Students edit only the three bodies in `crates/cq-compiler/src/passes.rs`. The answers branch delegates
them to staff; the homework branch leaves the same entry points. Fixtures define the contract.

## Optional routing

[Homework 4](HW4-OPTIONAL.md) cumulatively extends the production CQ, algebra, requirements, and
sequential physical pass with negation and aggregation. It continues through `mini_linq!`; feature
`hw4` gates optional tests, not language meaning or backend selection.

[Push/Tokio, Rayon, and multiple-query sharing](OPTIONAL-PROJECT-TRACKS.md)
make different architectural choices. A backend gets a distinct typed target and explicit route when
implemented; Cargo features do not silently reinterpret an existing macro.

## Verification and editor workflow

After the three student passes are complete, run:

```text
cargo fmt --all --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test --manifest-path xtask/Cargo.toml --locked
cargo test -p cq-workloads --features compiled-queries --locked
cargo xtask differential all tiny --scenario all
```

Each workload query is ordinary Rust under `workloads/src/queries`. Inspect one catalog case at every
boundary with the real typed passes:

```text
cargo xtask explain triangle --through relational-plan
cargo xtask explain triangle
```

Snapshots `01-cq.rs` through `06-final-rust.rs` go under `target/mini-linq-explain/<case>/`; failure
writes `ERROR.md` but preserves earlier output. The stepper does not build DuckDB. In VS Code, run
**MiniLinq: Explain query** and compare adjacent files. Snapshots are outside the Cargo graph, so Rust
Analyzer does not type-check them; recursive expansion is only a final-code view. See the
[complete workflow](R1-START-HERE.md#inspect-one-query-one-pass-at-a-time).
