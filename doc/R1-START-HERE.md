# Basic query-lowering checkpoint: Start Here

Implement exactly three typed passes:

```text
CQ
  -- Basic HW1 --> RelationalPlan
  -- Basic HW2 --> IndexRequirements preserving the complete RelationalPlan
  -- Basic HW3 --> complete staged Rust file with one pull! { RustAccessPlan }
  -- supplied pull --> same file with iterator_pipeline! { IteratorPipeline }
  -- supplied Rust lowering --> final ordinary Rust file
```

Before coding, read the [complete six-artifact Triangle
expansion](DSL.md#complete-triangle-expansion). It shows the entire program
after every pass. Those artifacts, the typed contracts, and the tests are the
assignment specification.

## What students write

The three executable homework bodies are in
`crates/cq-compiler/src/passes.rs`.

| Homework | Typed function | Successful result |
|---|---|---|
| Basic HW1 | `compile_cq(&cq::Module)` | `syn::Result<relational_plan::Module>` containing the unchanged source `Program` and a complete named relational plan |
| Basic HW2 | `compile_relational_plan(&relational_plan::Module)` | `syn::Result<index_requirements::Module>` containing the exact unchanged RelationalPlan plus its canonical equality keys |
| Basic HW3 | `compile_index_requirements(&index_requirements::Module)` | `syn::Result<syn::File>` containing the complete sequential `Program + Storage` API and exactly one expression-position `pull!` region |

Staff supplies the syntax definitions, parsers, local contracts, edge
contracts, direct proc-macro entries, `compile_pull`,
`compile_iterator_pipeline`, fixtures, and the structural and runtime harness.
Do not add a second parser, a token-stream compiler API, a generic pass
framework, or an expansion wrapper.

The names reflect the software boundary:

```text
proc macro: TokenStream -> parse exact source type -> call typed compile_* -> ToTokens
compiler:   &TypedSource -> Result<TypedTarget>
```

The proc-macro entry owns grammar parsing. The typed function owns one
transformation. Its debug precondition is the semantic source contract, and
its postconditions describe the edge to the target. A public macro entry does
not repeat `contract::check`, because that would traverse the same typed input
twice. The diagnostic `contract::check` functions remain available to tests
and tools that explicitly want span-aware semantic errors.

## Basic HW1: CQ to RelationalPlan

The output is a complete named relational-algebra program, not an index list.
Each body occurrence receives its own complete `rename` directly from the
atomic declared relation name. Every later operator names the exact earlier
result it consumes.

CQ arguments are positional against each declaration's ordered named columns.
Walk the positive CQ body from left to right while maintaining the current
result and the next unused `rN` number.

For each body occurrence:

1. emit a fresh Rename for that occurrence;
2. map every declared source attribute, in declaration order, to the
   occurrence's positional query variable;
3. if it is the first occurrence, make its Rename the current result;
4. otherwise emit `natural_join current with renamed_occurrence`, even if the
   complete right heading is already bound; and
5. make the NaturalJoin result current.

Finally emit one `project` keeping the source-head attributes, then separate
`output result as head.` metadata. For Triangle this creates six logical
results:

```text
r0 = rename R {c0 -> x, c1 -> y};
r1 = rename S {c0 -> y, c1 -> z};
r2 = natural_join r0 with r1;
r3 = rename T {c0 -> z, c1 -> x};
r4 = natural_join r2 with r3;
r5 = project r4 keep {x, y, z};
output r5 as triangle(x, y, z).
```

`rN` is a serialized DAG reference to a logical set relation, not an algebra
operator or assumed SSA prerequisite. It does not choose a row store, hash
index, Rust source expression, or iterator combinator. Construct the typed
result with `quote!` and `syn::parse2`; do not format a string and parse it
back.

RelationalPlan also contains the staff-supplied `rN = unit;`, which denotes
the singleton zero-attribute relation. It exists for global clauses in the
negation-and-aggregation extension. A positive baseline CQ always begins with
a Rename, so Basic HW1 must not emit `Unit`.

The local contract accepts arbitrary unique result identifiers and checks only
local heading rules, earlier operands, output, and reachability. It does
not enforce CQ occurrence or source order. The separate exact pass contract
defines the canonical `r0`, `r1`, and so on sequence above.

## Basic HW2: RelationalPlan to IndexRequirements

Preserve the **entire** input RelationalPlan, byte-for-byte at the typed syntax
level, and append the equality-index overlay required by its probes. Do not
reconstruct the original CQ walk and do not discard the logical operators.

The basic sequential backend accepts a supported left-deep plan. For each
NaturalJoin:

1. resolve its right operand to an earlier direct Rename;
2. compute the intersection of the left and right inferred headings;
3. invert the right Rename's source-attribute mapping to increasing positions
   in declaration order;
4. emit no requirement for an empty intersection; and
5. emit each `(declared relation, key columns)` only at its first use.

Triangle therefore adds exactly:

```text
indexes {
    S[0];
    T[0, 1];
}
```

This rule supports self-joins, reuse of one index by several Rename
occurrences, and several different indexes on one declared relation. The
output still contains all six Triangle `rN` definitions; see the complete
third artifact in `DSL.md`.

If a locally well-formed relational plan is not in the basic backend's
supported left-deep form—for example, a NaturalJoin's right operand is not a
direct Rename, or the plan contains `Unit`—return a span-aware `syn::Error`. The
negation-and-aggregation extension consumes `Unit`; a future bushy or materializing
backend may consume the same logical language through a different physical
lowering.

## Basic HW3: IndexRequirements to staged Rust

This pass consumes the indexed logical plan and returns a complete
`syn::File`. It owns every remaining baseline storage and Rust-access decision:

- the public `Program` and owning `ProgramStorage` declarations;
- exact input, row, key, payload, iterator-item, and result types;
- generated `inputN`, `relationN`, `rowsN`, `indexN`, and builder-local names;
- normalization and construction of every required or explicitly written
  physical object;
- each concrete Rust scan, lookup, membership condition, and output row;
- eager `build`, lazy `query`, deterministic `materialize`, and convenience
  `run` methods; and
- one local occurrence-annotated `pull! { rust_access_plan::Plan }` expression.

Do not return a storage descriptor, builder plan, isolated RustAccessPlan, or
token stream. At this boundary, the struct declarations and all build/query
shell syntax are already concrete Rust AST.

### Storage policy

Normalize every used input exactly once into `HashSet<Row>` so public input
duplicates obey relation set semantics. An input used by neither the logical
query nor a written extra index is dropped without invoking its iterator.

Choose exactly one final ownership-taking physical consumer per normalized
relation:

- a final row store consumes it with `into_iter().collect::<Vec<_>>()`;
- a final proper-key map consumes its rows while building the map; or
- a final full-key index takes the normalized `HashSet<Row>` by direct move.

If the relation needs additional physical representations, build them first
by borrowing `relationN.iter()` and cloning the key and complement fields that
the new representation must own. Never recollect a full-key set identical to
the normalized relation.

Build every locally valid index written in `IndexRequirements`, including an
extra one not used by the query. If the logical plan needs a key that the
module omitted, return a span-aware error at the needing relation occurrence.

The baseline representations are:

| Access | Concrete storage | Query action |
|---|---|---|
| unbound scan | `Vec<Row>` | borrow with `rowsN.iter()` |
| proper equality key | `HashMap<Key, Vec<Complement>>` | borrow payloads with `get(&key).into_iter().flatten()` |
| full equality key | `HashSet<Row>` | test with `contains(&row)` |

Key columns follow the written increasing column order. Complement columns
follow base-relation order. Query traversal borrows stored rows and payloads;
do not append `.cloned()` to scan or lookup sources. Rust match ergonomics
therefore bind the positive variables as references. Generated lookup keys,
membership rows, and the owned result tuple clone only the selected fields.

### Generated API

`Program::build(inputs...) -> ProgramStorage` performs all data work. It
normalizes inputs, builds row stores and indexes, and returns their owner.

`Storage::query(&self) -> impl Iterator<Item = Row> + '_` returns one lazy
iterator for the entire distinct result set. Its whole body is exactly one
`pull!` expression. Creating the iterator reads no stored row; the first
`next()` starts the root access.

`Storage::materialize(&self) -> Vec<Row>` collects `self.query()`, sorts the
already-distinct rows with `sort_unstable`, and returns them. It does not
perform a second distinctness algorithm.

`Program::run(inputs...) -> Vec<Row>` is exactly the convenience path
`Self::build(inputs...).materialize()`.

Index building is eager and occurs only in `build`; `query` is the
database-driver-style result cursor and the only generated cursor method.

## Supplied pull path

The first supplied pass maps each concrete access to a named
`IteratorPipeline`:

```text
RustAccessPlan
  -> iter0 = scan (x, y,) in (self.rows0.iter()) yield (x, y,)
  -> iter1 = join iter0 as (x, y,) with (z,) in (self.index0.get(&(y.clone(),)).into_iter().flatten()) yield (x, y, z,)
  -> iter2 = filter iter1 as (x, y, z,) if (self.index1.contains(&(z.clone(), x.clone(),)))
  -> iter3 = project iter2 as (x, y, z,) yield ((x.clone(), y.clone(), z.clone(),))
  -> iter4 = distinct iter3
  -> return iter4
```

Every `iterN` is one lazy iterator value, not a relation and not a loop that
materializes its input. The plan exposes the conventional left-deep binary
operator boundaries and intermediate binding tuples. It deliberately does not
contain the Rust names `map` or `flat_map`.

The second supplied pass performs the semantic lowering from those operators
to one ordinary Rust `syn::Expr`. It creates one named `let iterN` per plan
definition. `scan` becomes `map`, binary join becomes `flat_map` whose right
side is a parameterized `map`, predicates become `filter`, projection becomes
`map`, and distinctness becomes a stateful `HashSet` filter.

An outer `once_with(move || { ... }).flatten()` defers even the root source
expression until the first `next()`. This expression is the final concrete
IR. The `iterator_pipeline!` proc macro only calls `ToTokens` on it; there is
no hidden code generator after this pass.

## Recommended implementation order

1. Read the six complete Triangle artifacts in `DSL.md`.
2. Implement and test Basic HW1 before touching index selection.
3. Implement Basic HW2 by walking the named logical operators.
4. Implement the Basic HW3 storage schema and builders.
5. Add the physical walk that translates the same logical operator chain into
   concrete Rust accesses.
6. Emit the complete `syn::File` around the one local `pull!` expression.
7. Run formatting, contracts, structural tests, rustc-backed tests, and the
   independent workload oracle.

Do not put physical storage decisions back into RelationalPlan. In particular,
a fully bound later atom remains a NaturalJoin logically; membership is chosen
only during physical lowering. Do not reconstruct NaturalJoin structure during
index selection. If one pass becomes responsible for both logical and physical
decisions, its source or target IR is missing an explicit fact.

## Inspect one query, one pass at a time

`cargo xtask explain` is the MiniLinq layer stepper. It invokes the real typed
passes and writes each complete result as an ordinary `.rs` file, so you can
inspect your own compiler output in the editor instead of comparing only a
final recursive macro expansion.

Start with Triangle and stop after the layer you are implementing:

```text
cargo xtask explain triangle --through cq
cargo xtask explain triangle --through relational-plan
cargo xtask explain triangle --through index-requirements
cargo xtask explain triangle --through rust-access-plan
cargo xtask explain triangle --through iterator-pipeline
cargo xtask explain triangle --through final-rust
```

The last command can be shortened to `cargo xtask explain triangle`, because
`final-rust` is the default. Run `cargo xtask list` for the other catalog case
names. The `explain`, `list`, and `generate` commands use the lightweight tool
build and do not compile DuckDB; only `golden` and `differential`
transparently enable the optional oracle.

| `--through` value | Artifact | First student implementation required |
|---|---|---|
| `cq` | `01-cq.rs` | none |
| `relational-plan` | `02-relational-plan.rs` | Basic HW1 |
| `index-requirements` | `03-index-requirements.rs` | Basic HW2 |
| `rust-access-plan` | `04-rust-access-plan.rs` | Basic HW3 |
| `iterator-pipeline` | `05-iterator-pipeline.rs` | Basic HW3; the pull lowering is supplied |
| `final-rust` | `06-final-rust.rs` | Basic HW3; both later lowerings are supplied |

The files appear under `target/mini-linq-explain/triangle/`. Artifact 04 is
the **entire staged Rust file** whose one expression hole is
`pull! { RustAccessPlan }`; artifact 05 is that entire file with the hole
replaced by `iterator_pipeline! { IteratorPipeline }`. They are not isolated
operator excerpts. The directory's `README.md` identifies the generated
files. Because the trace lives under ignored `target/`, its `.rs` files are
editor-readable syntax and diff snapshots rather than modules in the Cargo
crate graph. Rust Analyzer does not type-check them as workspace code; use the
real query call site, focused tests, or `cargo check` for type diagnostics.

If a requested compiler edge fails, the command exits unsuccessfully, keeps
every earlier artifact successfully produced in that run, removes stale files
from later stages, and writes the diagnostic to `ERROR.md`. This lets you open
the last valid program beside the error without accidentally reading output
from an older successful run.

### Use it from VS Code

1. Open the repository root in VS Code.
2. Choose **Tasks: Run Task** from the Command Palette.
3. Select **MiniLinq: Explain query**.
4. Accept `triangle` or enter another catalog case, then choose a stopping
   stage.
5. Click the absolute artifact paths printed in the terminal.
6. In the Explorer, choose **Select for Compare** on one stage and **Compare
   with Selected** on the next stage.

The task runs the same `cargo xtask explain <case> --through <stage>` command;
it deliberately prints paths instead of depending on the `code` command to
open an editor window.

Rust Analyzer's **Expand macro recursively at caret** is a separate final-code
view, not a Racket-style stepper. It recursively expands to ordinary Rust.
Moreover, `workload_query!` directly calls the first three typed compiler
functions when compiled queries are enabled, so their results are not nested
macro calls for Rust Analyzer to pause on. Do not invoke expansion again from
the virtual `[EXPANSION].rs` buffer; it is not a supported source document and
may produce an editor error. Use the six generated files for layer-by-layer
inspection and the recursive expansion only when you want the final code in
the context of its call site.

Usually the artifacts plus the focused tests below are sufficient. If you
want to step through the compiler implementation itself, place a breakpoint in
`crates/cq-compiler/src/passes.rs` and use the Rust Analyzer **Debug** CodeLens
on a focused test with the optional CodeLLDB extension installed. CodeLLDB is
not required for the homework; it is simply a conventional debugger for the
pass code, while `cargo xtask explain` remains the source-level IR stepper.

## Verification commands

Start with the smallest edge under construction:

```text
cargo test -p cq-ir --test relational_plan
cargo test -p cq-compiler --test pass_contracts cq_to_relational_plan_contract -- --ignored
cargo test -p cq-compiler --test pass_contracts relational_plan_to_index_requirements_contract -- --ignored
cargo test -p cq-compiler --test pass_contracts index_requirements_to_staged_rust_contract -- --ignored
cargo test -p cq-compiler --test documented_triangle
cargo test -p cq-compiler --test pull_lowering
cargo test -p mini-linq --test endpoint
```

Then run the full baseline:

```text
cargo fmt --all --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test --manifest-path xtask/Cargo.toml --locked
```

After all three TODOs work, enable direct compilation of the authored workload
queries and compare them with DuckDB:

```text
cargo test -p cq-workloads --features compiled-queries
cargo xtask golden all tiny --scenario all
```

Use `cargo xtask explain <case> --through <stage>` during these focused tests
to retain the last successful whole-program artifact for inspection.

The [negation-and-aggregation option](HW4-OPTIONAL.md) is one branch in the
required pick-one extension menu after this baseline. It cumulatively extends
CQ and RelationalPlan with Ascent-style negation and aggregation. Students edit
the production grammar and passes cumulatively; the legacy-named `hw4` feature
gates only that branch's tests. It does not switch the meaning of `mini_linq!`
or choose a backend.
