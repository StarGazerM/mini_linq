# Basic query-lowering checkpoint: Start Here

Implement exactly three typed passes:

```text
CQ
  -- Basic HW1 --> RelationalPlan
  -- Basic HW2 --> IndexRequirements preserving the complete RelationalPlan
  -- Basic HW3 --> Physical (IX plus one physical operator and mode per result)
  -- supplied selection --> Structures (what answers each read)
  -- supplied printer --> one Rust block over the cq-storage traits
```

Before coding, read the language grammars and the triangle example in
[DSL.md](DSL.md). They show the program after every pass. Those artifacts,
the typed contracts, and the tests are the assignment specification.

This handout describes the student checkpoint. The staff implementation also
provides the broader routes listed in
[Compiler ownership and composition](COMPILER-STRUCTURE.md).

## What students write

The assignments follow three semantic compilation edges. Their implementations
in this checkout live in separate implementation modules:

- CQ lowering: `crates/cq-query-lowering/src/written_order/lower.rs`;
- input-index selection: `crates/cq-index-policy/src/lib.rs`;
- physical lowering: `crates/cq-physical-lowering/src/lib.rs`.

| Homework | Typed function | Successful result |
|---|---|---|
| Basic HW1 | `cq_query_lowering::written_order::compile(&cq::Module)` | `syn::Result<relational_plan::Module>` containing the unchanged source `Program` and a complete named relational plan |
| Basic HW2 | `cq_index_policy::compile(&relational_plan::Module)` | `syn::Result<index_requirements::Module>` containing the exact unchanged RelationalPlan plus its canonical equality keys |
| Basic HW3 | `cq_physical_lowering::compile(&index_requirements::Module, &Policy)` | `syn::Result<physical::Module>` containing the unchanged IX plus one physical operator, mode, and `canon` flag per result |

Staff supplies the syntax definitions, parsers, local contracts, edge
contracts, direct proc-macro entries, `cq_structure_selection::compile`,
`cq_physical_realization::compile`, fixtures, and the structural and runtime harness.
Do not add a second parser, a token-stream compiler API, a generic pass
framework, or an expansion wrapper.

The names reflect the software boundary:

```text
proc macro: TokenStream -> parse exact source type -> call typed compile_* -> ToTokens
compiler:   &TypedSource -> Result<TypedTarget>
```

The proc-macro entry owns grammar parsing. The typed function owns one
transformation and calls the source `contract::check`, returning span-aware
semantic errors in debug and release. Its debug postconditions describe the
edge to the target. A public macro entry does not repeat the source check.
Tests and tools can also call `contract::check` directly.

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
`output result as head;` metadata. For Triangle this creates six logical
results:

```text
r0 = rename R {c0 -> x, c1 -> y};
r1 = rename S {c0 -> y, c1 -> z};
r2 = natural_join r0 with r1;
r3 = rename T {c0 -> z, c1 -> x};
r4 = natural_join r2 with r3;
r5 = project r4 keep {x, y, z};
output r5 as triangle(x, y, z);
```

`rN` is a serialized DAG reference to a logical set relation, not an algebra
operator or assumed SSA prerequisite. It does not choose a row store, hash
index, Rust source expression, or iterator combinator. Construct the typed
result with `quote!` and `syn::parse2`; do not format a string and parse it
back.

RelationalPlan also contains the staff-supplied `rN = unit;`, which denotes
the singleton zero-attribute relation. The implemented expression route uses
it to seed a prefix before any atom; it can also seed optional global clauses.
A positive baseline CQ always begins with a Rename, so Basic HW1 must not
emit `Unit`.

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
direct Rename, or the plan contains `Unit`—return a span-aware `syn::Error`.
The implemented expression-prefix route accepts `Unit`, and the materialized
backend accepts bushy plans through a separate physical lowering. Neither
extends the basic homework's required domain.

## Basic HW3: IndexRequirements to Physical

This pass consumes the indexed logical plan and a query-independent policy
and returns a `physical::Module`: the unchanged IX plus, for every result,
one physical operator and one mode:

- a natural join becomes `probe` (the right result read through a key) or,
  when the policy says `join = merge` and both sides can be ordered on the
  key, `merge`; a semijoin becomes `semi`; a select over an ordered key may
  become `range`; every other operator keeps its logical name;
- a `pipe` is read exactly once by its consumer; a `let` is read through a
  key or more than once; an `alias` is a host relation read in place;
- a result is flagged `canon` when it must be a set and uniqueness is not
  inferred; the returned result must be canonical.

Nothing at this boundary names a Rust type, a loop, or a variable: operands
are result names and columns are attribute names. Do not return a storage
descriptor, builder plan, or token stream.

