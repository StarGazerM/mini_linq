# Executable paper overview

This small documentation driver calls the existing typed compiler passes and
both public CQ macros. It does not add an IR, compiler pass, or runtime feature.
The program in `src/main.rs` follows the actual Yannakakis route:

```text
answer(x,y) :- R(x,k,tag,unused), S(k,tag,z), T(z,y), if y > 0;
```

Its join tree is `R --{k,tag}-- S --{z}-- T`, rooted at `T`. The composite
separator distinguishes rows sharing `k` but differing in `tag`. Shared
columns `k`, `tag`, and `z` are required for joining and disappear before
the answer. The separate private column `unused` is pruned after reduction;
that projection merges two distinct surviving input tuples.

From the repository root:

```sh
cargo run --manifest-path doc/formal/example/Cargo.toml --locked
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --release
```

By default the driver writes into `doc/formal/build/example/`, which is ignored.
An optional command-line argument selects another output directory. Both final
Rust programs are compiled with `rustc --edition=2024` and executed. The two
public macro routes also execute within the driver. Exact sorted outputs and
materialized intermediate counts are asserted; converting outputs to a set does
not hide duplicate outputs.

The driver asserts the exact join-tree certificate, administrative schedule,
RA and index requirements. Both upward semijoins and both downward semijoins
remove rows. The filter is applied to `T` before reduction. Retention then
drops `unused`, `z`, and finally `{k,tag}` at their respective last-use points.
All thirteen intermediate `(arity,count)` pairs are checked. The answer is
`[(1,7),(1,8),(2,9)]`; counters record 20 semijoin probes, six candidate join
pairs, and 19 rows processed across index constructions.

Both public macro routes and both final Rust executables check original,
reversed duplicate, empty-R, empty-S, and empty-T inputs: 20 executions,
plus one instrumented run. A whole-tuple oracle checks the public-route
answers; standalone Rust harnesses assert fixed expected results for the
same presentations. The default route provides the canonical non-Yannakakis
comparison; the manuscript follows the planned route through its lower IRs.

For each `lazy` and `materialized` prefix, inspect:

| File suffix | Actual artifact |
| --- | --- |
| `01-cq.rs` | Source wrapped in the selected public CQ macro |
| `02-relational.rs` | Typed CQ pass output wrapped in the corresponding RA macro |
| `03-indexes.rs` | Typed index pass output wrapped in the physical macro |
| `04-access.rs` | The physical pass's ordinary `syn::File` containing iterator-valued `pull!` or set-valued `collect_access!` holes |
| `05-pipeline.rs` | Each existing Access hole replaced using `cq_pull::compile` |
| `06-rust.rs` | Each Pipeline hole replaced by its iterator/set Rust endpoint, plus an executable assertion harness |

`report.txt` is the actual planned derivation. `validation.txt` records the
asserted results and counters. Pipeline expansion visits both expression macros
and tail statement macros in Syn's AST. It does not manually parse or synthesize
an approximation of a compiler pass's result. Whitespace inside printed macro
bodies follows `prettyplease` and may differ from the normalized paper listing.

The driver checks one pure query on these five input presentations. It is not a proof of
cross-route equivalence for arbitrary host expressions, an exhaustive compiler
test suite, or a benchmark. The source is deliberately separate from the student
starter and belongs only in the answer repository.

## Computed-key prefix route

```sh
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin computed_key
```

[computed_key.rs](src/bin/computed_key.rs) compiles the computed-key query:
`answer(x,*y+1) :- R(x,k), let next:i32 = *k+1, S(next,y), if y > 0;`.
It records all six boundaries of both compositions under `build/computed-key/`
relative to `doc/formal/`, and compiles and runs both final Rust files. The
computed key and head select dependency-constrained prefix rules; this example
has no GYO certificate or Yannakakis sweeps. Its answer is
`[(1,11),(2,11),(3,5)]`, with intermediate counts asserted.

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

## Interacting argument dependencies and an authored plan

```sh
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin constraint_planning
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --release --bin constraint_planning
```

[constraint_planning.rs](src/bin/constraint_planning.rs) gives the complete
five-relation query with two computed atom arguments, a dependent `let`, and
two filters. Its three literal public entries compile the same source through
the lazy and planned CQ routes, and an independently authored RA through
`planned_relational_plan!`. The authored plan places filters at different
subtrees, computes a binding before a later join, prepares a semijoin-filtered
right input, and prunes expression inputs after constructing a composite key.

A whole-tuple oracle and fixed authored answers check 14 input presentations
(42 compared query executions), including each relation emptied individually and
reversed duplicate input. Selected actual intermediate arities/counts are
also asserted in one additional instrumented run. The dependent guard is
necessary to meet those intermediate expectations; on this particular input,
omitting it would not change the final answer. Functions use total wrapping
arithmetic and integer comparison.
All three results agree in debug and release. The current planner does not
discover the authored plan: this is evidence that an application can supply
such a plan through an existing lower language and reuse the backend.

## Runtime representation choice through existing Access macros

```sh
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin access_representations
```

[access_representations.rs](src/bin/access_representations.rs) uses the same
`pull!` control shape over a native `HashSet` and an exact bounded `u64` bitmap.
Ordinary Rust selects a branch from the observed build-key range. Both macros
expand at compile time; storage construction and selection execute at runtime.
The representations need no common trait, and the compiler is unchanged.

A separate example applies an intentionally collision-prone eight-bucket filter
before exact membership. All 264 cases agree with an independent input-membership
reference, including empty inputs, duplicates, signed-key extremes, the 64/65-key
range boundary, and all 256 subsets of eight signed keys. The run selects bitmap
262 times and hash twice, and removes 1,672 false-positive probes.

The cutoff is illustrative. This checks local access and macro composition;
it does not implement a YA schedule, production Bloom filter, zone-map source,
factorized join, or optimizer cost model. The architectural placement and
proof obligations are in [architecture_placement.tex](../architecture_placement.tex).
