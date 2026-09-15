# Private all-answers branch

This branch is a self-contained, editor-facing checkout of the completed basic
query-lowering checkpoint. It is private staff material and must not be pushed
to the student repository or distributed before the release policy permits it.

The private reference groups compiler implementations by macro boundary. The
[compiler ownership map](doc/COMPILER-STRUCTURE.md) records family crates and
independently selectable implementation modules. `cq-compose` owns only
actual default route selection and complete-program composition; callers import
explicit implementations and planning types from their owning crates.

The basic route is CQ → RelationalPlan → IndexRequirements → staged Rust.
`cq-plan::canonical`, `cq-index::input` and `cq-physical::input` supply its three
transformations. `cq-pull` and `cq-pipeline` lower its embedded Access and
Pipeline regions to native Rust.

The canonical bare-variable route begins with Rename and does not emit `Unit`.
The implemented Rust-expression route can seed a prefix with the empty-heading
`Unit` relation; the materialized and direct physical routes also support it.
These extensions do not implement Optional HW4. See the
[current IR boundaries](doc/IR-STAGE-REVIEW.md) for each route's domain.

## Inspect every layer in VS Code

The repository-owned stepper is the authoritative way to inspect intermediate
compiler results. From the repository root, either run:

```text
cargo xtask explain triangle
cargo xtask explain triangle --through index-requirements
```

Alternatively, choose **Tasks: Run Task** and **MiniLinq: Explain query** in
VS Code.
The task prompts for a catalog case and one of the six stopping stages. The
command prints clickable absolute paths under
`target/mini-linq-explain/<case>/`:

```text
01-cq.rs
02-relational-plan.rs
03-index-requirements.rs
04-rust-access-plan.rs
05-iterator-pipeline.rs
06-final-rust.rs
README.md
```

`explain` uses the lightweight tool build and does not compile DuckDB. The
oracle is enabled automatically only for `golden` and `differential`.

Every `.rs` file is the complete query program or staged Rust residual at that
boundary. Select adjacent files in the Explorer with **Select for Compare**
and **Compare with Selected**. If a pass fails, the stepper preserves the
successful earlier files from that run, removes stale later files, and writes
`ERROR.md` at the failed edge. The trace files live outside the Cargo crate
graph and are intended for syntax inspection and diffs; Rust Analyzer does not
type-check them as workspace modules.

Rust Analyzer can still show the final generated code in its call-site
context. Open a query such as
`workloads/src/queries/classic/triangle.rs`, put the caret on
`workload_query!`, and choose **rust-analyzer: Expand macro recursively at
caret**. After switching between the student and all-answers branches in an
open window, first run **rust-analyzer: Reload workspace** so cached
proc-macro artifacts are replaced.

The branch-local `.vscode/settings.json` enables
`cq-workloads/compiled-queries`. That feature forwards
`mini-linq/compiled-workloads`, so every direct `workload_query!` invocation
emits its typed-CQ catalog factory and calls `cq_compose::compile_program` to emit its generated `Program` and
`ProgramStorage`. It does
not invoke a nested `mini_linq!` wrapper.

Consequently, Rust Analyzer's recursive command is **not** a layer stepper: it
returns the final ordinary Rust, and the first three typed results never exist
as nested macro invocations in `workload_query!`. Do not try to expand again
from the virtual `[EXPANSION].rs` buffer. Use `cargo xtask explain` for the six
compiler boundaries.

`Program::build` performs one-time input normalization and all physical
construction. `Storage::query` returns one lazy, distinct result cursor;
constructing it reads no stored row, and its first `next()` enters the outer
`once_with`. `Storage::materialize` drains that already-distinct cursor into a
`Vec` and sorts it. No second raw-result cursor is generated.

The supplied `cq_pull::compile` lowering preserves operator boundaries as an
IteratorPipeline: `scan`, left-deep binary `join`, `filter`, `project`, and
explicit `distinct`, with one named `iterN` result and a complete intermediate
binding tuple at every edge. `cq_pipeline::iterator::compile` then chooses the
actual Rust `map`, `flat_map`, and `filter` adapters and emits one Rust local
per `iterN`. Its returned `syn::Expr` is the final concrete IR; proc-macro
`ToTokens` is pure emission.

Generated query access borrows physical tuples: row scans use `.iter()` and
proper-index payload traversal uses `.flatten()` without `.cloned()`. Bound
positive variables are references, so lookup keys, membership tests, and
projection clone only the fields needed to construct owned tuples. Lazy
distinctness retains one clone of each first-seen owned result. An additional
build-time representation likewise borrows the normalized relation and clones
only the key and complement fields that new representation must own before the
normalized relation's final move.

Rust Analyzer does not provide MiniLinq-specific completion inside a custom
token tree; the typed syntax, contracts, stepper artifacts, and focused tests
own those diagnostics.

## Verify the answer branch

Check all authored queries through the same proc-macro route used by the
editor:

```text
cargo check -p cq-workloads --features compiled-queries --locked
```

Run the complete baseline suite and its focused pass edges:

```text
cargo test --workspace --locked
cargo test -p cq-compose --test edge_contracts cq_to_relational_plan_contract
cargo test -p cq-compose --test edge_contracts relational_plan_to_index_requirements_contract
cargo test -p cq-compose --test edge_contracts index_requirements_to_staged_rust_contract
cargo test -p cq-compose --test documented_triangle
```

Run all generated programs against the independent DuckDB oracle:

```text
cargo xtask differential all tiny --scenario all
```

Optional Homework 4 is intentionally not implied by the completed basic
answers. Its `hw4` feature gates only its optional test targets. On an HW4
answer checkpoint, students cumulatively extend the production CQ,
RelationalPlan, index, and physical passes while continuing to use
`mini_linq!`; Cargo does not silently replace the baseline macro semantics.
