# Planned joins through the existing compiler languages

Implementation plan, started 2026-09-06. This combines Stage I functionality with the
language-oriented architecture correction. R2 is a project milestone, not a
compiler module or another IR. R1 remains a separately callable reference route.

## Pass boundaries and ownership

| Source | Pass | Target | Decision owned by this edge |
|---|---|---|---|
| `cq::Module` | `compile_cq_planned` | `relational_plan::Module` | Occurrence analysis, legal join order, semijoin reduction, selection and projection placement |
| `relational_plan::Module` | `compile_result_indexes` | `index_requirements::Module` | Exact first-use keys on named right operands; preserve the entire RA |
| `index_requirements::Module` | `compile_materialized` | staged `syn::File` with local `pull!` holes | Tuple representation, hash storage, existence access, materialization and DAG sharing |
| `rust_access_plan::Plan` | existing `compile_pull` | `iterator_pipeline::Pipeline` | Explicit local physical streams and binding flow |
| `iterator_pipeline::Pipeline` | existing `compile_iterator_pipeline` | `syn::Expr` | Rust iterator realization |

Macro entries parse exactly the source language and call its typed pass.
`planned_query!` routes to `planned_relational_plan!`, which routes to
`materialized!`, which emits the staged file. Existing R1 macro meanings stay
unchanged. Any intermediate artifact can be quoted directly at its entry.

The compiler implementation is private and divided by transformation:
`cq_to_relational` (with analysis, rules, and strategy helpers),
`relational_to_indexes`, and `indexes_to_rust`. Public compiler functions and
their contracts remain in `passes.rs`. Local language contracts remain in
`cq-ir`; cross-language relations remain under `pass_contracts`.

