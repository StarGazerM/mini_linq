# MiniLinq

MiniLinq is a Rust query compiler and teaching project built from replaceable
macro boundaries. Logical planning, equality-demand selection, whole-query
realization, native storage operations and Access execution are separate choices.
The completed implementation and its tests live in implementation modules;
[teaching extraction](teaching/ya/README.md) selects a smaller foundation and
student exercises without maintaining a second compiler.

```text
CQ → RA → IndexRequirements → Rust[Access, Native] → Rust[Pipeline, Native] → Rust
     RA ── configured relation realization ──→ Rust[Access, Native]
                                Access ── push to set ──→ Rust
```

The enclosing Rust file is a residual program containing local macro calls.
There is no contextual relational IR, runtime relation interpreter or capability
registry. [The boundary specification](doc/IR-STAGE-REVIEW.md) lists the macro
families and their semantic obligations; [compiler structure](doc/COMPILER-STRUCTURE.md)
explains crate/module ownership.

The opt-in [staged borrowed route](doc/STAGED-BORROWED-EXECUTION.md) uses the same
index, Access and Pipeline boundaries. An explicit empty index policy selects
borrowed scan realization; local Access regions become support streams,
first-witness booleans or a distinct answer iterator. Selected replay storage
retains demanded prefixes of shared results, with independent cursors and no
payload cloning. `next()` or `any()` executes traversal; `collect()` additionally
stores outputs. Prefix storage and nested comparisons remain explicit costs.

## Choose a composition

`mini_linq!` chooses canonical or source-prefix planning. `optimized_query!`
chooses graph or prefix planning. Their generated stage calls resolve through
the caller's `mini_linq` binding, including compiled workload queries.

An explicit eager composition is an ordinary Rust facade:

```rust
mod eager {
    pub use ::mini_linq::{
        optimized_query, graph_query, prefix_query,
        result_indexes as choose_indexes,
        hash_materialized as choose_realization,
        pull_set, collect_pipeline,
    };
}
use eager as mini_linq;

mini_linq::optimized_query! {
    pub struct Query;
    relation R(a:i32,b:i32);
    relation S(b:i32,c:i32);
    answer(a,c) :- R(a,b), S(b,c);
}
```

The explicit hash realizations are `hash_inputs!`, `hash_prefix!` and
`hash_materialized!`. Their storage and materialization commitments are part of
the selected implementation. Changing a later Access macro changes execution
within that already chosen representation.

`relation_program!` and `relation_query!` instead lower configured RA with
per-input/result row providers and per-operation algorithms. One query can
combine block storage, heterogeneous tuple hash storage and tuple sorted results.
Its native pair boundary supports hash, nested-loop and sort-merge joins. Its
candidate boundary supports scans or block metadata rejection followed by exact
membership. Unsupported storage operations fail during macro expansion or Rust
type checking. See [native access](doc/ACCESS-REFINEMENT.md).

`pull!`, `pull_set!` and `pull_stream!` share Access-to-Pipeline lowering with
different consumers. `push_set!` emits nested Rust loops directly into an exact
set. A set-producing implementation can replace a set hole; it does not thereby
satisfy an iterator's demand contract. Host-expression effects make this
restriction observable and are covered by executable counterexamples.

## Inspect and run

```sh
cargo run --example cq-inspection
cargo run --example pull-execution
cargo run --example free-variables
cargo run -p mini-linq --example blocked-ya --locked
cargo run -p mini-linq --example rust-expressions --features compiled-workloads
cargo xtask list
cargo xtask explain triangle
```

`cargo xtask explain` writes complete, formatted intermediate artifacts under
`target/mini-linq-explain/<case>/`. The [boundary artifacts](doc/planned-example/01-cq.rs)
are compiled as tests. The [expression gallery](examples/rust-expressions/README.md)
shows scope, computed bindings, query results and evaluation counts.

The [workload catalog](workloads/README.md) contains authored queries, deterministic
datasets and SQL oracles. Catalog factories work independently of completed
compiler passes; `compiled-queries` enables the same caller-bound macro route.

```sh
cargo test --workspace
cargo check -p cq-workloads --features compiled-queries --locked
python3 tools/check_dependencies.py --isolate
python3 tools/check_features.py
make -C doc/formal
```

Feature and source-removal gates check actual selected implementations, native
operations and end-to-end consumers. The [formal snapshot](doc/formal/snapshot.txt)
records completed validation and source identity. The [manuscript](doc/formal/README.md)
maps named semantic rules to code and separates proved conditional laws from
remaining obligations. Passing source correspondence does not prove arbitrary
user replacements correct.

## Teaching and scope

Start with the [documentation map](doc/README.md), [CQ-to-RA handout](doc/HW1-START-HERE.md)
and [language contract](doc/DSL.md). Course labels identify exercises; compiler
modules are grouped by semantic responsibility. The optional `hw4` exercise
remains unfinished and is excluded from the supported feature matrix.

Current implementations use finite in-memory set semantics. Configured relation
realization has explicit materialization boundaries; it does not automatically
search costs or fuse arbitrary regions. Rust types, ownership, captures and
provider laws remain part of composition correctness. The
[expression contract](doc/RUST-EXPRESSIONS.md) describes those limits.
