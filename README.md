# MiniLinq

MiniLinq compiles conjunctive queries embedded in Rust into iterator programs at
macro expansion time. Small language crates check complete IR programs; passes
record planning, execution, and representation decisions in those programs.
Generated Rust borrows host data through public runtime traits and returns
distinct answer tuples.

```text
CQ → CQ-NF → VO → RA′ → IX → Physical → Structures → Sequence → Rust
```

Each structural pass has an independent source/target check. Printers are
validated by compiling and executing their output against independent oracles.
Queries denote `bottom + Set`. Embedded Rust is opaque, pure and total by
premise; a panic violates that premise. Placement follows free variables, not
written-order Rust ancestry. Relational equality assumes Leibniz identity.

Core body blocks `{ A } | { B }` combine answers; `{ A } else { B }` use B only
where A is empty. Both need two nonempty branches, allow nesting, and export
only names bound in every branch. Multiple rules for one head are union sugar.
Join distributes over union, while fallback is `B1 ∪ (B2 ▷_F B1)`, with an
antijoin on the block's free variables F. Rust-free blocks become derived
relations; other blocks run per row and stream their prefix once. The IR records
branch paths, common headings, negative references, canonicalization, witness
stopping, retention, and Rust environments. Rust `||` keeps its guard rules.

## Layout

| Folder | Contents |
|---|---|
| `languages/` | Complete programs, parsers, printers, local contracts, and shared `cq-syntax` |
| `passes/` | Transformations and independent correspondence checks; Rust printers |
| `runtime/` | Borrowed access traits, storage, loaders, and distinct-state implementations |
| `facade/` | `cq-stage` pass protocol and `mini-linq` macro entry points |
| `testkit/` | Public corpus, semantic oracles, mutation and refinement helpers |
| `tests/` | Optional homework acceptance tests for the root course package (`hw4` feature) |
| `features/` | Standalone external consumers of the packaged public API |
| `tools/` | Dependency, external-consumer, public-API, and regression gates |
| `experiments/mac/` | Git-managed MCCL/Metal runtime, GPU-built RadixRun provider, and matched CPU/GPU experiments |
| `doc/` | Short design notes and the formal specification |

Grammars and extension APIs live in crate `//!` documentation. See
[the design](doc/DESIGN.md) and [the formal specification](doc/formal/README.md).
`passes/cq-reduce/tests/ya_schedule.rs` supplies the reduction pass's teaching acceptance tests.

The primary checkout contains all active Mac work. Its standalone workspaces are
`experiments/mac/mccl`, `experiments/mac/cq-mac`, `experiments/mac/gpu-index`, and
`experiments/mac/query-expansion`; none requires a sibling development checkout.
See [Mac implementation and build commands](doc/MAC-IMPLEMENTATION.md).
Metal remains outside the portable core workspace.

## Compose a pipeline

Select macro paths in named phases; omitted phases use the defaults. Pass
arguments belong to the selected pass, and rewrite phases accept ordered lists.
The default planner uses left-deep connected growth (Connected/Coarse), not
written order: the first scheduled clause of each scope streams and later
clauses are indexed and probed. Before the stream starts, each indexed clause
is reduced by its filtered subtrees of the
stream-rooted reduction tree (checked Masks semijoins, plus exact atom-to-block
cuts), and each filtered build pushes an exact key set into the probe-side
scan. Each block branch is analyzed independently. Select
`plan = mini_linq::passes::plan(no_reduce);` to disable optional reductions while
retaining connected scheduling; other phases keep their defaults.

The shipped CPU target lowers with `sequence`, grades with `sequential`, and
emits with `translate_pull`, including profile counters when enabled.
An explicit Structures terminal such as `own_push::push` remains selectable:
set `print` without authoring `sequence`, `grade`, or `decompose`.

