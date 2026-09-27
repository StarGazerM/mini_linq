# Rust expressions in CQ and RA planning

CQ stores atom arguments and `if` conditions as `syn::Expr`, and `let` bindings
as `syn::Pat` plus `syn::Expr`. Syn handles calls, methods, blocks, closures,
match expressions, macros and generic arguments. A comparison recognizer is
an optimization premise, not the expression grammar.

This is a CQ syntax example accepted for logical planning, not a currently
supported end-to-end physical query:

```text
struct Labels;
relation R(key: i32, value: String);
relation answer(label: String);
answer(format!("{key}:{label}")) :-
    R(key, value),
    let label = value.to_uppercase(),
    if *key > 0 && !label.is_empty();
```

Every rule ends with `;`. Bare positive arguments introduce variables;
already-bound arguments and repeats such as `R(x,x)` impose equality. `_`
retains an anonymous witness. `R(x), S(f(x))` matches against a computed value;
`R(x,f(x))` can depend on an earlier field in the same occurrence.

`let PAT = EXPR` introduces fresh logical names. Lexical locals inside the Rust
expression can shadow normally; logical names must be available before use.
Host functions, types and constants remain Rust's responsibility. Macros are
opaque to dependency analysis, which conservatively retains all available
logical names and preserves their original tokens/syntax contexts.

Computed heads require a result schema declaration; it is not an input.
Bare head types come from input columns or explicitly typed bindings. The
compiler does not infer Rust function return types. The chosen realization
must establish actual field borrowing, ownership and collection requirements;
logical planning does not grant ownership or insert a universal Clone bound.

Literal normalization is `R(7,x) → let k: ColumnType = 7, R(k,x)` for a fresh
`k`. C-CONSTANT uses the same binding construction as a typed source `let`.
This occurs within CQ → RA, not through another public IR. General `f(x)`
arguments cannot be hoisted under the literal rule.

## Dependency judgments and their code

The input is a well-formed CQ under `cq::contract`: relation arities agree,
logical uses follow their binders, and `let` introduces fresh logical names.
Lexical locals inside Rust expressions follow Rust scope. Define
`uses(e, B) = free_variables(e).dependencies(B)`; for opaque syntax this is
all of `B`. This is an availability judgment, not a purity judgment.

[dependencies.rs](../crates/cq-query-lowering/src/dependencies.rs)
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

[rules.rs](../crates/cq-query-lowering/src/clause_prefix/rules.rs)
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

[prefix clause choice](../crates/cq-query-lowering/src/clause_prefix/strategy.rs)
receives only C-READY candidates. Written chooses the first clause; other
policies prefer ready guards/bindings, then connected atoms, then optional
width ranking. The policy cannot emit RA or make an inapplicable rule legal.

The [source-to-RA checker](../crates/cq-query-lowering/src/clause_prefix/check/clauses.rs)
reads actual definitions and independently checks their operators, Rust
expressions, bindings, equality constraints, projection, and output. Its
schedule check shares dependency facts with the planner, so that part is not
an independent validation of D-ATOM/D-MOVE. Authored dependency fixtures and
all 120 permutations of a five-clause query test those premises separately.
Lower passes inspect their own IR inputs and do not reconstruct this schedule.

## Current verification boundary

The parser/helper tests cover round trips, lexical shadowing, opaque macros,
Rust expression ASTs and logical forward-reference rejection. The moved planning
suites are:

- [expression_planning.rs](../crates/cq-query-lowering/tests/expression_planning.rs):
  twelve authored expression shapes under four policies, expected join orders,
  one handwritten RA plan, all 120 clause permutations of the dependency example,
  literal normalization and width limits.
- [rust_expression_contracts.rs](../crates/cq-query-lowering/tests/rust_expression_contracts.rs):
  mutated expressions/equalities, guard ordering, retained witnesses and
  derivation re-entry.
- [composition.rs](../crates/cq-query-lowering/tests/composition.rs):
  selected rule families and rejection of unsupported domains.

These inspect RA and derivations. The old compiled-expression gallery,
materialized/lazy host APIs, runtime evaluation-count matrix and
`compiled-workloads` feature are removed; they are not verification of the
current physical path. The physical chain has its own
[full-chain test](../crates/mini-linq/tests/physical_query.rs) for its
admitted domain, including pushed comparisons that bound an ordered scan and
a computed column; the broader expression-rich and YA planning discussed
here is verified logically, not by compiled execution.

```sh
cargo test -p cq-query-lowering --all-features \
  --test expression_planning --test rust_expression_contracts --test composition
cargo test -p cq-ir --test rust_expressions --test free_variables --test expression_syntax
```

Negation and aggregation remain optional semantic work over the same Rust
expression syntax. Dependency-aware binary planning does not supply a YA
certificate for arbitrary Rust computations or establish their purity.
