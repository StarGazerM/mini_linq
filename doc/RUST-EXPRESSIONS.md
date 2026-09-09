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

Use `;` after a rule containing an unparenthesized final expression. The legacy
`.` remains supported, but Rust lexes `4.` as a floating-point literal; write
`if x < 4;` or `if (x < 4).`.

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

## Administrative rules in the existing passes

The CQ-to-RA pass uses these rules for general expressions. `P` denotes the
complete preceding relation, and `B` its bound attributes.

| Source operation | Relational effect | Required dependency |
| --- | --- | --- |
| Positive occurrence | Rename its columns, then natural-join with `P` | Shared attributes form equality keys |
| Prefix expression argument | `extend P` with a fresh typed key, then join | Evaluate once per prefix tuple |
| Same-occurrence expression or repeated variable | Fresh renamed attribute, then Select with equality | Referenced earlier argument is available |
| `if e` | `select P if (e)` | Known logical uses of `e` are in `B` |
| `let pat = e` | `extend P` with the pattern bindings | Uses are available; bindings are fresh |
| Head | Final Project and exact head expressions | Keep expression dependencies and full witnesses for computed heads |

Extend is an operator of the existing RelationalPlan:

```text
r1 = extend r0 with {label: _} value ({let label = value.to_uppercase(); (label,)});
```

Its value expression produces the tuple of new columns. A Unit relation seeds
a computation preceding the first atom. No extra IR or macro boundary is added.

General Rust expressions fix evaluation boundaries and retain complete prefix
headings. Connected/width policies reorder positive atoms inside each boundary;
ready total primitive comparisons run before the next join. Literal arguments
normalize to fresh typed let bindings before scanning. The Written policy
retains source clause order after this normalization. Arbitrary Rust cannot be assumed pure merely
because its visible variables are bound. Macro opacity also prevents dropping
columns that an expression might use. Computed heads run before final result
deduplication; their distinct input witnesses must survive projection.

The independent source-to-RA contract consumes the actual definitions and
checks normalization, exact Rust expressions, bindings, joins, final projection,
and absence of extra operations. Derivation checks additionally require exactly
a Constant normalization step per literal argument, one Match, Guard, or Bind
rule per clause, and a final Finish rule. Legal alternative schedules can be
replayed; crossing an effect boundary or using an unavailable input is rejected. Lower passes
inspect their own RA/index input; they do not reconstruct planning from CQ.

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

In the homework starter, parser and supplied-lowering tests run by default.
The expression execution fixtures use the existing `compiled-workloads`
completion gate, alongside the other tests requiring student pass solutions.

The production planning acceptance matrix additionally covers 12 expression
patterns under four policies. It checks expected relation orders, one fully
handwritten RA plan, actual compiled results on six datasets, and the numbers
of key, guard, and head evaluations against independent source-order Rust
loops. A literal-key fixture must also enumerate fewer join pairs than its
written Cartesian prefix. This is dependency-aware binary planning; it does
not claim a Yannakakis certificate for queries containing general Rust.