Within CQ-to-RA, `dependencies.rs` derives D-* clause judgments and
`hypergraph.rs` implements G-* reductions. `rules.rs` implements P-* and Y-*
transitions on `JoinState`; `clause_rules.rs` implements C-* transitions on
`ClauseState`, with A-* argument judgments composing C-MATCH. `strategy.rs`
chooses among legal candidates. The [expression rule specification](RUST-EXPRESSIONS.md#dependency-judgments-and-their-code)
pairs each judgment and transition with its code and states the restricted
domain in which graph planning currently applies.

The expression checker reads the actual RA separately from emission. It shares
clause dependency analysis, which is tested against authored facts and legal
orders; sharing that analysis is not an independent proof of its correctness.

Rust lexical scope remains in `cq::rust::free_variables`. Clause type
ascriptions belong to `Let::binding_types`; the basic language fragment is
recognized by `Query::is_basic`. Whole-query planning and generated CQ names
are owned by `cq_to_relational`.

No `Standalone` RA wrapper, public `stage1` subsystem, new physical language,
generic compiler engine, or direct RA-to-final-Rust shortcut is required.
The existing RA module retains its program annotation. Lowering consumes the
written algebra, not a reconstructed plan from that annotation.

## Necessary language extensions

1. RA `semijoin left with right` records an existence operation, including
   composite and empty separators. Its meaning is join then left projection;
   physical lowering must not enumerate matching pairs.
2. CQ arguments, `if`, and `let` retain native Rust expressions; RA uses
   `select` and `extend` for their computations. A bounded comparison
   recognizer admits i32 variables/literals and six comparisons for movement.
   Every occurrence of a comparison variable must have a syntactically known
   primitive type. Arbitrary Rust calls and arithmetic preserve evaluation
   boundaries. Physical comparison lowering also anchors operands to i32.
3. `IndexRequirement` gains an explicit result namespace: `@r[0, 2]` refers
   to attributes at those ordinals in the sorted heading of named result `r`.
   Existing `R[0]` continues to mean declared input-column positions. The `@`
   prevents a result and an input with equal spellings from being confused.
   Empty keys have no index entry. Indexes carry requirements, not containers.
4. RustAccessPlan checks a logical comparison label paired with an existing
   Rust `if` access, and permits empty local headings for Unit, projection to
   `{}`, and empty-separator existence guards. Empty occurrences require `if`,
   not `for`. The pull and IteratorPipeline operators need no extension.

The invented CQ `escape` keyword and detached schema/module wrapper are
removed. Quotation tests exercise existing language entry points and Rust
expression holes. A future CQ-level embedding requires a separate binding
and composition specification.

## Why existing local physical IRs suffice

Stage I deliberately materializes each named set result. A staged Rust file
therefore has several local cursor holes, one per RA definition, separated by
ordinary Rust bindings and index builders. Each hole uses at most a scan and
a join/guard, then projection and distinct. This obeys the existing linear
IteratorPipeline contract; a shared RA result is computed once and borrowed
by subsequent regions. We do not pretend that the whole RA DAG is one linear
pipeline or reconstruct its order in the last emitter.

For `l = rename R ...; r = rename S ...; f = semijoin l with r`, the index
pass writes an index on **r**, and the physical pass emits an existence test
against that index inside f's local access plan. If r is already reduced,
the index is built from that reduced result after its definition. Projection
changes the actual stored tuple type at its own materialization boundary.

The generated API keeps `build`, `query`, `materialize`, and `run`: build
performs the specified blocking work; query traverses the stored answer;
materialize sorts distinct output tuples. Work counters report result arities
and counts, index construction, join matches, and semijoin probes.

## Administrative rules and contracts

Occurrence IDs never merge duplicate-looking atoms. GYO updates a residual
analysis graph and attachment forest; it never deletes executable atoms.
Reduction and assembly have separate invariants. Reduction uses immutable
RA versions with a current-reference map, child-before-parent upward steps,
parent-before-child downward steps, and an explicit completed freeze.
Independent sibling steps may be reordered.

Assembly frontier coverages partition all occurrences. Retention is
`vars(C) ∩ (head ∪ vars(outside C) ∪ vars(pending predicates))`. Ready
predicates are consumed before pruning; exact boundary pruning precedes the
next combine. Every combine checks all shared attributes. Output requires
complete occurrence and predicate coverage and exactly the original head.

The derivation checker reads actual emitted RA definitions and checks rule
premises, dependencies, and facts. It must not call the strategy or emitting
rule implementation to regenerate the expected program. Graph recognition
also has an independent running-intersection check and a bounded tree oracle.
Arity bounds concern every named RA result, not cardinality or fractional
hypertree width. A greedy arity policy makes no global optimality claim.

The rule-to-code correspondence is explicit:

| Formal operation | Administrative command and implementation |
|---|---|
| G-PRIVATE / G-ABSORB | `GyoRule::Private` / `Absorb`; `GyoState::remove_private` / `absorb` in [hypergraph.rs](../crates/cq-compiler/src/cq_to_relational/hypergraph.rs) |
| P-START | `Introduce`; `JoinState::introduce` |
| P-SELECT | `Select`; `JoinState::select` |
| P-BEGIN | `Begin`; `JoinState::begin` |
| Y-UP / Y-DOWN | `Up` / `Down`; `JoinState::up` / `down`, each with its own premises |
| Y-FREEZE | `Freeze`; `JoinState::freeze` |
| P-COMBINE / P-TRIM | `Combine` / `Trim`; `JoinState::combine` / `trim`, using `retained_heading` for K(C,P) |
| P-OUTPUT | `Output`; `JoinState::output` |
| D-ATOM / D-GUARD / D-BIND / D-MOVE / D-GRAPH | `ClauseDependencies::analyze` and `ClauseFacts::atom` in [dependencies.rs](../crates/cq-compiler/src/cq_to_relational/dependencies.rs) |
| C-READY | `ClauseState::require_ready` / `require_clause` |
| C-CONSTANT | `Constant`; `ClauseState::normalize_constant` |
| C-MATCH | `Match`; `ClauseState::match_clause`, composed from A-* in `AtomBindings::match_argument` |
| C-GUARD / C-BIND | `Guard` / `Bind`; `ClauseState::guard_clause` / `bind_clause` |
| C-FINISH | `Finish`; `ClauseState::finish` establishes the output |

The graph transitions are in [rules.rs](../crates/cq-compiler/src/cq_to_relational/rules.rs);
the prefix transitions are in [clause_rules.rs](../crates/cq-compiler/src/cq_to_relational/clause_rules.rs).
[strategy.rs](../crates/cq-compiler/src/cq_to_relational/strategy.rs) selects
frontier pairs or ready clauses. The [edge checker](../crates/cq-compiler/src/pass_contracts/planned_join.rs)
consumes the actual emitted definitions and report. Downstream passes consume
only their RA/index languages, with no dependency on graph state, policy, or reports.

The [complete CQ artifact](planned-example/01-cq.rs) expands through
[RA](planned-example/02-relational.rs), [indexes](planned-example/03-indexes.rs),
[local access plans](planned-example/04-access.rs),
[pipelines](planned-example/05-pipelines.rs), and [Rust](planned-example/06-rust.rs).
The boundary tests compare these files to actual pass output and compile each
entry separately.

## Implementation sequence and completion gates

1. Remove the rejected wrappers/routes and restore language ownership. Retain
   the 25 reference fixtures and independent valuation/RA interpreters.
2. Integrate graph planning behind the CQ-to-RA typed pass; split the checker
   from emission and test broken traces plus alternate valid schedules.
3. Extend index syntax/checking and implement exact result-key derivation.
   Test missing, wrong-version, out-of-range, duplicate and extra requirements.
4. Implement indexed RA-to-staged-Rust composition using existing local holes.
   Test every operator, shared DAGs, unused inputs, empty headings, non-Copy
   values, semijoin existence work and concrete projected tuple layouts.
5. Supply a complete query artifact at every boundary and test quotation and
   partial lowering, including independent lower macro invocation.
6. Run all 25 shapes with four policies on the authored database and 64
   generated databases: CQ valuation = RA interpretation = compiled result.
   Check frozen occurrence relations against full-join projections. Compare
   GYO with all 4,680 small ordered hypergraphs over three variables.
7. Run the R1 and new contract, structural, macro, runtime and differential
   tests in debug and release, then formatting and clippy. Update the status
   documents with actual results and limits, not fixture-only success.

Stage II WCOJ/cardinality estimation is outside this implementation. Completion
does not imply projected-output-linear Yannakakis bounds or a general pure
Rust predicate analysis.

## Completion

All seven steps above are implemented and validated as of 2026-09-07. The
[verification record](STAGE-I-VERIFICATION.md#verification-commands) reports
192 passing tests in each of debug and release, 16 separate xtask tests,
warnings-denied clippy, and formatting checks. The physical implementation
materializes named results and supports at most twelve attributes per result;
its comparison fragment is primitive i32. These are explicit Stage I limits.

On 2026-09-08, [public-macro tests](../crates/mini-linq/tests/stage1_shapes.rs)
added all 25 shapes through actual recursive macro expansion, alongside the
typed-pass tests above. The new target passed 30 tests in debug and release
and warnings-denied clippy, covering 4,990 input presentations per profile.
This tests the current Auto macro route; continuation configuration remains
a separate composition change.
