# Pick-one extension: Ascent-style negation and aggregation

This is one option in the required advanced-extension menu after Basic
Homeworks 1–3. A team works on it only when assigned the released
`pick-one/negation-aggregation` branch; the normal common-core build and grading
commands do not run it. Start from the released reference branch, not from
unfinished basic pass bodies. The extension is harder because you
extend the source language contract, the named relational
algebra fragment, the index-requirement relation, and the physical Rust lowering.
Its relationship to push, Rayon, multi-query, and arbitrary-type options is
recorded in the
[extension compatibility matrix](OPTIONAL-PROJECT-TRACKS.md#route-graph-and-configuration-rule).
Comments containing `OPTIONAL HW4` mark every production edit site:

```text
rg -n "OPTIONAL HW4" crates/*/src
```

The existing `cq::BodyItem` used by `cq::Query` already supplies the clause
syntax; there is no second query parser or second query macro. This is a
cumulative checkpoint: students extend the contracts and the same three typed
passes, and completed extended queries continue to enter through
`mini_linq!`. The legacy-named `hw4` Cargo feature gates only this extension's test targets; it
does not install production syntax, swap a pass implementation, or select a
backend. Syntax for the original first implementation site is now supplied;
the remaining numbered sites mark semantic work. The supplied
[Rust-expression helpers](RUST-EXPRESSION-HELPERS.md) expose lexical free names
and pattern bindings without implementing CQ scope, planning, or lowering.

## Why this syntax

Use Ascent's body-clause forms, rather than inventing SQL-like syntax:

```text
!Blocked(person, _)
agg total = sum(amount) in Purchase(person, _, amount)
```

In Ascent, negation is syntactic sugar for an aggregate using `not()`. An
aggregate clause correlates its input relation with variables already bound by
earlier clauses, passes selected relation columns to an aggregator, and binds
the aggregator's output pattern. The relevant implementation is in Ascent's
[`BodyItemNode`, `NegationClauseNode`, and `AggClauseNode`](https://github.com/s-arash/ascent/blob/cf5e9a87525bb95268cf6680a59882264b0fe0de/ascent_macro/src/ascent_syntax.rs), its
[`!R(...)` desugaring](https://github.com/s-arash/ascent/blob/cf5e9a87525bb95268cf6680a59882264b0fe0de/ascent_macro/src/ascent_syntax.rs#L989-L998), and its
[`sum` and `not` aggregators](https://github.com/s-arash/ascent/blob/cf5e9a87525bb95268cf6680a59882264b0fe0de/ascent/src/aggregators.rs).

This homework implements a deliberately narrow, Ascent-shaped fixed-`i32`
subset. It keeps MiniLinq's `:-` rule separator and fixed `i32` columns, but
uses Ascent's negation and aggregation clause syntax and evaluation model. For
sums that fit in `i32`, its results agree with that subset; MiniLinq
additionally requires checked overflow so debug and release execution cannot
disagree. The course keeps the existing `cq` module so the exercise is a
focused IR extension rather than duplicate parser plumbing. In its internal
Rust API names, read “CQ” as historical compatibility, not as a claim that
negation is a positive conjunctive query.

## Required example

```text
pub struct SpendProgram;
relation Person(c0: i32);
relation Blocked(c0: i32, c1: i32);
relation Purchase(c0: i32, c1: i32, c2: i32);
spend(person, total) :-
    Person(person),
    !Blocked(person, _),
    agg total = sum(amount) in Purchase(person, _, amount);
```

For set-valued inputs

```text
Person   = {(1,), (2,), (3,)}
Blocked  = {(2, 7,)}
Purchase = {(1, 10, 5,), (1, 20, 5,), (1, 30, 5,), (2, 30, 99,)}
```

the result is `{(1, 15,), (3, 0,)}`. Ascent's `sum` produces one zero for an
empty matching input, so person 3 remains. Input duplicates are removed when
`inputN` is normalized to `relationN`; different relation tuples that carry
the same aggregate value still contribute separately. Thus the three distinct
person-1 tuples each contribute `5`. The second columns of both `Blocked` and
`Purchase` are intentionally ignored: this fixture makes `_` part of the
required surface and proves that wildcard and aggregate-value columns do not
become index-key columns.

### Global-clause example

HW4 must also support a safe negation or aggregation before any positive atom
has bound a variable:

```text
pub struct GlobalSummaryProgram;
relation Blocked(c0: i32);
relation Purchase(c0: i32, c1: i32);
summary(total) :-
    !Blocked(_),
    agg total = sum(amount) in Purchase(_, amount);
```

Both clauses have an empty correlation environment. If `Blocked` is empty and
`Purchase = {(10, 5,), (20, 7,)}`, the result is `{(12,)}`. If `Blocked` is
nonempty, the result is empty. If both inputs are empty, the result is `{(0,)}`.
This example requires the staff-supplied logical `Unit` source described
below; students do not invent a dummy positive relation.

## Supplied surface grammar

The existing parser accepts:

```text
Query          ::= Atom ":-" BodyItem ("," BodyItem)* ";"
BodyItem       ::= Atom | "if" RustExpr
                 | "let" RustPat [":" RustType] "=" RustExpr
                 | Negation | Aggregate
Atom           ::= Ident "(" [RustExpr ("," RustExpr)* [","]] ")"
Negation       ::= "!" Atom
Aggregate      ::= "agg" RustPat "=" Aggregator
                   "(" [Ident ("," Ident)* [","]] ")" "in" Atom
Aggregator     ::= RustPath | "(" RustExpr ")"
```

This homework's required semantic subset uses the simple path `sum` with
exactly one aggregate-local binder, a fresh bare output variable, and input
arguments that are bare variables or `_`. The supplied syntax is broader:
the parenthesized aggregator and all atom arguments retain native `syn::Expr`
values. Supporting that broader syntax in execution is a separate extension.

`agg` becomes a body-clause keyword. A relation whose Rust identifier is
literally `agg` must therefore use the raw spelling `r#agg`, consistent with
the existing raw-identifier policy.

`Negation` and `Aggregate` are nominal payloads in the existing `BodyItem`.
All syntax objects derive `Parse` and `ToTokens`; native Syn parses Rust
leaves. Reuse `Atom`, `cq::rust::variable`, and `cq::rust::bindings` when
checking the required subset. For expression extensions, use
`free_variables(expr).dependencies(environment)` to retain logical inputs,
including conservative dependencies of opaque macros. The caller must still
establish the correct CQ environment and aggregate-local scope.

## Local language contract

Let `B` be the set of variables bound before a body item.

### Positive atom

The existing rule remains: check the declared relation and arity, then extend
`B` with the atom's variables. Existing variables are equality-join inputs;
new variables are outputs.

### Negation

For `!R(t0, ..., tn)`:

- `R` must be a declared input with the written arity;
- every named term must already belong to `B`;
- `_` is an anonymous wildcard;
- the clause binds no variable, so the outgoing environment is still `B`.

This is safe, correlated negation. A later positive clause cannot retroactively
bind a variable used by an earlier negation.

### Aggregation

For `agg out = sum(value) in R(t0, ..., tn)`:

- `R` must be a declared input with the written arity;
- `out` must be a fresh variable;
- `value` is a fresh local aggregate variable and must occur exactly once in
  the pattern atom;
- `out` and `value` must be different variables;
- every other named pattern term must already belong to `B`;
- `_` is an anonymous wildcard;
- `value` does not escape the aggregate clause;
- only `out` is added to the outgoing environment.

The required subset accepts exactly one argument for the one-segment path
`sum`. Have the explicit `cq::contract::check` diagnostic API reject
shadowing, an already-bound aggregate value, a repeated or absent aggregate
value, `out == value`, an unbound correlated variable, a missing aggregate
argument, and a non-`sum` aggregator with a span-aware `syn::Error`.
Head variables must be bound after the final body item, as before.

MiniLinq currently has one nonrecursive rule whose body reads only declared
input relations. Therefore every negative or aggregate dependency is on a
fixed extensional relation and stratification is automatic. Do **not** add an
SCC/stratification framework to this homework. If a future language permits
derived relations in these clauses, it must then add Ascent's dependency-graph
check and reject cycles through negation or aggregation.

## RelationalPlan extension

Do not lower the extended CQ directly to indexes. Basic HW1 remains a real
logical boundary. RelationalPlan already supplies `Unit`, the singleton
zero-column relation used to seed a global clause. Extend its operator sum with
`AntiSemijoin` and `AggregateApply`, then emit a named `rN` result for every
source occurrence and operator. Do not redesign or reimplement `Unit`.

The correlated required example is:

```text
r0 = rename Person {c0 -> person};
r1 = rename Blocked {c0 -> person, c1 -> blocked_anon0};
r2 = antisemijoin r0 with r1;
r3 = rename Purchase {c0 -> person, c1 -> purchase_anon0, c2 -> amount};
r4 = aggregate_apply r2 with r3 value amount using sum into total;
r5 = project r4 keep {person, total};
output r5 as spend(person, total);
```

The complete logical residual for the global-clause example is exactly:

```text
pub struct GlobalSummaryProgram;
relation Blocked(c0: i32);
relation Purchase(c0: i32, c1: i32);
summary(total) :-
    !Blocked(_),
    agg total = sum(amount) in Purchase(_, amount);
relational {
    r0 = unit;
    r1 = rename Blocked {c0 -> blocked_anon0};
    r2 = antisemijoin r0 with r1;
    r3 = rename Purchase {c0 -> purchase_anon0, c1 -> amount};
    r4 = aggregate_apply r2 with r3 value amount using sum into total;
    r5 = project r4 keep {total};
    output r5 as summary(total);
}
```

`Unit` denotes `{()}`: one empty binding, not an empty relation. It has no
operand and its exact definition syntax is `r0 = unit;`. When the first
extended body item needs a left input and no positive prefix exists, emit one
`Unit` and make it current. Reuse that current logical relation for later
global clauses; do not emit one unit per clause. A positive-first query never
needs `Unit`.

As with NaturalJoin, correlation is inferred from the intersection of named
headings. An empty intersection is semantically significant. The global
AntiSemijoin keeps `{()}` exactly when the declared `Blocked` input is empty.
If that binding survives, AggregateApply groups the entire `Purchase` input,
uses `amount` as its value, and appends the owned `total`; an empty input still
produces the one sum value `0`.

`AntiSemijoin` preserves its left heading and removes a left row when the right
input has a row with the same inferred correlation attributes.
`AggregateApply` evaluates one correlated right group for each left row,
applies the named aggregator to its value attribute, and appends the owned
aggregate result. This explicit apply operator is
important: `sum` over an empty matching group yields one zero, so person 3 is
not lost as it would be by an ordinary inner join with a grouped relation.

The compiler-generated `*_anonN` target attributes above represent wildcard
source columns. They are not user variables and cannot be referenced by a
later CQ clause, but each complete Rename must retain them until aggregation.
Otherwise set projection would collapse the three distinct Purchase tuples
carrying value `5` into one `(person, amount)` row and produce the wrong sum.
Generated anonymous names must be collision-free within the plan. The
CQ-to-plan edge maps each source `_` to its generated anonymous target in the
Rename for that occurrence; no logical Scan operator is added.

The extended RelationalPlan checker keeps the baseline local DAG rules: every
result ID is unique, every operand is earlier, and inferred headings are exact. It also checks that AntiSemijoin preserves the left heading and that
AggregateApply produces `left heading + fresh output`, consumes its value
exactly once, and does not expose a right-local or anonymous attribute. The
cross-stage HW4 contract separately checks that this complete plan is the
exact expansion of the extended CQ; local well-formedness alone is not enough.

`Unit` and an AntiSemijoin over its empty heading naturally have an empty
heading. AggregateApply then appends its fresh output, so its global result has
the heading `{total}`.

## IndexRequirements extension

Retain the existing IndexRequirements target and preserve the complete
extended RelationalPlan inside it. Basic HW2 now derives keys from the named
operators rather than repeating a direct CQ-body walk:

- a `NaturalJoin` uses the baseline heading-intersection and Rename-inversion
  rule;
- an `AntiSemijoin` requires all inferred correlated non-wildcard source
  columns from its right Rename;
- an `AggregateApply` requires its inferred correlated source columns,
  excluding its aggregate value and anonymous wildcard attributes;
- add only nonempty keys, deduplicated by `(relation, columns)` in first-use
  plan order.

The baseline Basic-HW2 pass intentionally rejects any plan containing `Unit`.
The HW4 extension admits the supplied Unit source and continues the logical
walk through AntiSemijoin and AggregateApply. For the global example, both
heading intersections are empty, so neither clause creates an index
requirement. An empty key means “enumerate/test the whole relation”; it is not
written as an empty `R[]` index.

The complete IndexRequirements target therefore retains both the source and
logical plan and then appends the two keys:

```text
pub struct SpendProgram;
relation Person(c0: i32);
relation Blocked(c0: i32, c1: i32);
relation Purchase(c0: i32, c1: i32, c2: i32);
spend(person, total) :-
    Person(person),
    !Blocked(person, _),
    agg total = sum(amount) in Purchase(person, _, amount);
relational {
    r0 = rename Person {c0 -> person};
    r1 = rename Blocked {c0 -> person, c1 -> blocked_anon0};
    r2 = antisemijoin r0 with r1;
    r3 = rename Purchase {c0 -> person, c1 -> purchase_anon0, c2 -> amount};
    r4 = aggregate_apply r2 with r3 value amount using sum into total;
    r5 = project r4 keep {person, total};
    output r5 as spend(person, total);
}
indexes {
    Blocked[0];
    Purchase[0];
}
```

The complete index overlay for the global example preserves its entire plan
and has no entries:

```text
pub struct GlobalSummaryProgram;
relation Blocked(c0: i32);
relation Purchase(c0: i32, c1: i32);
summary(total) :-
    !Blocked(_),
    agg total = sum(amount) in Purchase(_, amount);
relational {
    r0 = unit;
    r1 = rename Blocked {c0 -> blocked_anon0};
    r2 = antisemijoin r0 with r1;
    r3 = rename Purchase {c0 -> purchase_anon0, c1 -> amount};
    r4 = aggregate_apply r2 with r3 value amount using sum into total;
    r5 = project r4 keep {total};
    output r5 as summary(total);
}
indexes {}
```

Both the student pass and the staff contract predicate must implement the same
extended judgment. They share the numbered `STUDENT 4/6` region.

## `rust_access_plan::Plan` and physical Rust extension

The existing physical pass must consume the extended IndexRequirements object
and emit the same complete staged `Program + Storage` Rust file as Basic HW3, not
another negation/aggregation or storage descriptor IR. Its one
`pull! { rust_access_plan::Plan }` expression must retain every extended
logical occurrence beside the exact Rust leaf that implements it.

For the example, the relevant concrete objects are conceptually:

```rust
let rows0: ::std::vec::Vec<(::std::primitive::i32,)> =
    relation0.into_iter().collect();
let mut index0: ::std::collections::HashMap<
    (::std::primitive::i32,),
    ::std::vec::Vec<(::std::primitive::i32,)>,
> = ::std::collections::HashMap::new();
for (person, reason,) in relation1 {
    index0.entry((person,)).or_default().push((reason,));
}
let mut index1: ::std::collections::HashMap<
    (::std::primitive::i32,),
    ::std::vec::Vec<(::std::primitive::i32, ::std::primitive::i32,)>,
> = ::std::collections::HashMap::new();
for (person, category, amount,) in relation2 {
    index1
        .entry((person,))
        .or_default()
        .push((category, amount,));
}
```

Each normalized relation above has only one physical consumer, so that
consumer moves its rows. If an extended program requires several genuinely
different objects for one relation, earlier builders borrow and only the final
builder consumes it. Such an earlier eager builder may use
`relationN.iter()` and clone the selected key and complement fields to populate
an additional owning object. Those are construction-time field clones;
query-time positive and aggregate accesses must borrow their stored tuples
instead.

The occurrence-annotated residual region is already Rust-shaped. This excerpt
focuses on the new occurrence-to-access relation:

```rust
::mini_linq::pull! {
    spend(person, total) => ((
        ::core::clone::Clone::clone(person),
        total,
    )) :-
        Person(person) => for (person,) in (self.rows0.iter()),
        !Blocked(person, _) => if (!self.index0.contains_key(&(
            ::core::clone::Clone::clone(person),
        ))),
        agg total = sum(amount) in Purchase(person, _, amount)
            => for total in (::std::iter::once_with(move || {
                let wide_total = self.index1
                    .get(&(::core::clone::Clone::clone(person),))
                    .into_iter()
                    .flatten()
                    .map(|(_, amount,)| ::std::primitive::i128::from(*amount))
                    .try_fold(0_i128, |total, amount| total.checked_add(amount))
                    .expect("MiniLinq sum overflow");
                ::std::primitive::i32::try_from(wide_total)
                    .expect("MiniLinq sum overflow")
            }));
}
```

Extend `rust_access_plan::contract::check(&Plan)` so safe negation requires `If` and
binds nothing. Aggregation requires `For` with a simple scalar identifier
pattern exactly equal to its fresh output—`for total`, not `for (total,)`—and
binds only that result. The checker must not inspect the Rust expressions to
rediscover correlation or aggregation semantics.

The ownership distinction in this plan is deliberate. A positive source binds
`person` by reference. Negation only observes that borrowed value, so its key
clones just `person`. The aggregate bucket also yields borrowed complement
tuples, and its projection reads only the borrowed `amount` needed by `sum`.
The fold produces the owned scalar `total`; unlike a positive variable, that
aggregate output is moved into the head tuple. Thus no stored relation row or
index payload is cloned wholesale during traversal.

The aggregate source uses `once_with`, so constructing that source does not run
the fold; requesting its one row does. The mapped amounts accumulate in `i128`
and convert to `i32` only once. Do not fold directly in `i32`: `relationN` is a
`HashSet`, so iteration order is unspecified, and `MAX + 1 - 1` must not
sometimes overflow before the cancellation is seen. The aggregate result is
then bound by the `For` pattern.

The physical choice follows the already-derived key shape:

| Clause | Empty key | Proper key | Full key |
|---|---|---|---|
| `!R(...)` | test whether the row collection is empty | negate `HashMap::contains_key` | negate `HashSet::contains` |
| `sum(...) in R(...)` | fold the row iterator | fold the matching `HashMap` bucket | not required: the aggregate value column is excluded from the key |

Wildcard columns remain in a hash bucket's complement tuple. They are ignored
only by the Rust projection passed to `sum`; do not silently remove them from
the physical row representation.

HW4 needs no new execution IR after RustAccessPlan. It reuses the supplied
pull lowering and IteratorPipeline-to-Rust lowering unchanged. The access plan
above becomes:

```rust
::mini_linq::iterator_pipeline! {
    iter0 = scan (person,) in (self.rows0.iter()) yield (person,);
    iter1 = filter iter0 as (person,)
        if (!self.index0.contains_key(&(
            ::core::clone::Clone::clone(person),
        )));
    iter2 = join iter1 as (person,) with total in (::std::iter::once_with(move || {
        let wide_total = self.index1
            .get(&(::core::clone::Clone::clone(person),))
            .into_iter()
            .flatten()
            .map(|(_, amount,)| ::std::primitive::i128::from(*amount))
            .try_fold(0_i128, |total, amount| total.checked_add(amount))
            .expect("MiniLinq sum overflow");
        ::std::primitive::i32::try_from(wide_total)
            .expect("MiniLinq sum overflow")
    })) yield (person, total,);
    iter3 = project iter2 as (person, total,) yield ((
        ::core::clone::Clone::clone(person),
        total,
    ));
    iter4 = distinct iter3;
    return iter4;
}
```

The `iterN` identifiers make every physical stream edge explicit. The
intermediate bindings make the ownership transition visible: `person` remains
a borrowed field across physical `scan` and `filter`, while the aggregate
source produces the owned scalar `total` at the binary `join`. These names
must not be confused with the logical `rN` DAG references. More generally:

- a positive relation or aggregate result already supplies a `For` pattern and
  source expression, which becomes `scan` or binary `join`;
- negation already supplies an `If` condition, which becomes `filter`;
- the Plan head already supplies the output expression, which becomes
  `project`; and
- the unchanged IteratorPipeline lowering returns one lazy iterator over all
  extended-query results. Public `Storage::query` constructs that iterator
  without evaluating any access leaf; the explicit `distinct` stream removes
  projected duplicates as `next()` advances it. The head constructs an owned
  result row, and lazy distinctness retains each first-seen owned result; that
  result-level ownership cost is separate from borrowing physical tuples.

`Negation` and `Aggregate` remain visible in `rust_access_plan::Plan` because
its useful job is to check the logical occurrence-to-access relation. The
baseline pull pass then erases those logical labels while preserving their
concrete leaves in the existing operator IR. HW4 does not add another
operator language or special emitter.

## Marked implementation sites

| Marker | File | Required change |
|---|---|---|
| Supplied syntax (former `STUDENT 1/6`) | `crates/cq-ir/src/cq.rs` and `cq/rust.rs` | Read the derived clause syntax and lexical helpers; no parser implementation is assigned |
| `STUDENT 2/6 (WF)` | `crates/cq-ir/src/cq.rs` | Implement the left-to-right safety rules |
| `STUDENT 3/6 (LOGICAL)` | `crates/cq-ir/src/relational_plan.rs`, `crates/cq-compiler/src/pass_contracts.rs`, and `passes.rs` | Add `AntiSemijoin`/`AggregateApply`, their inferred-heading rules, and the exact extended-CQ-to-plan lowering; reuse the supplied `Unit` |
| `STUDENT 4/6 (INDEX)` | `crates/cq-compiler/src/pass_contracts.rs` and `passes.rs` | Extend canonical key derivation over the complete RelationalPlan |
| `STUDENT 5/6 (ANNOTATION)` | `crates/cq-ir/src/rust_access_plan.rs` | Extend occurrence-to-access checking for negation and aggregation |
| `STUDENT 6/6 (PHYSICAL)` | `crates/cq-compiler/src/passes.rs` | Emit the complete file with annotated concrete negative and aggregate Rust accesses |

You may add private helpers next to these sites. Do not add a generic pass
trait, a dynamic query interpreter, or a second storage/access descriptor IR.

## Tests and workflow

The syntax and lexical helper tests run in the ordinary suite. Complete the
local contracts and logical operators first:

```text
cargo test -p cq-ir --features hw4 --test hw4_optional
```

Then isolate both exact pass edges—extended CQ to RelationalPlan and that
complete plan to IndexRequirements—before testing physical Rust:

```text
cargo test -p cq-compiler --features hw4 --test hw4_optional
```

After all three completed basic passes support the cumulative extension, run
the normal `mini_linq!` endpoint tests in both debug and release modes:

```text
cargo test -p mini-linq --features hw4 --test hw4_optional
cargo test -p mini-linq --release --features hw4 --test hw4_optional
```

The endpoint checks duplicate elimination, safe negation, correlated sum,
empty-input sum behavior, checked overflow, and result rows. It must also use
an observable aggregate source to assert that constructing
`Storage::query()` does not run the aggregate fold and that the first
relevant `next()` does. The ordinary basic-checkpoint commands do not enable
`hw4`, so this extension cannot block a basic submission.

Include the global example in all three IR/pass suites. Test the exact single
`Unit`, both empty inferred correlations, the empty index overlay, empty and
nonempty `Blocked`, and empty and nonempty `Purchase`.

## Definition of done

- The extended query parses, re-emits, reparses, and compares structurally.
- The `Query` and `Module` EBNF comments describe `BodyItem`, not the old
  positive-only body.
- `cq::contract::check` returns `syn::Error` for unsafe correlation and
  aggregate-result shadowing.
- The complete RelationalPlan contains exact named `AntiSemijoin` and
  `AggregateApply` results, preserves wildcard source columns needed by
  aggregation, and passes both its local and cross-stage contracts.
- A global first clause produces exactly one supplied `Unit`; global
  AntiSemijoin and AggregateApply infer an empty correlation intersection, and
  AntiSemijoin preserves Unit's empty heading.
- IndexRequirements preserves that entire logical plan rather than replacing
  it with only a key list.
- The exact requirements are `Blocked[0]` then `Purchase[0]`.
- The global example has no index requirements and never writes an empty-key
  index entry.
- `rust_access_plan::Plan` retains and checks every extended logical occurrence beside
  its concrete Rust access.
- The physical pass emits concrete Rust builders and access expressions in the
  complete `Program + Storage` shell. `Program::build` performs normalization
  and index construction; constructing the iterator returned by
  `Storage::query` evaluates no access leaf or aggregate fold.
- The supplied RustAccessPlan-to-IteratorPipeline and IteratorPipeline-to-Rust
  lowerings are unchanged.
- All four opt-in commands above pass.
- You can explain why this one-rule subset is stratified by construction.

## Honest stretch directions

- Add `min` and `max`; unlike `sum`, they emit no value on an empty input.
- Add Ascent's `count()`. Its result is `usize`, so this requires an explicit
  output-type/row-representation design; silently casting to `i32` is not an
  acceptable implementation.
- Add parameterized or user-defined aggregator paths.
- Add derived relations and then implement a real dependency graph and
  stratification check.

These are separate semantic extensions, not extra match arms hidden in the
Rust emitter.