### Policy

The policy is query-independent: host facts (`R keyed [a] ordered canon`),
strategy (`join = merge`, `semijoin = prefer_lazy`), and `#[index(Type)]`
advice on relation declarations. The same policy is passed unchanged to the
supplied structure selection.

## Supplied structures and printer

`cq_structure_selection::compile` (Physical → Structures) binds every keyed
read to the host key that answers it or, when no host key does, to a Rust
type built over the result through `cq_storage::Build`; `#[index(Type)]`
advice names that type. It also discharges every `merge` and return
premise: an ordered host key, a built ordered type, or a pipe sorted by its
own merge.

`cq_physical_realization::compile` (Structures → Rust) prints one Rust block
over the `cq-storage` traits (`Build`, `IndexRead`, `ExactMembership`,
`OrderedRead`/`merge`/`sorted_by`, `Canonical`). Built structures are
constructed at the top of the block and guarded by `cq_storage::require`
naming the trait every read on them uses; the block's value is an
iterator of answer tuples in head order. A program over hosts alone
evaluates to the lazy iterator itself, so `next()`, `any()` and `count()`
execute traversal without materializing the result; a program that builds
structures collects its answers inside the block, since the structures live
only for the block. A `canon` result is deduplicated through the caller's
`distinct_state!` provider. The `structures_to_rust!` proc macro
only calls `ToTokens` on that block; there is no hidden code generator after
this pass.

## Recommended implementation order

1. Read the language grammars and the triangle example in `DSL.md`.
2. Implement and test Basic HW1 before touching index selection.
3. Implement Basic HW2 by walking the named logical operators.
4. Implement Basic HW3 by assigning each result its physical operator and
   mode from the policy and its uses.
5. Bind the supplied `select_structures!` and `structures_to_rust!` slots
   and read the printed Rust block.
6. Run formatting, contracts, structural tests, rustc-backed tests, and the
   independent workload oracle.

Do not put physical storage decisions back into RelationalPlan. In particular,
a fully bound later atom remains a NaturalJoin logically; membership is chosen
only during physical lowering. Do not reconstruct NaturalJoin structure during
index selection. If one pass becomes responsible for both logical and physical
decisions, its source or target IR is missing an explicit fact.

## Inspect one query, one pass at a time

`cargo xtask explain` is the MiniLinq layer stepper. It invokes the real typed
passes and writes each complete result as a `.tokens` file, so you can
inspect your own compiler output in the editor instead of comparing only a
final recursive macro expansion. It stops at IX.

Start with Triangle and stop after the layer you are implementing:

```text
cargo xtask explain triangle --through cq
cargo xtask explain triangle --through relational-plan
cargo xtask explain triangle --through index-requirements
```

The last command can be shortened to `cargo xtask explain triangle`, because
`index-requirements` is the default. Run `cargo xtask list` for the other catalog case
names. The `explain`, `list`, and `generate` commands use the lightweight tool
build and do not compile DuckDB; only `golden` and `differential`
transparently enable the optional oracle.

| `--through` value | Artifact | First student implementation required |
|---|---|---|
| `cq` | `01-cq.tokens` | none |
| `relational-plan` | `02-relational-plan.tokens` | Basic HW1 |
| `index-requirements` | `03-index-requirements.tokens` | Basic HW2 |

The files appear under `target/mini-linq-explain/triangle/`. Each is a
complete typed compiler payload, not an isolated
operator excerpt. The directory's `README.md` identifies the generated
files. Because the trace lives under ignored `target/`, its files are
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
Do not invoke expansion again from
the virtual `[EXPANSION].rs` buffer; it is not a supported source document and
may produce an editor error. Use the generated files for layer-by-layer
inspection and the recursive expansion only when you want the final code in
the context of its call site.

Usually the artifacts plus the focused tests below are sufficient. If you
want to step through the compiler implementation itself, place a breakpoint in
the module for that edge listed above and use the Rust Analyzer **Debug** CodeLens
on a focused test with the optional CodeLLDB extension installed. CodeLLDB is
not required for the homework; it is simply a conventional debugger for the
pass code, while `cargo xtask explain` remains the source-level IR stepper.

## Verification commands

Start with the smallest edge under construction:

```text
cargo test -p cq-ir --test relational_plan
cargo test -p cq-query-lowering --test written_order
cargo test -p cq-index-policy --test index_demands
cargo test -p cq-physical-lowering --test lowering
cargo test -p cq-structure-selection --test selection
cargo test -p cq-physical-realization --test realization
cargo test -p mini-linq --test physical_query
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

Check the authored workload catalog and compare the golden answers with
DuckDB:

```text
cargo test -p cq-workloads
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
