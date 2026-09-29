# Supplied Rust syntax and free-variable analysis

The parser and lexical helper are provided infrastructure. Students implement
the semantic contracts and compiler passes assigned to their checkpoint.
Parsing an expression does not establish that it is well scoped, pure, safe to
reorder, or supported by a particular lowering.

## Reading the syntax tree

Every expression leaf is a `syn::Expr`; patterns and types use `syn::Pat` and
`syn::Type`. Parsing and printing derive from the syntax declarations. Query
terminators are `;`. There is no token splitting or period compatibility path.

| Source position | Rust AST field |
|---|---|
| `R(expression)` or `answer(expression)` | `Atom.args` |
| `if expression` | `Filter.condition` |
| `let pattern: Type = expression` | `Let.pattern`, optional `Let.annotation`, `Let.expression` |
| `!R(expression)` | `Negation.atom.args` |
| `agg pattern = (expression)(value) in R(expression)` | `Aggregate.pattern`, `Aggregator::Expression.expression`, `Aggregate.atom.args` |

An aggregator can also be a path, as in `sum(value)` or `stats::<i32>(value)`.
The `value` list declares aggregate-local names; its elements are identifiers,
not Rust call arguments. General Rust expressions belong in the parenthesized
aggregator and the input atom. This follows Ascent's
[aggregate grammar](https://github.com/s-arash/ascent/blob/cf5e9a87525bb95268cf6680a59882264b0fe0de/ascent_macro/src/ascent_syntax.rs#L386-L420).

For example, all of this parses without implementing any homework pass:

```text
struct Inspect;
relation R(value: i32);
relation S(key: i32, value: i32);
answer(format!("{x}")) :-
    R(x),
    if { let minimum = limit; *x > minimum },
    let (key, label): (i32, String) = make::<i32, String>(x),
    S(key + offset, value),
    agg total = (factory(config, |a, b| a + b))(amount)
        in S(key + offset, amount);
```

The basic fragment of positive atoms with distinct bare variables is a useful
restricted subset. The default `cq_to_ra!` normalizes Rust argument
expressions, guards and lets, orders Written/Coarse and elaborates. Negation
and aggregation parse here but fail the baseline CQ contract. The opt-in
`hw4` tests specify that later extension; supplying syntax does not make
those semantic tests pass.

## Collecting free variables

Use `cq_syntax::rust::free_variables(&expression)`. It returns:

```rust
pub struct FreeVariables {
    pub names: BTreeSet<String>,
    pub opaque: bool,
}
```

`names` contains free simple value names visible in the Rust AST, with raw
identifiers normalized (`r#x` and `x` have the same name). The set is sorted and
deduplicated. The helper does not filter names against a CQ environment:
`accept(x)` reports both `accept` and `x`. Rust resolves which names are host
functions, constants, or values. Member names in `row.field` and method names
in `row.method(x)` are not free variables; their receivers and arguments are.
Qualified path components are not reported as variables, but expression-valued
const generic arguments are visited.

```rust
use std::collections::BTreeSet;
use cq_syntax::rust::free_variables;

let expression: syn::Expr = syn::parse_quote!({
    let x = y;
    (|y| x + y)(z)
});
let free = free_variables(&expression);
assert_eq!(free.names, ["y", "z"].map(String::from).into());
assert!(!free.opaque);

let logical: BTreeSet<_> = ["x", "y", "z"].map(String::from).into();
assert_eq!(free.dependencies(&logical), ["y", "z"].map(String::from).into());
```

The visitor implements lexical binding scopes:

| Rust construct | Scope treatment |
|---|---|
| `let x = expression` | Inspect the initializer before binding `x` for later statements |
| `let Some(x) = expression else { ... }` | The `else` expression sees the preceding scope |
| Closure parameters | Bind only inside the closure |
| `for pattern in expression` | Inspect the source first; bind the pattern inside the loop |
| Match arms | Each arm has its own pattern bindings, guard, and body |
| `if let` / `while let`, including `&&` chains | Bind for later conditions and the successful body; not the `else` branch or following code |
| Nested block | Restore the enclosing scope on exit |

## Opaque Rust and conservative dependencies

Rust macros have their own grammars and may hide uses in strings or introduce
bindings. `format!("{x}")` therefore reports `opaque = true`; an empty `names`
set is not evidence that it uses no variables. Local items, attributes, and
verbatim AST nodes are also marked opaque. Under that flag, `names` is only a
syntactic observation and must not be treated as an exact free-variable set.
The helper does not expand macros or implement Rust name/type resolution.

`free.dependencies(&environment)` returns the intersection with `names` for
ordinary syntax, or the entire supplied environment for opaque syntax. Pass the
complete logical environment available at that expression's source position
when computing liveness. For diagnostics, do not diagnose a hidden macro use as
absent or assume a syntactically visible name cannot have been bound by a macro.
Rust's compiler still checks the residual expression.

```rust
let expression: syn::Expr = syn::parse_quote!(format!("{x}"));
let free = free_variables(&expression);
assert!(free.opaque);
assert_eq!(free.dependencies(&logical), logical);
```

## Bindings and clause dependencies are different questions

`cq_syntax::rust::bindings(&pattern)` supplies the syntactic names declared by
a Rust pattern, in source order. Rust still checks refutability, types, and
ambiguous constant patterns. Use this helper for a `let` or aggregate result
pattern. A result binding is not in scope while its initializer is evaluated.

`Atom::variables()` returns only bare logical-variable arguments. It skips
computations, so it is **not** a substitute for `free_variables` on `Atom.args`.

Students still determine CQ scopes: which positive terms bind, which names are
aggregate-local, which uses refer to the outer environment, and when a clause's
outputs become available. A free-variable set records dependencies; it does not
prove that a Rust call has no effects or that an aggregate may change order.
No join ordering, normalization, relational lowering, or execution algorithm
is implemented by this helper.

## Run the examples and checks

```text
cargo run --example free-variables
cargo test -p cq-lang-cq --test expression_syntax --test free_variables_execution
cargo test -p cq-syntax --test free_variables
```

The first tests compare known free-variable sets and structural syntax
roundtrips. The execution test generates Rust functions with exactly the
reported logical inputs, compiles them with `rustc`, and checks their results.
It validates the supplied helper independently of any compiler pass. This
branch's expression and planner endpoint tests additionally exercise lowering
and execution.
