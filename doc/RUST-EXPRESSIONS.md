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

`cq_lang_cq::normal::normalize` turns `R(7,x)` into a fresh typed `let` for the
literal before the atoms and an atom using that bound variable. A repeated
argument becomes a fresh atom variable plus an equality guard after the atom;
`_` becomes a fresh witness variable. A computed argument becomes a binding
before its atom when its dependencies are already available, or a fresh atom
variable and an expression guard afterward when it depends on a variable
introduced earlier in the same atom. These are CQ-NF body items, not another
public expression language.

## Expression dependencies and normal form

The input is a well-formed CQ under `cq::contract`: relation arities agree,
logical uses follow their binders, and `let` introduces fresh logical names.
Lexical locals inside Rust expressions follow Rust scope. The
[Rust-expression helpers](RUST-EXPRESSION-HELPERS.md) compute free names and
pattern bindings. For an opaque expression, dependency analysis conservatively
includes every name in the available logical environment. Availability is not
a proof that an arbitrary Rust computation is pure.

| CQ input | CQ-NF representation | Where it is scheduled |
| --- | --- | --- |
| Literal argument | A fresh typed `let` and a variable argument | The literal binding precedes every atom. |
| Repeated variable | A fresh variable argument and an equality `if` | The guard follows its atom during normalization. |
| `_` | A fresh witness variable | Its atom binds the witness. |
| Computed argument | A fresh typed `let` before the atom, or a fresh variable with an `if` afterward | Its dependencies determine which representation normalization uses. |
| Source `if` or `let` | The original expression or pattern remains a body item | The valuation plan places it when its logical inputs are bound. |

`cq::normal::check` validates the normalized form: each atom argument is a
distinct variable. The [normal-form description](DSL.md#normal-form) and
[implementation](../languages/cq-lang-cq/src/normal.rs) define the kernel to which
later plans erase. Its atoms, guards, bindings and head compare up to variable
renaming.

## Valuation steps and elaboration

`cq-order` turns CQ-NF into a VO valuation plan. Each atom step names a source
clause and a subatom; a guard or binding step appears once its variables are
bound. The VO contract checks that the plan covers every atom and places its
guards and bindings. `cq-elaborate` then turns those steps into named RA.

| VO step | RA operation |
| --- | --- |
| First mention of an atom clause | Rename its input columns to the CQ-NF variables. |
| Subatom | Project the clause relation to the listed columns when needed, then join with the running prefix; a partial subatom that binds no new variable uses a semijoin. |
| `if e` | Select the running prefix. |
| `let pat = e` | Extend the running prefix with its bound values. |
| Reduction | Select or semijoin the clause relation before its atom step. |
| End of plan | Project the head variables and emit the written head. |

When a guard or binding precedes all atoms, elaboration seeds the prefix with
`unit`. A computed head retains the full heading for its final projection.
See [valuation plans](DSL.md#valuation-plans) and the
[elaboration pass](../passes/cq-elaborate/src/lib.rs) for these operators.

### Atom arguments through normal form

The atom arguments that formerly needed separate matching cases have already
become distinct variables before `cq-order` runs. The fresh binding for a
literal supplies a join key; a repeated argument's equality guard constrains
its fresh variable; a wildcard's fresh variable keeps its witness. Computed
arguments retain their Rust expression in a binding or guard, according to
which variables that expression needs.

`cq-order` can use `Split::Coarse` for one full subatom per atom (a binary join
plan) or `Split::Fine` for variable-at-a-time subatoms (generic join). The
split changes the valuation steps, while their kernel remains the CQ-NF
query's kernel.

### Order, reductions, and verification

The [order pass](../passes/cq-order/src/lib.rs) offers `Heuristic::Written`,
`Heuristic::Prefix(Ranking::{Connected, GreedyWidth, Auto})`, and
`Heuristic::Elimination`. Readiness depends on available variables; guards and
bindings retain their respective query orders. The default `cq_to_ra!` facade
normalizes, orders with Written/Coarse, and elaborates.

The [reduction pass](../passes/cq-reduce/src/lib.rs) can push a guard onto a
single atom when its lexical dependencies are visible and all belong to that
atom. It can also insert Yannakakis semijoin reductions along a GYO join tree
for an acyclic query. `cq_to_ra_hypergraph!` uses atom guard pushdown and tree
reductions when acyclic, between ordering and elaboration.

Normalization checks CQ-NF. The order and reduction passes provide
`check(source, target, options)` correspondence checks; elaboration provides
`check(source, target)`. Their targets must satisfy their language contracts;
VO reductions erase to the unreduced plan, and `relational_plan::kernel`
compares RA with the VO kernel up to variable renaming. These checks concern
the logical plan under set semantics; they do not prove purity of arbitrary
Rust expressions.

## Current verification boundary

The [CQ expression tests](../languages/cq-lang-cq/tests/rust_expressions.rs),
[free-variable tests](../languages/cq-syntax/tests/free_variables.rs), and
[expression-syntax tests](../languages/cq-lang-cq/tests/expression_syntax.rs) cover
round trips, lexical shadowing, opaque macros, Rust expression ASTs and
logical forward-reference rejection. The [valuation-language tests](../languages/cq-lang-vo/tests/valuation_language.rs)
cover normalization, plan contracts and kernel comparison.

The [order tests](../passes/cq-order/tests/order.rs),
[reduction tests](../passes/cq-reduce/tests/reduce.rs), and
[elaboration tests](../passes/cq-elaborate/tests/elaborate.rs) inspect the
planned steps and their RA. The reduction suite also compares CQ and RA
results on generated databases for its tested queries. The
[full-chain test](../facade/mini-linq/tests/physical_query.rs) exercises its
admitted physical domain, including a pushed comparison that bounds an
ordered scan and a computed column.

```sh
cargo test -p cq-lang-cq --test rust_expressions --test expression_syntax
cargo test -p cq-syntax --test free_variables
cargo test -p cq-lang-vo --test valuation_language
cargo test -p cq-order --test order
cargo test -p cq-reduce --test reduce
cargo test -p cq-elaborate --test elaborate
```

Negation and aggregation remain optional semantic work over the same Rust
expression syntax. Dependency-aware ordering does not establish purity for
arbitrary Rust computations or provide a Yannakakis reduction for cyclic
queries.
