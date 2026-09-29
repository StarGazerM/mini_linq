# Basic query-lowering checkpoint: Start Here

The checkpoint follows three homework boundaries. Basic HW1 composes the
CQ-to-RA nanopasses:

```text
CQ
  -- Basic HW1: normalize --> CQ-NF
  -- order (Written, Coarse) --> VO
  -- elaborate --> RA
  -- Basic HW2 --> IX: the unchanged RA plus exact-result demands
  -- Basic HW3 --> Physical: standalone declarations, detailed operators, modes, canon and output
  -- supplied selection --> Structures: Physical plus the bindings Γ answering each keyed read
  -- supplied printer --> one Rust block over the cq-storage traits
```

Before coding, read the language grammars in [DSL.md](DSL.md) and the
Triangle scenario at every stage:
[CQ](../test-support/relational/scenarios/triangle.cq),
[VO](../test-support/relational/scenarios/triangle.vo),
[RA](../test-support/relational/scenarios/triangle.ra),
[IX suffix](../test-support/relational/scenarios/triangle.ix),
[Physical](../test-support/relational/scenarios/triangle.physical),
[policy](../test-support/relational/scenarios/triangle.policy) and
[Structures suffix](../test-support/relational/scenarios/triangle.structures).
The IX and Structures files are suffixes; `Scenario::ix` and
`Scenario::structured` in
[scenarios.rs](../test-support/relational/scenarios.rs) append them to the RA
and Physical programs. Those artifacts, the typed contracts, and the tests are
the assignment specification.

This handout describes the student checkpoint. The staff implementation also
provides the broader routes listed in
[Compiler ownership and composition](COMPILER-STRUCTURE.md).

## What students write

The assignments follow these semantic compilation boundaries. Their
implementations in this checkout live in separate modules:

- CQ normalization: `languages/cq-lang-cq/src/normal.rs`;
- valuation ordering: `passes/cq-order/src/lib.rs`;
- RA elaboration: `passes/cq-elaborate/src/lib.rs`;
- demand derivation: `passes/cq-index-policy/src/lib.rs` (its check is in
  `correspondence.rs`);
- physical lowering: `passes/cq-physical-lowering/src/lib.rs`.

| Homework | Typed function | Successful result |
|---|---|---|
| Basic HW1 | `cq_lang_cq::normal::normalize` → `cq_order::compile` (`Written`, `Coarse`) → `cq_elaborate::compile` | `syn::Result<ra::Program>` containing declarations, the complete named relational plan and output metadata |
| Basic HW2 | `cq_index_policy::compile(&ra::Program)` | `syn::Result<ix::Program>` containing the exact unchanged RA plus its exact-result demands |
| Basic HW3 | `cq_physical_lowering::compile(&ix::Program, &Policy)` | `syn::Result<physical::Program>` containing declarations, full physical operators, modes, canon flags and output metadata |

Staff supplies the syntax definitions, parsers, language contracts, pass
correspondence checks, direct proc-macro entries,
`cq_structure_selection::compile`, `cq_physical_realization::compile`,
fixtures, and the structural and runtime harness.
Do not add a second parser, a token-stream compiler API, a generic pass
framework, or an expansion wrapper.

The names reflect the software boundary:

```text
proc macro: TokenStream -> parse exact source Program -> call typed pass::compile -> ToTokens
compiler:   &source::Program, options -> syn::Result<target::Program>
checker:    &source::Program, &target::Program, options -> syn::Result<()>
```

The proc-macro entry owns grammar parsing. Each typed stage checks its input
and returns span-aware semantic errors; `compile` also checks its target
contract. Each pass's separate `check(&source, &target, options…)` verifies
correspondence independently of `compile`; tests and tools call it, along
with local `contract::check`. Direct lowering entries delegate semantic
checking to the typed passes; the `mini_linq!` selector also validates CQ
before forwarding.

## Basic HW1: CQ to RA

The output is a complete named relational-algebra program, not a demand list.
Each body occurrence receives its own complete `rename` directly from the
atomic declared relation name. Every later operator names the exact earlier
result it consumes.

CQ arguments are positional against each declaration's ordered named columns.
`cq::normal::normalize` first puts each atom in kernel form with distinct
variable arguments, using bindings and guards for constants, repeats and
expressions and fresh witness variables for wildcards. `cq_order::compile` with `Written` and
`Coarse` schedules one full subatom per atom in body order. For a variable-only
body such as Triangle, elaboration maintains the current result and the next
unused `rN` number.

For each body occurrence:

1. emit a fresh Rename for that occurrence;
2. map every declared source attribute, in declaration order, to the
   occurrence's positional query variable;
3. if it is the first occurrence, make its Rename the current result;
4. otherwise emit `natural_join current with renamed_occurrence`, even if
   the complete right heading is already bound; and
5. make the NaturalJoin result current.

