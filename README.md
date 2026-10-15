# MiniLinq

MiniLinq compiles conjunctive queries embedded in Rust into iterator programs at
macro expansion time. Small language crates check complete IR programs; passes
record planning, execution, and representation decisions in those programs.
Generated Rust borrows host data through public runtime traits and returns
distinct answer tuples.

```text
CQ → CQ-NF → VO → RA′ → IX → Physical → Structures → Rust
```

Each structural pass has an independent source/target check. Printers are
validated by compiling and executing their output against independent oracles.
Queries denote `bottom + Set`: a panic aborts evaluation, opaque Rust steps
retain their written ancestry, and relational equality assumes Leibniz identity.

Core body blocks `{ A } | { B }` combine answers; `{ A } else { B }` use B only
where A is empty. Both need two nonempty branches, allow nesting, and export
only names bound in every branch. Multiple rules for one head are union sugar.
Join distributes over union, while fallback is `B1 ∪ (B2 ▷_F B1)`, with an
antijoin on the block's free variables F. Rust-free blocks become derived
relations; other blocks run per row and stream their prefix once. The IR records
branch paths, common headings, negative references, canonicalization, witness
stopping, retention, and the Rust ancestry tree. Rust `||` keeps its guard rules.

## Layout

| Folder | Contents |
|---|---|
| `languages/` | Complete programs, parsers, printers, local contracts, and shared `cq-syntax` |
| `passes/` | Transformations and independent correspondence checks; Rust printers |
| `runtime/` | Borrowed access traits, storage, loaders, and distinct-state implementations |
| `facade/` | `cq-stage` pass protocol and `mini-linq` macro entry points |
| `testkit/` | Public corpus, semantic oracles, mutation and refinement helpers |
| `features/` | Standalone external consumers of the packaged public API |
| `tools/` | Dependency, external-consumer, public-API, and regression gates |
| `doc/` | Short design notes and the formal specification |

Grammars and extension APIs live in crate `//!` documentation. See
[the design](doc/DESIGN.md) and [the formal specification](doc/formal/README.md).
`teaching/ya/` supplies the reduction pass's teaching acceptance fixtures.

## Compose a pipeline

Select macro paths in named phases; omitted phases use the defaults. Pass
arguments belong to the selected pass, and rewrite phases accept ordered lists.
The default planner is left-deep in written order: the first clause of each
scope streams and later clauses are indexed and probed. Before the stream
starts, each indexed clause is reduced by its filtered subtrees of the
stream-rooted reduction tree (checked Masks semijoins, plus exact atom-to-block
cuts), and each filtered build pushes an exact key set into the probe-side
scan. Each block branch is analyzed independently. Select
`plan = mini_linq::passes::plan(no_reduce);` to disable optional reductions while
retaining written order; other phases keep their defaults.

```rust
let rows = [(1_i32,), (1,), (2,)];
let answers: Vec<_> = mini_linq::compile! {
    pipeline {
        plan = mini_linq::passes::plan;
        print = mini_linq::passes::print_pull;
    }
    struct Q;
    relation R(x: i32) = (rows);
    answer(x) :- R(x);
}.map(|(x,)| *x).collect();
assert_eq!(answers, vec![1, 2]);
```

## Features

Every feature is a standalone Cargo workspace and must build and test outside
the repository against Cargo-packaged core crates.

- `composition`: explicit pass composition over heterogeneous borrowed hosts.
- `diverged-ra`: an external theta language enters at IX and uses probe/merge routes.
- `extensions`: surface passes for negation, aggregation, recursion, and correlation.
- `inspection`: public CQ parsing and Rust free-variable inspection.
- `own-dialect`: a push printer that preserves recorded Structures decisions.
- `planners`: nine checked CQ-to-RA′ planning choices.
- `rewrites`: checked Physical rewrites and Structures type replacement.
- `stacked-ir`: an external Trace language inserted and erased through checked passes.
- `workloads`: pipeline matrix, deterministic datasets, SQL goldens, fuzzing, and benchmarks.

## Gates

Run from the repository root with the pinned Rust toolchain. Heavy external
builds disable incremental compilation; workload corpus profiles omit debug
data while retaining the full matrix. The workload alias enables its DuckDB
oracle. The PDF build requires `latexmk` and the TeX packages in
`doc/formal/common.tex`.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
bash tools/test_regressions.sh
python3 tools/check_dependencies.py --isolate --frontends --consumers
python3 tools/check_external.py
python3 tools/check_public_api.py
python3 doc/formal/check.py
make -C doc/formal pdf
cargo xtask golden
cargo xtask fuzz --cases 200
cargo xtask fuzz --cases 2000
cargo xtask bench --tpch --sf 0.1
cargo xtask bench --tpch --sf 1
```

Regression logs live in `target/regression-logs/`; isolation, external, and API
gates keep their diagnostics under `target/`. Reviewed API snapshots change
only through an explicit `tools/check_public_api.py --update` review.
