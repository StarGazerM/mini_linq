# Executable paper overview

This small documentation driver calls the existing typed compiler passes and
both public CQ macros. It does not add an IR, compiler pass, or runtime feature.
The program in `src/main.rs` computes a join key with a Rust `let`, joins against
that key, applies an `if`, and computes a head component.

From the repository root:

```sh
cargo run --manifest-path doc/formal/example/Cargo.toml --locked
```

By default the driver writes into `doc/formal/build/example/`, which is ignored.
An optional command-line argument selects another output directory. Both final
Rust programs are compiled with `rustc --edition=2024` and executed. The two
public macro routes also execute within the driver. Exact sorted outputs and
materialized intermediate counts are asserted; converting outputs to a set does
not hide duplicate outputs.

For each `lazy` and `materialized` prefix, inspect:

| File suffix | Actual artifact |
| --- | --- |
| `01-cq.rs` | Source wrapped in the selected public CQ macro |
| `02-relational.rs` | Typed CQ pass output wrapped in the corresponding RA macro |
| `03-indexes.rs` | Typed index pass output wrapped in the physical macro |
| `04-access.rs` | The physical pass's ordinary `syn::File` containing `pull!` holes |
| `05-pipeline.rs` | Each existing Access hole replaced using `compile_pull` |
| `06-rust.rs` | Each Pipeline hole replaced using `compile_iterator_pipeline`, plus an executable assertion harness |

`report.txt` is the actual planned derivation. `validation.txt` records the
asserted results and counters. Pipeline expansion visits both expression macros
and tail statement macros in Syn's AST. It does not manually parse or synthesize
an approximation of a compiler pass's result. Whitespace inside printed macro
bodies follows `prettyplease` and may differ from the normalized paper listing.

The driver checks one pure, non-overflowing instance. It is not a proof of
cross-route equivalence for arbitrary host expressions, an exhaustive compiler
test suite, or a benchmark. The source is deliberately separate from the student
starter and belongs only in the answer repository.

## Atom-expression placement witness

```sh
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin argument_placement
```

The separate `argument_placement` binary executes
`answer(x) :- R(x, f(x), g());` through both public macro routes. It asserts
the exact answer and that `g()` runs once before the two calls `f(1)` and
`f(2)`. It permits either order of those two calls because the input sets do
not prescribe an enumeration order. This records the implemented A-Prefix
and A-Local evaluation placement; it does not assume that DSL atom matching
has Rust function-call argument semantics.