Host native reads require declared capabilities and a recorded per-reader
Structures choice; undeclared hosts use the recorded fallback. Printing uses
direct runtime calls and `Build::build`, not autoref tiers or a `build!` macro.
Closed captures use plain owned values cloned at each consuming move closure,
not compiler-inserted `Arc` handles. See [storage contracts](doc/STORAGE.md#133-host-reads-and-dispatch-no-fallback-tiers)
and [current progress and dated evidence](doc/PLANNING-REDESIGN.md#progress).

```rust
let rows = [(1_i32,), (1,), (2,)];
let answers: Vec<_> = mini_linq::compile! {
    pipeline {
        plan = mini_linq::passes::plan;
        print = mini_linq::passes::translate_pull;
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
- `datalog`: checked recursive Datalog compilation and execution.
- `datalog-parallel`: parallel Datalog execution with checked round planning.
- `diverged-ra`: an external theta language enters at IX and uses probe/merge routes.
- `extensions`: surface passes for negation, aggregation, recursion, and correlation.
- `inspection`: public CQ parsing and Rust free-variable inspection.
- `join-filters`: checked join-filter planning and runtime providers.
- `own-dialect`: a push printer that preserves recorded Structures decisions.
- `planners`: nine checked CQ-to-RA′ planning choices.
- `rewrites`: macro wrappers for core Physical rewrites and Structures type replacement.
- `sequence-rewrites`: macro wrappers for core checked Sequence rewrite passes.
- `sequence-tasks`: a parallel task-based Sequence terminal and executable CPU conformance.
- `stacked-ir`: an external Trace language inserted and erased through checked passes.
- `workloads`: pipeline matrix, deterministic datasets, SQL goldens, fuzzing, and benchmarks.

## Gates

Run from the repository root with the pinned Rust toolchain. Heavy external
builds disable incremental compilation. Workloads use visible typed cases and
pairwise pipeline coverage per classic/retail family, plus a joining triangle
kernel's full 2,400 supported configurations. Push excludes merge/smallest;
native leading merge reads require ordered hosts. Named kernel targets split
compilation by planner; their profiles omit debug data. UI fixtures cover
unsupported choices and malformed retained layouts. Workload oracle commands
run the prebuilt DuckDB CLI (`duckdb` on PATH, or the executable selected by
`DUCKDB_CLI`); they never compile or link DuckDB C++. Install the CLI before
running golden, fuzz, or TPC-H commands. The `cq-testkit/duckdb` and xtask
`oracle` features gate Rust oracle code only. Sessions exchange exact CSV
values and Parquet inputs through temporary files. TPC-H benchmark DuckDB
timings include CLI CSV serialization, transfer and parsing, unlike the
historical embedded-library Arrow measurements; process startup and loading
remain outside query timing. The PDF
build requires `latexmk` and the TeX packages in `doc/formal/common.tex`.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
# Final behavioral gate; omit --changed-since for all 118 deterministic targets.
bash tools/test_regressions.sh --changed-since BASE_REV
# Random/generated checks are opt-in, never part of default or quick tiers.
bash tools/test_regressions.sh --tier fuzz
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

Use `--tier quick` or a focused package/target selector while iterating, then the
impact-selected regression suite once for the final behavioral gate. Do not also
sweep every feature workspace with `cargo test --workspace`: those sweeps duplicate
the inventory. Root `cargo test` remains a separate gate; a feature-workspace sweep
is appropriate only for changed-workspace coverage absent from the inventory.
See [the gate protocol and measurements](tools/README.md) for timing, selection,
opt-in fuzzing and explicit passing-evidence reuse.

Regression logs and immutable evidence live in `target/regression-logs/RUN-ID/`;
isolation, external, and API gates keep their diagnostics under `target/`.
Reviewed API snapshots change only through an explicit
`tools/check_public_api.py --update` review.

`tools/test_regressions.sh --jobs N` bounds concurrent independent targets
(default 2; `--jobs 1` preserves sequential execution). Cargo metadata groups
targets by both workspace and effective target directory, including
`CARGO_TARGET_DIR`; targets sharing either resource never overlap. Scenario
Cargo contexts are recorded in `tools/test_regressions.py`; unknown scenarios
or unavailable metadata conservatively run without overlapping Cargo targets.
`CARGO_BUILD_JOBS` still controls each Cargo process, so budget for up to
`N * CARGO_BUILD_JOBS` compiler jobs. Exact `package/target` selectors work with
either mode. Every selected target runs despite failures; per-target logs and
numeric exit statuses are retained. `results.tsv` also records build, test and
total wall seconds and whether execution reused a recorded passing run.
Final status lines and TSV rows retain inventory order, while live RUN lines
reflect dispatch order. The runner prints build/test totals and elapsed wall
time (including grouping); a proven-empty impact selection succeeds.
