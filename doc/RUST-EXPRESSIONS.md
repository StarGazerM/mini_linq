# Rust expressions in CQ

The [runnable gallery](../examples/rust-expressions/README.md) includes 24
queries with inputs and asserted outputs, a lazy cursor walkthrough, and six
join schedules. It covers each implemented feature below. The separate
[negation/aggregation examples](../examples/negation-aggregation/README.md)
specify the optional extension, which remains unimplemented.

CQ stores atom arguments and `if` conditions as `syn::Expr`, and stores `let`
bindings as `syn::Pat` plus `syn::Expr`. Parsing uses Rust's grammar, including
calls, methods, blocks, closures, match expressions, macros, and generic
arguments. The comparison recognizer is an optimization premise, not a grammar.

```rust
mini_linq! {
    struct Labels;
    relation R(key: i32, value: String);
    relation answer(label: String);
    answer(format!("{key}:{label}")) :-
        R(key, value),
        let label = value.to_uppercase(),
        if *key > 0 && !label.is_empty();
}
```

Every rule ends with `;`, including rules with only positive atoms. A period
is not a rule terminator. Rust expressions are parsed directly by Syn.

Bare arguments of a positive atom introduce variables; already-bound variables
are equality constraints. Repeated arguments such as `R(x, x)` constrain two
columns to the same value. `_` matches a column without naming it. Computed
arguments are Rust expressions: `R(x), S(f(x))` matches the value of `f(x)`;
`R(x, f(x))` checks the second column using the first column's binding.
Atom bindings borrow input fields. Let bindings carry their Rust values;
ordinary Rust borrowing, ownership, and trait checks still apply.

`let PAT = EXPR` introduces fresh pattern bindings; it cannot shadow an existing
logical variable. Lexical locals inside expressions can shadow them normally.
Known logical variables must be bound before use. Host functions, constants,
paths, and types remain Rust's responsibility. Macros are opaque to this pass:
liveness conservatively retains the complete available environment, and their
original tokens and syntax contexts reach rustc.

A head containing computed values needs a declaration of its result column
types, as above. That declaration is excluded from `build`/`run` input arguments.
Bare head variables obtain their types from input columns or a simple explicitly
typed binding such as `let label: String = ...`. The macro does not infer Rust
function return types. Intermediate inferred types stay local; public result
types must be nameable. Stored rows and computed columns used in materialized
sets need `Clone`, `Eq`, and `Hash`; sorted results additionally need `Ord`.

A literal argument has the normalization equation
`R(7, x) => let k: ColumnType = 7, R(k, x)`, with a fresh `k`.
The current implementation records this as an explicit Constant rule and uses
exactly the same binding lowering as a typed source let. This normalization
currently lives inside CQ-to-RA; it is not a separate nominal IR/macro boundary.
General `f(x)` arguments cannot be hoisted like literals.

## Dependency judgments and their code

The input is a well-formed CQ under `cq::contract`: relation arities agree,
logical uses follow their binders, and `let` introduces fresh logical names.
Lexical locals inside Rust expressions follow Rust scope. Define
`uses(e, B) = free_variables(e).dependencies(B)`; for opaque syntax this is
all of `B`. This is an availability judgment, not a purity judgment.

[dependencies.rs](../crates/cq-compiler/src/cq_to_relational/dependencies.rs)
derives `ClauseFacts = (required, provided, reorderable, atom_arity,
literal_arguments)`. `provided` includes every bare atom name, even an equality
key already supplied by another atom; it does not mean every name is fresh.
`ClauseDependencies::analyze` processes the written prefix `B` and records
which names were supplied by preceding lets (`T`). The clauses remain CQ;
these facts have no separate syntax, quotation entry, or lowering boundary.

| Judgment | Premises and conclusion | Code |
| --- | --- | --- |
| D-ATOM | Walk arguments left to right with earlier bare names `L`. A bare name joins `L`; a literal records its position; `_` adds no logical name. A computation adds `uses(e, B ∪ L) − L` to `required`. Bare names in `T` are also required: an atom cannot produce a let's value. `provided = L`, and arity counts every positional column. | `ClauseFacts::atom`, then the let-origin dependency in `analyze` |
| D-GUARD | `required = uses(e,B)`, `provided = ∅`. A recognized comparison's movement premise is deferred until all occurrence types are known. | Filter arm of `analyze` |
| D-BIND | `required = uses(e,B)`, `provided = bindings(pattern)`. Extend `B` and `T` with those names and record their written types. | Let arm of `analyze` |
| D-MOVE | Atoms containing only bare names, literals, and `_` are movable. A let is movable only for a simple identifier pattern and a literal expression. A guard is movable only for the supported primitive comparison, with every occurrence of each operand name known to be primitive i32. All other Rust computations are barriers. | Atom/Let arms and final comparison loop in `analyze` |
| D-GRAPH | The head has distinct bare names, each atom has distinct bare arguments, there are no lets, and all guards satisfy D-MOVE. This selects the existing graph-rule domain; GYO separately decides acyclicity. | `supports_graph_rules` in `analyze` |

D-MOVE is the current conservative optimizer contract. Free-variable analysis
does not prove arbitrary Rust code pure. D-GRAPH also remains a whole-query
restriction: queries outside it use the prefix rules below. This implementation
does not yet apply GYO to pure regions separated by computations, and it does
not supply lowering rules for negation or aggregation.

## Prefix state and administrative transitions

[clause_rules.rs](../crates/cq-compiler/src/cq_to_relational/clause_rules.rs)
implements `ClauseState = (D,P,B,U,K,O)`:

- `D` is the sequence of immutable RA definitions.
- `P` is the current prefix relation; before one is needed it is implicit Unit.
- `B` is its heading, including generated keys and anonymous witness columns.
- `U` is the set of pending source clause indices.
- `K` maps normalized literal positions to fresh typed keys.
- `O` is absent until the unique output transition.

