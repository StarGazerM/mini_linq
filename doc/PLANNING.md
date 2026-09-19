# Logical planning and its compiler boundary

The logical planners produce RA. They do not choose providers or an execution
model. The [verification guide](PLANNING-VERIFICATION.md) describes independent
logical oracles and authored plans. Post-IX compilation follows the separate
[IX/Physical design](IX-PHYSICAL-REDESIGN.md).

## Ownership and composition

| Operation | Implementation |
|---|---|
| Written-order CQ → RA | `cq_query_lowering::written_order::compile` |
| Dependency-aware clause-prefix CQ → RA | `cq_query_lowering::clause_prefix::compile` |
| Graph-planned CQ → RA, including YA | `cq_query_lowering::hypergraph::compile` |
| Typed planner selection/report/replay | `cq_query_lowering::composition` |
| RA → named index demands and uses | `cq_index_policy::compile` |
| IX plus explicit assignments → Physical | `cq_physical_selection::given_bindings` |
| Physical realization | `cq_physical_lowering::{begin,resume}` plus selected external providers/rules |

`choose_graph_or_prefix!` dispatches through caller-bound
`cq_to_ra_hypergraph!` or `cq_to_ra_clause_prefix!`. Each planner passes its RA to
`mini_linq::choose_index_policy!`; a caller can import `index_demands` under that
name. IX then goes to the caller's `choose_physical!` policy. The Physical table
records each relation/index provider and every operation's lowering separately.
There is no default whole-query hash backend.

Planning can be tested through the typed API without any physical provider.
The current borrowed slice/map implementation admits direct input Rename,
indexed NaturalJoin and terminal Project. The
[full-CQ trace](ix-physical/query-trace/review.md) follows the written-order
planner through that supported domain. Select, Extend, YA Semijoin and
nonterminal projection do not yet have migrated physical rules. The
[runnable composition example](../examples/composition/README.md) starts from
explicit RA; its [trace](ix-physical/realization-trace/review.md) is reported
separately from the full-CQ query.

The policies are Written, Connected, GreedyWidth and Auto. They use source order,
connectivity or heading arity; they do not estimate cardinalities or perform
global cost search. Auto uses GYO and a join tree for alpha-acyclic bodies,
performs YA reduction, then assembles the result; cyclic bodies use binary
planning. YA is a reduction schedule, not a claim of cost-based join ordering.

Within `cq-query-lowering`, `dependencies.rs` derives D-* judgments;
`hypergraph/structure.rs` implements G-* reductions; `hypergraph/ya.rs`
schedules the two sweeps; `hypergraph/rules.rs` implements P-* and Y-* rules.
`clause_prefix/rules.rs` implements C-* rules and the A-* argument judgments.
Each `strategy.rs` chooses among applicable commands. The
[expression specification](RUST-EXPRESSIONS.md) states dependency and movement
premises and the graph planner's admitted domain.

The checkers inspect actual emitted RA separately from emission. Expression
checking shares dependency analysis, so authored dependency facts and legal
orders test that analysis separately. Rust lexical scope remains in
`cq::rust::free_variables`; type ascriptions belong to `Let::binding_types`.
RA retains its source program annotation, but downstream passes consume the
written algebra instead of rebuilding a plan from that annotation.

## Languages used by planning

- RA Semijoin denotes a natural join followed by projection to the left heading,
  including composite and empty separators. Logical YA preserves exact relation
  versions through both sweeps; physical pruning is a separate choice.
- CQ atom arguments, `if`, and `let` preserve Rust syntax. Select and Extend
  retain the corresponding computations. The movable comparison fragment uses
  primitive-i32 operands and six comparisons; arbitrary Rust calls/arithmetic
  remain barriers.
- IX names indexes on exact RA versions: `i = @r[0,2]` uses ordinals in `r`'s
  sorted logical heading, and `j.right = i` records consumption. Empty shared
  headings still require an explicit `[]` demand. No provider is chosen by IX.
- Access associates logical occurrences with already-selected concrete Rust
  leaves. Its existing execution endpoints do not participate in logical join
  planning. Push/pull placement in the redesigned compiler remains unresolved.

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

## Scope and evidence

Logical width is heading arity, not cardinality, bytes, treewidth or fractional
hypertree width. The greedy policy makes no global optimality claim. WCOJ,
cardinality estimation, general Rust purity analysis and projected-output-linear
YA complexity are outside the implemented planning claims.

The current [planning tests](PLANNING-VERIFICATION.md) cover independent
valuation/RA equality, intermediate YA support, derivation mutation and expression
scheduling. They do not certify generated execution for expression-rich or YA
programs after removal of the old physical backends. Physical realization has
its own narrower integration tests and
[actual expansion trace](ix-physical/realization-trace/review.md).
