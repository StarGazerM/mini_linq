# Administrative query compilation across embedded languages

This is an **extended research manuscript with open proof obligations**.
[compiler.tex](compiler.tex) is the LaTeX entry point; `make` produces
[build/compiler.pdf](build/compiler.pdf). The central question is which
language, environment and observation contracts make a compiler pass
replaceable. The manuscript includes an executable overview, administrative
algorithms, conditional proofs, research context, and a conformance evaluation.

The specification distinguishes language well-formedness, algorithmic
transitions, semantic preservation, and correspondence with the Rust
implementation. A successful local check or test does not imply the other
properties. Conditional statements name their assumptions; known failures
remain failures of the claimed property.

[Terminology and source correspondence](TERMINOLOGY.md) records the AHV
definitions and the course adaptations used here. The foundations distinguish
sorts, named tuples, valuations, and the notes' valuation relations before
introducing compiler states. Mathematical renaming uses AHV's `δ` notation.
The assembly invariant explicitly converts a named result to head tuple order.

## Coverage

| Existing boundary or algorithm | Specification |
| --- | --- |
| Problem, architectural contribution and claim scope | [introduction.tex](introduction.tex) |
| Computed key, guard and head through every existing IR and both routes | [overview.tex](overview.tex), [executable artifact](example/README.md) |
| Sorts, tuples, valuations, CQ denotation, host effects and observations | [foundations.tex](foundations.tex) |
| Representation abstraction, hash versus sorted enumeration, demand, finite effect laws and provenance | [effects_provenance.tex](effects_provenance.tex) |
| Native Rust syntax, binding, logical dependencies, movement, evaluation population and staging | [host_embedding.tex](host_embedding.tex) |
| Lexical dependencies and the local judgments of every IR | [languages.tex](languages.tex) |
| CQ → RA: canonical lowering, GYO, assembly, selection, retention, two-pass tree reduction, dependency-constrained prefix lowering | [planning.tex](planning.tex) |
| RA → IndexedRA: base/result index selection and exact required-key coverage | [physical.tex](physical.tex) |
| IndexedRA → staged Rust: storage representations, basic/lazy/materialized realization, host expression elaboration | [physical.tex](physical.tex) |
| Access → Pipeline → Rust expression: finite successful cursor semantics and lowering rules | [staging.tex](staging.tex) |
| Quotation, residual holes, continuation composition and replacement of a pass | [staging.tex](staging.tex) |
| Implementation correspondence, proof obligations and refuted claims | [conformance.tex](conformance.tex) |
| Oracles, shape coverage, actual compilation paths and counterexamples | [evaluation.tex](evaluation.tex) |
| Prior query, compiler, effect and provenance theory; remaining research problem | [related_work.tex](related_work.tex) |

The mathematical states and contexts in the manuscript are specification
notation. They do not introduce a new compiler IR, runtime stage, or public
macro parameter. Public macro continuations are currently fixed. Negation,
aggregation, WCOJ, cardinality estimation and planning separate dependency
regions are not implemented algorithms in this snapshot.
Sorted storage and dynamic provenance instrumentation are also semantic
comparisons and research constructions, not new implemented IR features.

## What remains open

The relational proofs require a stated host domain: total deterministic
expressions that do not observe representation or evaluation order, lawful
and silent value operations, and adequate typed Rust elaboration. Accepting
an arbitrary Syn expression does not establish any of those properties.

The manuscript leaves these obligations explicit:

- **C-Rust:** generated types, ownership, captures, layouts and representation.
- **C-Resolution:** aliases, quotation, macro captures and intended name resolution.
- **C-Conformance:** simulation of each concrete implementation transition,
  including rejection and preservation of state on a rejected command.
- **C-Effects:** semantics and simulations for divergence, panic/unwinding and
  destruction. The current cursor proofs cover finite successful polling.

In particular, a theorem conditional on successful Rust elaboration does not
prove type preservation or total compilation of every locally well-formed
input. A correspondence manifest does not prove that a function implements
its associated rule.

Two counterexample families and a placement witness accompany the manuscript:

