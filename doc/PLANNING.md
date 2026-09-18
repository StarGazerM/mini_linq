# Planned joins through the existing compiler languages

This guide maps the implemented planned CQ route to its compiler modules and
contracts. The formal rules and algorithms are in [planning.tex](formal/planning.tex);
[the verification guide](PLANNING-VERIFICATION.md) describes the independent
oracles, reference fixtures, and reproducible checks.

## Pass boundaries and ownership

| Source | Pass | Target | Decision owned by this edge |
|---|---|---|---|
| `cq::Module` | `cq_compose::compile_cq_planned` | `relational_plan::Module` | Occurrence analysis, legal join order, semijoin reduction, selection and projection placement |
| `relational_plan::Module` | `cq_index_policy::result_demands::compile` | `index_requirements::Module` | Exact first-use keys on named right operands; preserve the entire RA |
| `index_requirements::Module` | `cq_relation_lowering::materialized_hash::compile` | staged `syn::File` with set-valued `access_to_set!` holes | Tuple representation, hash storage, existence access, materialization and DAG sharing |
| `rust_access_plan::Plan` | existing `cq_access_lowering::to_pipeline::compile` | `iterator_pipeline::Pipeline` | Explicit local physical streams and binding flow |
| `iterator_pipeline::Pipeline` | `cq_pipeline_lowering::distinct_iter::compile` / `cq_pipeline_lowering::collect_set::compile` | `syn::Expr` | Iterator / owned-set realization |

Explicit macro entries parse a source language and call its typed pass.
`choose_graph_or_prefix!` chooses caller-bound `cq_to_ra_hypergraph!` or `cq_to_ra_clause_prefix!`; each
emits RA to `choose_index_policy!`. To select eager indexed execution explicitly,
compose the following facade:

```rust
mod wiring {
    pub use ::mini_linq::*;
    pub use ::mini_linq::{
        result_index_demands as choose_index_policy,
        materialized_hash_program as choose_relation_lowering,
    };
}
use wiring as mini_linq;
```

Then invoke `mini_linq::choose_graph_or_prefix!` with a CQ. The result-index pass
emits `choose_relation_lowering!`, which this facade binds to eager hash realization.
Aliasing `materialized_hash_callable` instead selects a callable over the same
computation. With ordinary crate bindings, the index and realization choosers
select compatible implementations from their source domains and still invoke
the caller's aliases. No planner choice implies eager execution.

Generated paths resolve through those Rust imports; an alias can select a
continuation with the same contract. `materialized_hash_program!` also accepts
`access path;` for its set-valued Access holes, including `access_to_set_loops!` when only
completed answers are observed. Each intermediate artifact can be quoted directly.

The route selects implementation modules: `cq-query-lowering::hypergraph` and `cq-query-lowering::clause_prefix`
produce RA; `cq-index-policy::result_demands` derives named demands; `cq-relation-lowering::materialized_hash`
constructs indexed set computations and native hosts. `cq-relation-lowering::prefix_hash`
owns the expression-aware lazy alternative. `cq-relation-lowering::configured` accepts
configured RA directly and uses row, key, candidate and pair operations from
`cq-native-lowering`, as specified
in [Access refinement](ACCESS-REFINEMENT.md).

Each implementation owns its checked public entries and independent target
checks. `cq-compose` provides typed convenience recipes and checked report/replay
dispatch; the macro frontend composes actual continuation calls.
`cq-access-lowering` owns pull and eager push alternatives; `cq-pipeline-lowering` owns the
iterator endpoints. `cq-ir` supplies language judgments and factual heading
queries. Realization domains and index-coverage checks belong to their consumers.
[Compiler structure](COMPILER-STRUCTURE.md) records the dependency graph.