Initially `D,B,K` are empty, `P,O` are absent, and `U` contains every clause.
At completed clause transitions, `P` denotes the executed prefix and retains
its witness columns. This prevents premature set projection from changing the
number of evaluations of later Rust expressions. A-* below is an internal
judgment within C-MATCH; it does not expose an incomplete atom as a clause state.

**C-READY.** Clause `i` is applicable when normalization is complete, output is
absent, `i ∈ U`, and `required_i ⊆ B`. A Bind also requires `provided_i ∩ B = ∅`.
For every earlier pending `j < i`, both `i` and `j` must be reorderable. Thus
clauses can exchange positions inside a movable region, while neither side
can cross a pending barrier. `require_ready` checks the clause premises;
`apply` also checks normalization and matches the command against its source clause;
`apply` rejects every transition after output. Rejection leaves state unchanged.

| Transition | State change and relational operation | Code |
| --- | --- | --- |
| C-CONSTANT | Require the next literal in source/argument order and no executed clauses. Choose fresh `k`, perform `extend P` using `let k: ColumnType = literal`, and add its position to `K`. | `normalize_constant`, using `extend_binding` |
| C-MATCH | Require C-READY for a positive atom. Use A-* for each positional argument, emit Rename, natural-join with `P` when present, and apply accumulated equalities. Remove this clause from `U`. | `match_clause` |
| C-GUARD | Require C-READY for `if e`. Replace `P` by `select P if e`; remove this clause from `U`. | `guard_clause` |
| C-BIND | Require C-READY for `let pat = e`. Replace `P` by Extend evaluating the written binding and returning its bound names; extend `B`, then remove the clause from `U`. | `bind_clause`, using `extend_binding` |
| C-FINISH | Require complete normalization and `U = ∅`. For a bare head retain its dependencies; for a computed head retain all witnesses. Emit Project, update `P,B`, and set `O` to the exact written head. | `finish` |

`into_module` packages a finished state into the existing RA module. It cannot
supply an omitted C-FINISH. `plan` chooses and applies rules; `replay` directly
applies a supplied derivation. Both use the same transitions. The complete RA
is checked against `max_arity` before return; this is a bound on every named
relation, not a claim about cardinality or fractional width.

### Argument judgments composing C-MATCH

`AtomBindings::match_argument` has the following cases in this order. Its
context holds the prefix heading `B₀`, earlier bare arguments `L`, and pending
equalities `E`. Generated column names are fresh with respect to source names.

| Judgment | Premise | Effect |
| --- | --- | --- |
| A-CONSTANT | This position is in `K` | Rename the column to that existing typed key. |
| A-VARIABLE | Bare `x` has not occurred earlier in this atom | Add `x` to `L`; rename to `x`, sharing any prefix join key. |
| A-REPEAT | Bare `x` is already in `L` | Rename to fresh `k`; append `k == x` to `E`. |
| A-WILDCARD | Argument is `_` | Rename to fresh `k` and keep its witness column. |
| A-PREFIX | Computation `e` has no uses in `L − B₀` | Extend the prefix with fresh typed `k = e` before the atom's join; rename the input column to `k`. |
| A-LOCAL | Computation `e` uses a name in `L − B₀` | Rename to fresh `k`; append `k.eq(&(e))` to `E` for evaluation after the atom's join. |

C-MATCH is a composed rule with these explicit subjudgments; the diagnostic
trace still records one Match per atom. Its local equalities are applied in
argument order. C-CONSTANT and C-BIND share binding construction; general
computations use A-PREFIX or A-LOCAL and cannot be hoisted as literals.

### Policy and verification

[strategy::choose_clause](../crates/cq-compiler/src/cq_to_relational/strategy.rs)
receives only C-READY candidates. Written chooses the first clause; other
policies prefer ready guards/bindings, then connected atoms, then optional
width ranking. The policy cannot emit RA or make an inapplicable rule legal.

The [source-to-RA checker](../crates/cq-compiler/src/pass_contracts/ordered.rs)
reads actual definitions and independently checks their operators, Rust
expressions, bindings, equality constraints, projection, and output. Its
schedule check shares dependency facts with the planner, so that part is not
an independent validation of D-ATOM/D-MOVE. Authored dependency fixtures and
all 120 permutations of a five-clause query test those premises separately.
Lower passes inspect their own IR inputs and do not reconstruct this schedule.

The default execution route keeps one lazy pull region: `build()` prepares rows
and indexes; clauses execute when the cursor is advanced. The explicit
materialized route evaluates each named RA result during `build()`. Both still
lower through RustAccessPlan and IteratorPipeline. Negation and aggregation
remain the separate optional exercise; they must reuse this expression grammar.

## Verification

Parser tests cover round trips, Rust AST forms, lexical shadowing, opaque macros,
and invalid logical forward references. Contract mutation tests remove rules,
change expressions, reorder guards, and prune required witnesses. Executable
tests cover computed arguments and heads, repeated String arguments, wildcards,
destructuring lets, Rust macro wrappers, and evaluation counts. The default
route also checks that build and cursor construction execute no query clauses.

Parser and lowering tests run by default. The expression execution fixtures
use the `compiled-workloads` feature to compile the complete public macro route.

The production planning acceptance matrix additionally covers 12 expression
patterns under four policies. It checks expected relation orders, one fully
handwritten RA plan, actual compiled results on six datasets, and the numbers
of key, guard, and head evaluations against independent source-order Rust
loops. A literal-key fixture must also enumerate fewer join pairs than its
written Cartesian prefix. This is dependency-aware binary planning; it does
not claim a Yannakakis certificate for queries containing general Rust.