1. [formal_counterexamples.rs](../../crates/mini-linq/tests/formal_counterexamples.rs)
   shows that a stateful `let` followed by `if` returns both input rows through
   the lazy route and only one through the materialized route. A second test
   records different build and partial-consumption effect traces.
2. [formal_domains.rs](../../crates/cq-compiler/tests/formal_domains.rs) shows
   that `let () = ()` passes CQ/RA checks but fails both physical routes. The
   default route's support postcondition incorrectly predicts success: it
   panics in debug and returns a diagnostic in release. The materialized
   entry returns a diagnostic in both profiles.
3. [argument_placement.rs](example/src/bin/argument_placement.rs) confirms that
   `R(x, f(x), g())` evaluates `g()` once before two candidate-local `f` calls
   in both public routes. Its assertion permits either order of the two
   candidates. An atom is a matching construct; its effect schedule cannot
   be inferred from Rust function-call argument order.

The first family refutes unrestricted cross-route equivalence. Until the
source effect contract is chosen, it does not designate either route as the
reference semantics. The second exposes an actual lowering/support-contract
defect. The third records currently implemented placement. Passing these
checks must be interpreted according to those different claims.

## Build and validation

Requirements: Python 3, `latexmk`, pdfLaTeX, and the packages imported by
`compiler.tex` (including `mathpartir` and `stmaryrd`). From the repository root:

```sh
make -C doc/formal
make -C doc/formal example
cargo clippy --manifest-path doc/formal/example/Cargo.toml --locked --all-targets -- -D warnings
cargo test -p mini-linq --test formal_counterexamples --locked
cargo test -p cq-compiler --test formal_domains --locked
cargo test -p mini-linq --test formal_counterexamples --release --locked
cargo test -p cq-compiler --test formal_domains --release --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The three witness tests pass in debug and release on the recorded source
snapshot; workspace Clippy with warnings denied also passes. These are
focused audit checks, not a new whole-workspace test result.

The new overview driver passes through both public macros and both standalone
fully lowered Rust programs, asserting the exact output and intermediate
counts. The argument-placement binary passes in debug and release; the
example package passes Clippy on all targets with warnings denied. Generated
intermediate artifacts and assertions are in `build/example/`. The example
README describes their exact boundaries and reproduction commands.

The earlier regression logs record 269 passing workspace tests plus 18 xtask
tests before adding the three counterexample tests; the selected release gate
covered 109 tests, not the whole workspace. These counts describe those runs,
not a newly repeated broad test gate. The production fingerprints remain
unchanged during the manuscript extension. The detailed, reproducible
semantic experiment inventory is:

| Experiment | Source and instructions |
| --- | --- |
| GYO/tree enumeration | [hypergraph.rs](../../crates/cq-compiler/src/cq_to_relational/hypergraph.rs) |
| 25 authored queries, RA plans, independent valuations and contracts | [stage1_reference.rs](../../crates/cq-compiler/tests/stage1_reference.rs), [Stage I verification](../STAGE-I-VERIFICATION.md) |
| Four-policy typed compilation | [stage1_production.rs](../../crates/cq-compiler/tests/stage1_production.rs) |
| Literal public macros | [stage1_shapes.rs](../../crates/mini-linq/tests/stage1_shapes.rs) |
| 12 expression patterns and 120 schedules | [expression_planning.rs](../../crates/cq-compiler/tests/expression_planning.rs) |

The Stage I verification document gives the test commands, fixture-by-fixture
RA expectations, data generation domains and independent-oracle boundaries.

[rules.tsv](rules.tsv) locates all 119 named inference rules in source or marks
them as abstract specification rules. [check.py](check.py) checks rule inventory,
the 14 executable administrative rule variants, marker existence, and the
29 fingerprints in [sources.sha256](sources.sha256). It checks documentation
coverage and source identity, **not mathematical proofs or semantic conformance**.
Run `make -C doc/formal check` to perform those checks without building the PDF.

The snapshot includes the uncommitted rule refactor on `codex/all-answers`;
[snapshot.txt](snapshot.txt) records its Git parent. A fingerprint failure
requires reviewing the changed source and affected rules before refreshing
the snapshot. Generated PDF/build files are ignored by Git. This material
belongs to the answer repository and has not been copied to the starter.