For Triangle's variable-only head, finish with one `project` keeping the head
attributes, then separate `output result as head;` metadata. A computed head
instead keeps the whole heading for its expression. For Triangle this creates
six logical results inside a standalone program that repeats the declarations:

```text
relational {
    relation R(first: i32, second: i32);
    relation S(first: i32, second: i32);
    relation T(first: i32, second: i32);
    r0 = rename R { first -> a, second -> b };
    r1 = rename S { first -> b, second -> c };
    r2 = natural_join r0 with r1;
    r3 = rename T { first -> c, second -> a };
    r4 = natural_join r2 with r3;
    r5 = project r4 keep { a, b, c };
    output r5 as answer(a, b, c);
}
```

`rN` is a serialized DAG reference to a logical set relation, not an algebra
operator or assumed SSA prerequisite. It does not choose a row store, hash
index, Rust source expression, or iterator combinator. Construct typed values
at each IR boundary; do not format a string and parse it back.

RA also contains `rN = unit;`, which denotes the singleton zero-attribute
relation. Elaboration uses it to seed a guard or binding before any atom,
including the bindings normalization introduces for constants. The
variable-only Triangle plan starts with a Rename.

The local RA contract checks headings, earlier operands, output and
reachability. `cq_order::check` verifies the split and Written-order
obligations of the VO plan; `cq_elaborate::check` checks declarations and head
and compares the kernel read back from RA with the CQ-NF kernel up to
variable renaming. `Written` with `Coarse` gives the Triangle sequence above.

## Basic HW2: RA to IX

Preserve the **entire** input RA program unchanged and append the demands
its joins and semijoins apply results at. Do not reconstruct the original CQ
walk and do not discard the logical operators.

Walk the definitions in order. For each NaturalJoin or Semijoin:

1. take its right operand, whatever earlier result it is;
2. list the ordinals, in the right result's sorted logical heading, of the
   attributes the left heading shares — possibly none, giving `[]`;
3. record the descriptor `(right result, ordinals)` only at its first use; and
4. name the recorded demands `i0`, `i1`, … in first-use order.

Triangle's `r1` has sorted heading `{b, c}` and shares `b`; `r3` has `{a, c}`
and shares both. Triangle therefore adds exactly:

```text
indexes {
    i0 = @r1[0];
    i1 = @r3[0, 1];
}
```

Demands identify exact RA results, not declared relations: self-join
occurrences are separate renames and do not share demands automatically, and
one result read on different columns gets different demands. A Cartesian join
records an explicit `[]` demand. `Unit` and computed right operands are in
the domain; a release that restricts input shapes states that as grading
scope, not as language or pass validity. `cq_index_policy::check` requires
the unchanged RA and exactly this named, ordered list.

## Basic HW3: IX to Physical

This pass consumes the logical plan with demands and a query-independent policy
and returns a `physical::Program`: declarations and output metadata plus, for every result,
one physical operator and one mode:

- a natural join becomes `probe` (the right result read through a key) or,
  when the policy says `join = merge` and the shared heading is nonempty,
  `merge`; a Cartesian join stays `probe[]`; a semijoin becomes `semi`; a
  select bounding an ordered host key by a literal may become `range`; every
  other operator keeps its logical name;
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

## Supplied structure selection and printer

A Structures program extends Physical with the bindings Γ: the value
answering each keyed read.

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
only for the block. A `canon` pipe or alias is deduplicated through the
caller's `distinct_state!` provider; a `canon` let is read from its
materialized set. The `structures_to_rust!` proc macro only calls `ToTokens`
on that block; there is no hidden code generator after this pass.

## Recommended implementation order

1. Read the language grammars in `DSL.md` and the Triangle scenario files.
2. Implement and test Basic HW1 before touching demand derivation.
3. Implement Basic HW2 by walking the named logical operators.
4. Implement Basic HW3 by assigning each result its physical operator and
   mode from the policy and its uses.
5. Bind the supplied `select_structures!` and `structures_to_rust!` slots
   and read the printed Rust block.
6. Run formatting, contracts, structural tests, rustc-backed tests, and the
   independent workload oracle.

Do not put physical storage decisions back into RA. A full
`Coarse` subatom remains a NaturalJoin even when all its variables are bound;
only a partial consistency check elaborates to a logical Semijoin. Physical
lowering decides how to implement membership. Do not reconstruct NaturalJoin
structure during demand derivation. If one pass becomes responsible for both
logical and physical decisions, its source or target IR is missing an explicit
fact.

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
cargo test -p cq-lang-ra --test relational_program
cargo test -p cq-lang-vo --test valuation_language
cargo test -p cq-order --test order
cargo test -p cq-elaborate --test elaborate
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
CQ and RA with Ascent-style negation and aggregation. Students edit the
production grammar and passes cumulatively; the course package's `hw4`
feature gates only that branch's tests. It does not switch the meaning of
`mini_linq!` or choose a backend.