Within logical planning, `cq-query-lowering/src/dependencies.rs` derives D-* judgments;
`hypergraph/structure.rs` implements G-* reductions. `hypergraph/ya.rs` schedules
the two sweeps; `hypergraph/rules.rs` implements P-* and Y-* transitions. `clause_prefix/rules.rs`
implements C-* transitions, with A-* argument judgments composing C-MATCH.
Each implementation's `strategy.rs` chooses among its own legal candidates.
Component paths are relative to `crates/`. The
[expression rule specification](RUST-EXPRESSIONS.md#dependency-judgments-and-their-code)
pairs the judgments with code and states the graph planner's admitted domain.

The expression checker reads the actual RA separately from emission. It shares
clause dependency analysis, which is tested against authored facts and legal
orders; sharing that analysis is not an independent proof of its correctness.

Rust lexical scope remains in `cq::rust::free_variables`. Clause type
ascriptions belong to `Let::binding_types`; the basic language fragment is
recognized by `Query::is_basic`. Whole-query planning and generated CQ names
are owned by their logical implementation modules.

The RA module retains its program annotation. Lowering consumes the written
algebra; it does not reconstruct a plan from that annotation.

## Language forms used by planning

1. RA `semijoin left with right` records an existence operation, including
   composite and empty separators. Its meaning is join then left projection;
   this materialized backend implements it with an existence test.
2. CQ arguments, `if`, and `let` retain native Rust expressions; RA uses
   `select` and `extend` for their computations. A bounded comparison
   recognizer admits i32 variables/literals and six comparisons for movement.
   Every occurrence of a comparison variable must have a syntactically known
   primitive type. Arbitrary Rust calls and arithmetic preserve evaluation
   boundaries. Physical comparison lowering also anchors operands to i32.
3. `IndexRequirement` has an explicit result namespace: `@r[0, 2]` refers
   to attributes at those ordinals in the sorted heading of named result `r`.
   `R[0]` means declared input-column positions. The `@`
   prevents a result and an input with equal spellings from being confused.
   Empty keys have no index entry. Indexes carry requirements, not containers.
4. RustAccessPlan checks a logical comparison label paired with an existing
   Rust `if` access, and permits empty local headings for Unit, projection to
   `{}`, and empty-separator existence guards. Empty occurrences require `if`,
   not `for`. Pull maps these accesses to existing Pipeline operators.

## Why existing local physical IRs suffice

This indexed backend materializes each named set result. A staged Rust file
therefore has several local cursor holes, one per RA definition, separated by
ordinary Rust bindings and index builders. Each hole uses at most a scan and
a join/guard, then projection and distinct. This obeys the existing linear
IteratorPipeline contract; a shared RA result is computed once and borrowed
by subsequent regions. The RA DAG is represented by those bindings and local
regions; it is not flattened into one linear pipeline.

For `l = rename R ...; r = rename S ...; f = semijoin l with r`, the index
pass writes an index on **r**, and the physical pass emits an existence test
against that index inside f's local access plan. If r is already reduced,
the index is built from that reduced result after its definition. Projection
changes the actual stored tuple type at its own materialization boundary.

The generated API provides `build`, `query`, `materialize`, and `run`: build
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
| G-PRIVATE / G-ABSORB | `GyoRule::Private` / `Absorb`; `GyoState::remove_private` / `absorb` in [structure.rs](../crates/cq-query-lowering/src/hypergraph/structure.rs) |
| P-START | `Introduce`; `JoinState::introduce` |
| P-SELECT | `Select`; `JoinState::select` |
| P-BEGIN | `Begin`; `JoinState::begin` |
| Y-UP / Y-DOWN | `Up` / `Down`; `JoinState::up` / `down`, each with its own premises |
| Y-FREEZE | `Freeze`; `JoinState::freeze` |
| P-COMBINE / P-TRIM | `Combine` / `Trim`; `JoinState::combine` / `trim`, using `retained_heading` for K(C,P) |
| P-OUTPUT | `Output`; `JoinState::output` |
| D-ATOM / D-GUARD / D-BIND / D-MOVE / D-GRAPH | `ClauseDependencies::analyze` and `ClauseFacts::atom` in [dependencies.rs](../crates/cq-query-lowering/src/dependencies.rs) |
| C-READY | `ClauseState::require_ready`; `apply` checks normalization and matches the source clause |
| C-CONSTANT | `Constant`; `ClauseState::normalize_constant` |
| C-MATCH | `Match`; `ClauseState::match_clause`, composed from A-* in `AtomBindings::match_argument` |
| C-GUARD / C-BIND | `Guard` / `Bind`; `ClauseState::guard_clause` / `bind_clause` |
| C-FINISH | `Finish`; `ClauseState::finish` establishes the output |

The graph transitions are in [rules.rs](../crates/cq-query-lowering/src/hypergraph/rules.rs);
the prefix transitions are in [rules.rs](../crates/cq-query-lowering/src/clause_prefix/rules.rs).
The [graph strategy](../crates/cq-query-lowering/src/hypergraph/strategy.rs) selects frontier pairs;
the [prefix strategy](../crates/cq-query-lowering/src/clause_prefix/strategy.rs) selects ready clauses.
Their [graph](../crates/cq-query-lowering/src/hypergraph/check.rs) and
[prefix](../crates/cq-query-lowering/src/clause_prefix/check/mod.rs) checkers consume actual emitted
definitions and reports. Downstream passes consume
only their RA/index languages, with no dependency on graph state, policy, or reports.

The [complete CQ artifact](planned-example/01-cq.rs) expands through
[RA](planned-example/02-relational.rs), [indexes](planned-example/03-indexes.rs),
[local access plans](planned-example/04-access.rs),
[pipelines](planned-example/05-pipelines.rs), and [Rust](planned-example/06-rust.rs).
The boundary tests compare these files to actual pass output and compile each
entry separately.

## Scope and verification

The materialized backend supports at most twelve attributes per named result,
matching its Rust tuple set representation. Movable comparisons use the
recognized primitive-i32 fragment. Planning supports graph and prefix routes;
WCOJ, cardinality estimation, general Rust purity analysis, and
projected-output-linear Yannakakis bounds are outside this implementation.

[PLANNING-VERIFICATION.md](PLANNING-VERIFICATION.md) records fixture coverage,
independent semantic comparisons, mutation checks, and commands for both typed
pass and public-macro verification. The [IR boundary review](IR-STAGE-REVIEW.md)
separates this indexed route from the direct configurable physical edge.
