# Compiler review reconciliation

The earlier audit's blanket claim that every implementation and pass contract
matched the specification was incorrect. Rule inventory, source fingerprints,
and passing counterexample witnesses do not certify compiler correctness.

This cleanup keeps the existing IRs and compiler edges. It changes executable
premises and constructions where the review found failures, and updates the
affected specification instead of treating a recorded panic as successful
preservation.

## Binding-domain repairs

- Access Let binders are fresh relative to the current environment. Visible
  logical uses in the label's RHS must already be bound. This supplies the
  freshness premise required by Pipeline Join even for direct Access quotation.
- Public Access and Pipeline macros check their local contracts before calling
  the total typed lowering, producing diagnostics in debug and release.
- Both physical routes label Extend as an existing Let access over a singleton
  source. Empty tuples therefore retain RHS execution per prefix. They no
  longer generate a zero-argument positive label with a forbidden For.
- The empty-extension typed regression now requires successful lowering.
  Public execution tests check results and callback counts with duplicate and
  empty input, a root extension, and partial cursor demand.

## Administrative structure

ClauseFacts contains dependency and movement facts; it no longer caches a
complete scheduling command and duplicate clause identity. The rule kernel
matches the command against its source clause once. Nested checker calls reuse
checker-owned analysis without reusing the lowering's emitted derivation as
an oracle.

The default index edge owns its logical dataflow-domain analysis in
relational_to_indexes. Physical storage eligibility remains with the selected
physical implementation. The graph reducer uses reverse breadth-first order
for Up and forward breadth-first order for Down; both satisfy the Y-rule
dependencies, and the specification now names that actual schedule.

## Physical verification scope

The physical correspondence relation reads existing staged Rust syntax and
checks storage construction and Access regions against the source algebra.
Its source of truth is the incoming IndexedRA, not a call to the physical
emitter. Storage construction must be checked in execution order: finding
one correct builder somewhere in a method is insufficient if another statement
changes the store or a consumer runs before that builder.

The relation is structural. Rust type checking, macro/name-resolution
adequacy, arbitrary host effects, and the run/materialize convenience wrappers
retain separate obligations. A passing structural check is not an
unrestricted Rust contextual-equivalence theorem.

## Validation

Validation completed on 2026-09-09:

- `cargo test --workspace --locked`: 288 tests passed across 37 suites.
- `cargo test --workspace --release --locked`: 288 tests passed across 37
  suites. Both profiles include the binding regressions and physical mutation
  tests.
- The separate xtask suite: 18 tests passed, including compilation of a final
  Rust artifact and complete lowering of every catalog case.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo fmt --all -- --check`, and `git diff --check` passed.
- All three manuscript examples ran successfully. The constraint-planning
  example checked 14 input presentations through three public routes
  (42 executions), including intermediate arities and counts.
- `python3 doc/formal/check.py` checked 124 named rule links and 32 source
  fingerprints. The manuscript PDF built without LaTeX warnings or box
  diagnostics.

The source inventory records correspondence locations, not discharged
proofs. E1's stateful cross-route counterexamples and the manuscript's C-*
obligations remain applicable.
