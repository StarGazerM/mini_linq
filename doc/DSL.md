# MiniLinq basic language and lowering contract

The basic checkpoint exposes every important query-compilation boundary. A
student implements three typed passes; staff supplies the parsers, contracts,
macro routing, pull lowering, final Rust lowering, and test harness.

```text
CQ
  -- Basic HW1 --> RelationalPlan
  -- Basic HW2 --> IndexRequirements containing that complete RelationalPlan
  -- Basic HW3 --> staged syn::File containing pull! { RustAccessPlan }
  -- supplied pull --> same file containing iterator_pipeline! { IteratorPipeline }
  -- supplied Rust lowering --> final syn::File containing ordinary Rust
```

The first three named languages describe one complete query. The last two
named languages occupy one expression inside an otherwise ordinary Rust file.
That complete `syn::File` is a **staged Rust program** or **program residual**.
It is not another abstract whole-query IR.

`r0`, `r1`, and so on are serialized DAG references to logical,
set-valued results. They are not algebra operators, and named relational
algebra does not depend on an SSA presentation. `iter0`, `iter1`, and so on
name physical, lazy cursor results:

```text
rN    = a logical relation with a named heading
iterN = a resumable Iterator value and its Rust item binding
```

`IteratorPipeline` exposes the physical operator edges but deliberately does
not choose `map`, `flat_map`, or another Rust combinator. The supplied
`compile_iterator_pipeline` pass makes those choices and returns `syn::Expr`.
That expression is already the faithful concrete target IR. The proc-macro
entry then uses `ToTokens` only to emit the AST; token emission performs no
additional planning or lowering.

## Exact typed boundaries

| Owner | Function | Exact result |
|---|---|---|
| student, Basic HW1 | `compile_cq(&cq::Module)` | `syn::Result<relational_plan::Module>` |
| student, Basic HW2 | `compile_relational_plan(&relational_plan::Module)` | `syn::Result<index_requirements::Module>` preserving the complete input plan |
| student, Basic HW3 | `compile_index_requirements(&index_requirements::Module)` | `syn::Result<syn::File>` containing the complete API and one `pull!` expression |
| staff | `compile_pull(&rust_access_plan::Plan)` | `iterator_pipeline::Pipeline` with explicit `iterN` edges |
| staff | `compile_iterator_pipeline(&iterator_pipeline::Pipeline)` | one lazy whole-result-set `syn::Expr` |

These functions accept typed syntax values, not token streams. The
`mini-linq` proc-macro crate owns token transport: each direct macro parses its
exact input type, calls the corresponding typed function, and emits the result.
Syntax failures therefore remain span-aware `syn::Error` values. Semantic
well-formedness is the typed pass contract; the macro entry does not duplicate
that check.

## Planned alternatives through the same boundaries

The [Stage I implementation](PLANNED-PASS-IMPLEMENTATION.md) adds alternative
edges without replacing the basic homework contracts:

| Macro entry | Typed transformation |
|---|---|
| `planned_query!` | `compile_cq_planned`: CQ -> RelationalPlan |
| `planned_relational_plan!` | `compile_result_indexes`: RelationalPlan -> IndexRequirements |
| `materialized!` | `compile_materialized`: IndexRequirements -> staged Rust |

The target Rust file has one existing local `pull!` expression per named RA
result. Materialized sets outside these expressions preserve shared DAG
references. Each expression can be lowered independently by the existing
pull and IteratorPipeline passes. `build` performs the materialization;
`query` traverses the stored answer.

The nominal languages additionally support the following forms:

```ebnf
BodyItem         ::= Atom | "if" "(" Predicate ")"
Predicate        ::= Term ("==" | "!=" | "<" | "<=" | ">" | ">=") Term
Term             ::= Ident | I32Literal
RelOperator      ::= ... | "semijoin" RelId "with" RelId
                        | "select" RelId "if" "(" Predicate ")"
IndexRequirement ::= ["@"] Ident "[" Column ("," Column)* [","] "]"
```

Predicates are primitive i32 comparisons; Rust resolves aliases and rejects
incompatible types. Calls and arithmetic are outside this movable fragment.
`@r[0, 2]` refers to ordinals in result r's sorted heading; `R[0, 2]` retains
declared input-column order. Empty shared headings need no index entry.
The RA Semijoin keeps exactly its left heading and uses all shared attributes.
Local physical regions permit empty headings and bound comparison clauses
paired with Rust `if` accesses. The R1 passes reject forms outside their domain.

Complete, executable artifacts demonstrate each boundary:
[CQ](planned-example/01-cq.rs), [RA](planned-example/02-relational.rs),
[indexes](planned-example/03-indexes.rs), [access regions](planned-example/04-access.rs),
[pipelines](planned-example/05-pipelines.rs), and [Rust](planned-example/06-rust.rs).
They also demonstrate partial lowering and quotation at the existing macro
entries. A CQ-level embedding of arbitrary RA is not defined by these changes.

## Basic grammar summary

`RustVisibility`, `RustType`, `RustPat`, and `RustExpr` are parsed by `syn`.
`Column` is an unsuffixed integer literal.

```ebnf
CQModule          ::= Program
Program           ::= RustVisibility? "struct" Ident ";"
                      "relation" RelationDecl ("relation" RelationDecl)*
                      Query
RelationDecl      ::= Ident "(" ColumnDecl ("," ColumnDecl)* [","] ")" ";"
ColumnDecl        ::= Ident ":" RustType
Query             ::= Atom ":-" BodyItem ("," BodyItem)* ("." | ";")
BodyItem          ::= Atom | "if" RustExpr | "let" RustPat "=" RustExpr
Atom              ::= Ident "(" [RustExpr ("," RustExpr)* [","]] ")"

RelationalModule  ::= Program "relational" "{" RelationalPlan "}"
RelationalPlan    ::= RelDefinition* RelOutput
RelDefinition     ::= RelId "=" RelOperator ";"
RelOperator       ::= "unit"
                    | "rename" Ident "{" RenameMap ("," RenameMap)* [","] "}"
                    | "natural_join" RelId "with" RelId
                    | "project" RelId "keep" "{" [Ident ("," Ident)* [","]] "}"
                    | "semijoin" RelId "with" RelId
                    | "select" RelId "if" "(" RustExpr ")"
                    | "extend" RelId "with" "{" [ColumnDecl ("," ColumnDecl)*] "}"
                      "value" "(" RustExpr ")"
RenameMap         ::= Ident "->" Ident
RelOutput         ::= "output" RelId "as" Atom "."

IndexModule       ::= RelationalModule "indexes" "{"
                      [IndexRequirement (";" IndexRequirement)* [";"]]
                      "}"
IndexRequirement  ::= Ident "[" [Column ("," Column)* [","]] "]"

RustAccessPlan    ::= Output ":-" Clause ("," Clause)* "."
Output            ::= Atom "=>" "(" RustExpr ")"
Clause            ::= Atom "=>" RustAccess
RustAccess        ::= "for" RustPat "in" "(" RustExpr ")"
                    | "if" "(" RustExpr ")"

IteratorPipeline  ::= IterDefinition* IterReturn
IterDefinition    ::= IterId "=" IterOperator ";"
IterOperator      ::= "unit" "yield" RustExpr
                    | "scan" RustPat "in" "(" RustExpr ")"
                      "yield" RustExpr
                    | "join" IterId "as" RustPat "with" RustPat
                      "in" "(" RustExpr ")" "yield" RustExpr
                    | "filter" IterId "as" RustPat "if" "(" RustExpr ")"
                    | "project" IterId "as" RustPat "yield" "(" RustExpr ")"
                    | "distinct" IterId
IterReturn        ::= "return" IterId "."
```

The table above describes the basic semantic checkpoint. The supplied CQ
parser also accepts the following body forms, with Rust leaves parsed by Syn:

```ebnf
Query             ::= Atom ":-" BodyItem ("," BodyItem)* ";"
BodyItem          ::= Atom | "if" RustExpr
                    | "let" RustPat [":" RustType] "=" RustExpr
                    | "!" Atom | Aggregate
Aggregate         ::= "agg" RustPat "=" Aggregator
                      "(" [Ident ("," Ident)* [","]] ")" "in" Atom
Aggregator        ::= RustPath | "(" RustExpr ")"
```

The basic CQ contract accepts positive atoms with distinct bare variables;
parsing a larger surface does not implement its semantics. The
[Rust-expression helpers](RUST-EXPRESSION-HELPERS.md) provide lexical free names
and pattern bindings for students extending those contracts and passes.
Aggregate arguments are local binder names; Rust expressions occur in its
parenthesized aggregator and input atom arguments.

MiniLinq deliberately uses [Ascent's typed relation-declaration
shape](https://docs.rs/crate/ascent/latest). In this one-rule query language,
a declaration supplies a symbol and named Rust columns in positional order.
Input instances are passed to `build` or `run`. An optional declaration with
the head relation name specifies the result types and is not an input.
Variable-only heads infer their types from inputs or explicitly typed `let`
bindings; computed heads require the result declaration. The DSL has no
fact-insertion form. See [Rust expressions](RUST-EXPRESSIONS.md) for expression
arguments, bindings, scope, and evaluation order.

The number of named columns is the relation's arity. Their types are
operational, not comments: generated rows, keys, index payloads, and results
use those exact Rust types. The IR contracts preserve those type-syntax values
but do not compare them for semantic equality: Rust must resolve aliases and
check compatibility in the generated program. Set storage and result
deduplication require the corresponding tuple types to implement `Eq` and
`Hash`; generated ownership boundaries require `Clone`; deterministic
`materialize` additionally requires the output tuple to implement `Ord`.
These are ordinary Rust trait checks, not a second MiniLinq type system.

`unit` is the singleton zero-attribute relation `{()}`. Its exact syntax is
`rN = unit;`, and it has no operand. It is supplied as a general logical
source so the negation-and-aggregation extension can seed a global clause
before any positive atom binds an attribute.

A declared relation name is an atomic input expression. `rename` represents
one occurrence by mapping every declared source attribute exactly once to a
distinct query variable. `natural_join` infers its shared attributes from the
two headings and returns the heading union. It is still used when the right heading is wholly shared.
`project` keeps a distinct subset of attributes. Ordered result tuple layout
belongs only to the separate `output result as head;` metadata. Baseline atoms
contain distinct variables and no constants, so this fragment does not yet
need Selection; add it only with that source-language extension.

The RelationalPlan local language permits any acyclic dependency graph whose
operands are earlier definitions, including a bushy plan. It checks local
heading rules and reachability but does not enforce CQ occurrence or source
order. The exact CQ-to-relational pass contract owns the canonical policy: one
Rename per body occurrence, source-order left-deep NaturalJoin folding, one
final Project, and output metadata. The Basic-HW2 lowering is deliberately
narrower: every NaturalJoin must consume a direct right-hand Rename in a
left-deep chain, and the plan may not contain `Unit`. It returns a span-aware
error for another locally valid shape. The negation-and-aggregation extension extends this
edge to consume `Unit`; a future bushy backend can consume other logical
shapes separately.

`IndexRequirements` is an overlay, not a replacement plan. It repeats the
complete RelationalPlan and adds only the equality keys required by its
NaturalJoins. For a supported right-hand Rename, the required attributes are
the heading intersection; inverting its source-attribute mapping yields key
positions in declaration order. A key is written in increasing base-relation
column order. Repeated uses of one `(relation, key)` share one requirement.

`RustAccessPlan` keeps a logical occurrence beside its already selected Rust
access. Its Rust leaves are concrete: row-store iteration, hash lookup,
membership condition, and output expression have already been chosen. The
remaining labels let the local checker enforce source-order binding and the
required `for` versus `if` control shape.

`IteratorPipeline` gives every lazy operator result a name. `scan` creates the
root cursor; each binary `join` consumes the preceding cursor and a
parameterized right source; `filter` preserves its input binding; `project`
constructs the owned result row; and `distinct` enforces CQ set semantics.
These are streams of intermediate bindings, not materialized intermediate
relations.

Both logical RelationalPlan and physical IteratorPipeline spell an identity
source `unit`, but they remain different typed objects. Logical `Unit` denotes
the set relation `{()}`. Physical `Unit` denotes a lazy cursor equivalent to
`std::iter::once(())`, synthesized when a RustAccessPlan starts with a
predicate rather than an enumerating source.

## Complete Triangle expansion

Every fence below is the entire artifact at that boundary. There are no
omitted declarations or pseudocode placeholders.

### 1. Entire CQ

```rust
::mini_linq::mini_linq! {
    pub struct TriangleProgram;
    relation R(c0: i32, c1: i32);
    relation S(c0: i32, c1: i32);
    relation T(c0: i32, c1: i32);
    triangle(x, y, z) :-
        R(x, y),
        S(y, z),
        T(z, x).
}
```

The CQ states logical meaning and source order. It contains neither a join
operator nor an index or Rust execution choice.

### 2. Entire RelationalPlan program

```rust
::mini_linq::relational_plan! {
    pub struct TriangleProgram;
    relation R(c0: i32, c1: i32);
    relation S(c0: i32, c1: i32);
    relation T(c0: i32, c1: i32);
    triangle(x, y, z) :-
        R(x, y),
        S(y, z),
        T(z, x).
    relational {
        r0 = rename R {c0 -> x, c1 -> y};
        r1 = rename S {c0 -> y, c1 -> z};
        r2 = natural_join r0 with r1;
        r3 = rename T {c0 -> z, c1 -> x};
        r4 = natural_join r2 with r3;
        r5 = project r4 keep {x, y, z};
        output r5 as triangle(x, y, z).
    }
}
```

Every source occurrence gets its own Rename. `r2` is the natural join of the
first two occurrences. The fully bound `T(z, x)` occurrence is still folded by
the NaturalJoin `r4`; later physical lowering recognizes that it introduces no
fresh attributes and chooses membership. `r5` is the set projection, while the
separate output metadata preserves the head name and tuple order.

### 3. Entire IndexRequirements program

```rust
::mini_linq::index_requirements! {
    pub struct TriangleProgram;
    relation R(c0: i32, c1: i32);
    relation S(c0: i32, c1: i32);
    relation T(c0: i32, c1: i32);
    triangle(x, y, z) :-
        R(x, y),
        S(y, z),
        T(z, x).
    relational {
        r0 = rename R {c0 -> x, c1 -> y};
        r1 = rename S {c0 -> y, c1 -> z};
        r2 = natural_join r0 with r1;
        r3 = rename T {c0 -> z, c1 -> x};
        r4 = natural_join r2 with r3;
        r5 = project r4 keep {x, y, z};
        output r5 as triangle(x, y, z).
    }
    indexes {
        S[0];
        T[0, 1];
    }
}
```

The entire logical plan remains visible. The root `R` occurrence has no bound
key and needs no equality index. The second NaturalJoin requires `S` column 0;
the fully bound third occurrence requires all columns of `T`. Requirements are
deduplicated by `(relation, columns)` in first-use order, so the same rule also
handles self-joins and multiple indexes on one relation without redundant
structures.

### 4. Entire staged Rust file with one RustAccessPlan

```rust
pub struct TriangleProgram;
pub struct TriangleProgramStorage {
    rows0: ::std::vec::Vec<(i32, i32)>,
    index0: ::std::collections::HashMap<(i32,), ::std::vec::Vec<(i32,)>>,
    index1: ::std::collections::HashSet<(i32, i32)>,
}
impl TriangleProgram {
    pub fn build(
        input0: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input1: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input2: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
    ) -> TriangleProgramStorage {
        let relation0: ::std::collections::HashSet<(i32, i32)> = input0
            .into_iter()
            .collect();
        let relation1: ::std::collections::HashSet<(i32, i32)> = input1
            .into_iter()
            .collect();
        let relation2: ::std::collections::HashSet<(i32, i32)> = input2
            .into_iter()
            .collect();
        let index0: ::std::collections::HashMap<(i32,), ::std::vec::Vec<(i32,)>> = {
            let mut index0: ::std::collections::HashMap<
                (i32,),
                ::std::vec::Vec<(i32,)>,
            > = ::std::collections::HashMap::new();
            for (index0_column0, index0_column1) in relation1.into_iter() {
                index0.entry((index0_column0,)).or_default().push((index0_column1,));
            }
            index0
        };
        let rows0: ::std::vec::Vec<(i32, i32)> = relation0.into_iter().collect();
        let index1: ::std::collections::HashSet<(i32, i32)> = relation2;
        TriangleProgramStorage {
            rows0,
            index0,
            index1,
        }
    }
    pub fn run(
        input0: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input1: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input2: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
    ) -> ::std::vec::Vec<(i32, i32, i32)> {
        Self::build(input0, input1, input2).materialize()
    }
}
impl TriangleProgramStorage {
    pub fn query(&self) -> impl ::std::iter::Iterator<Item = (i32, i32, i32)> + '_ {
        ::mini_linq::pull! {
            triangle(x, y, z) => (
                (
                    ::core::clone::Clone::clone(x),
                    ::core::clone::Clone::clone(y),
                    ::core::clone::Clone::clone(z),
                )
            ) :-
                R(x, y) => for (x, y,) in (self.rows0.iter()),
                S(y, z) => for (z,) in (
                self.index0.get(&(::core::clone::Clone::clone(y),)).into_iter().flatten()
            ),
                T(z, x) => if (
                self.index1
                    .contains(&(::core::clone::Clone::clone(z), ::core::clone::Clone::clone(x)))
            ).
        }
    }
    pub fn materialize(&self) -> ::std::vec::Vec<(i32, i32, i32)> {
        let mut result = self.query().collect::<::std::vec::Vec<_>>();
        result.sort_unstable();
        result
    }
}
```

Basic HW3 has already made every storage and Rust-access decision. `build`
normalizes each used public input once, builds indexes outside the result
iterator, and moves each normalized relation into its final physical owner.
The proper-key map consumes `relation1`, the row store consumes `relation0`,
and the full-key set directly takes `relation2`; no identical full-key set is
recollected.

`query` returns one iterator for the entire result set. It does not contain a
loop that eagerly materializes that result. The remaining plan pairs each
logical occurrence with its exact Rust source or predicate and pairs the head
with the owned output expression.

### 5. Entire staged Rust file with one IteratorPipeline

```rust
pub struct TriangleProgram;
pub struct TriangleProgramStorage {
    rows0: ::std::vec::Vec<(i32, i32)>,
    index0: ::std::collections::HashMap<(i32,), ::std::vec::Vec<(i32,)>>,
    index1: ::std::collections::HashSet<(i32, i32)>,
}
impl TriangleProgram {
    pub fn build(
        input0: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input1: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input2: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
    ) -> TriangleProgramStorage {
        let relation0: ::std::collections::HashSet<(i32, i32)> = input0
            .into_iter()
            .collect();
        let relation1: ::std::collections::HashSet<(i32, i32)> = input1
            .into_iter()
            .collect();
        let relation2: ::std::collections::HashSet<(i32, i32)> = input2
            .into_iter()
            .collect();
        let index0: ::std::collections::HashMap<(i32,), ::std::vec::Vec<(i32,)>> = {
            let mut index0: ::std::collections::HashMap<
                (i32,),
                ::std::vec::Vec<(i32,)>,
            > = ::std::collections::HashMap::new();
            for (index0_column0, index0_column1) in relation1.into_iter() {
                index0.entry((index0_column0,)).or_default().push((index0_column1,));
            }
            index0
        };
        let rows0: ::std::vec::Vec<(i32, i32)> = relation0.into_iter().collect();
        let index1: ::std::collections::HashSet<(i32, i32)> = relation2;
        TriangleProgramStorage {
            rows0,
            index0,
            index1,
        }
    }
    pub fn run(
        input0: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input1: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input2: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
    ) -> ::std::vec::Vec<(i32, i32, i32)> {
        Self::build(input0, input1, input2).materialize()
    }
}
impl TriangleProgramStorage {
    pub fn query(&self) -> impl ::std::iter::Iterator<Item = (i32, i32, i32)> + '_ {
        ::mini_linq::iterator_pipeline! {
            iter0 = scan (x, y,) in (self.rows0.iter()) yield (x, y,);
            iter1 = join iter0 as (x, y,) with (z,) in (
                self.index0.get(&(::core::clone::Clone::clone(y),)).into_iter().flatten()
            ) yield (x, y, z,);
            iter2 = filter iter1 as (x, y, z,) if (
                self.index1
                    .contains(&(::core::clone::Clone::clone(z), ::core::clone::Clone::clone(x)))
            );
            iter3 = project iter2 as (x, y, z,) yield (
                (
                    ::core::clone::Clone::clone(x),
                    ::core::clone::Clone::clone(y),
                    ::core::clone::Clone::clone(z),
                )
            );
            iter4 = distinct iter3;
            return iter4.
        }
    }
    pub fn materialize(&self) -> ::std::vec::Vec<(i32, i32, i32)> {
        let mut result = self.query().collect::<::std::vec::Vec<_>>();
        result.sort_unstable();
        result
    }
}
```

Only the expression inside `query` changed. Each physical operator now has an
explicit output edge. `iter1` names the parameterized binary nested-loop join
of `iter0` with the indexed `S` lookup; `iter2` filters that stream by `T`
membership; `iter3` projects; and `iter4` performs lazy result distinctness.
The plan describes those physical relationships without yet committing to
Rust's iterator-adapter spelling.

### 6. Entire final Rust file after iterator-pipeline lowering

```rust
pub struct TriangleProgram;
pub struct TriangleProgramStorage {
    rows0: ::std::vec::Vec<(i32, i32)>,
    index0: ::std::collections::HashMap<(i32,), ::std::vec::Vec<(i32,)>>,
    index1: ::std::collections::HashSet<(i32, i32)>,
}
impl TriangleProgram {
    pub fn build(
        input0: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input1: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input2: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
    ) -> TriangleProgramStorage {
        let relation0: ::std::collections::HashSet<(i32, i32)> = input0
            .into_iter()
            .collect();
        let relation1: ::std::collections::HashSet<(i32, i32)> = input1
            .into_iter()
            .collect();
        let relation2: ::std::collections::HashSet<(i32, i32)> = input2
            .into_iter()
            .collect();
        let index0: ::std::collections::HashMap<(i32,), ::std::vec::Vec<(i32,)>> = {
            let mut index0: ::std::collections::HashMap<
                (i32,),
                ::std::vec::Vec<(i32,)>,
            > = ::std::collections::HashMap::new();
            for (index0_column0, index0_column1) in relation1.into_iter() {
                index0.entry((index0_column0,)).or_default().push((index0_column1,));
            }
            index0
        };
        let rows0: ::std::vec::Vec<(i32, i32)> = relation0.into_iter().collect();
        let index1: ::std::collections::HashSet<(i32, i32)> = relation2;
        TriangleProgramStorage {
            rows0,
            index0,
            index1,
        }
    }
    pub fn run(
        input0: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input1: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
        input2: impl ::std::iter::IntoIterator<Item = (i32, i32)>,
    ) -> ::std::vec::Vec<(i32, i32, i32)> {
        Self::build(input0, input1, input2).materialize()
    }
}
impl TriangleProgramStorage {
    pub fn query(&self) -> impl ::std::iter::Iterator<Item = (i32, i32, i32)> + '_ {
        ::core::iter::Iterator::flatten(
            ::core::iter::once_with(move || {
                let iter0 = ::core::iter::Iterator::map(
                    ::core::iter::IntoIterator::into_iter(self.rows0.iter()),
                    move |(x, y)| (x, y),
                );
                let iter1 = ::core::iter::Iterator::flat_map(
                    iter0,
                    move |(x, y)| {
                        ::core::iter::Iterator::map(
                            ::core::iter::IntoIterator::into_iter(
                                self
                                    .index0
                                    .get(&(::core::clone::Clone::clone(y),))
                                    .into_iter()
                                    .flatten(),
                            ),
                            move |(z,)| {
                                let x = ::core::clone::Clone::clone(&x);
                                let y = ::core::clone::Clone::clone(&y);
                                (x, y, z)
                            },
                        )
                    },
                );
                let iter2 = ::core::iter::Iterator::filter_map(
                    iter1,
                    move |#[allow(unused_variables)] (x, y, z)| {
                        if self
                            .index1
                            .contains(
                                &(
                                    ::core::clone::Clone::clone(z),
                                    ::core::clone::Clone::clone(x),
                                ),
                            )
                        {
                            ::core::option::Option::Some((x, y, z))
                        } else {
                            ::core::option::Option::None
                        }
                    },
                );
                let iter3 = ::core::iter::Iterator::map(
                    iter2,
                    move |#[allow(unused_variables)] (x, y, z)| (
                        ::core::clone::Clone::clone(x),
                        ::core::clone::Clone::clone(y),
                        ::core::clone::Clone::clone(z),
                    ),
                );
                let iter4 = {
                    let mut seen = ::std::collections::HashSet::new();
                    ::core::iter::Iterator::filter(
                        iter3,
                        move |row| seen.insert(::core::clone::Clone::clone(row)),
                    )
                };
                iter4
            }),
        )
    }
    pub fn materialize(&self) -> ::std::vec::Vec<(i32, i32, i32)> {
        let mut result = self.query().collect::<::std::vec::Vec<_>>();
        result.sort_unstable();
        result
    }
}
```

This last transformation is semantic lowering, not pretty-printing. It maps
the named operators to ordinary Rust `map`, `flat_map`, `filter_map`, and `filter` values,
one `let iterN` per plan definition. `once_with(...).flatten()` defers source
evaluation and distinct-state construction until the first `next()`. Every
later `next()` resumes the saved iterator state and does only enough work to
produce the next distinct result. Dropping a partially consumed iterator drops
that state without scanning the rest of the relations.

`iter0` and the indexed lookup borrow their stored tuples. The query therefore
does not copy a row from `rows0` or a payload from `index0`. It does copy the
small scalar values needed for lookup keys and constructs one owned result
array. Because the result ABI is copyable and `query` promises set semantics,
the stateful `distinct` filter also stores one copy of every first-seen result.
That is result-level state, not a copy of a stored input tuple.

`materialize` is deliberately separate. It drains the already-distinct cursor
into a `Vec`, sorts it for deterministic tests and display, and returns it.
Index construction never occurs inside the cursor. No async runtime is needed
for this finite in-memory protocol; async would be useful only when requesting
the next item can actually wait on I/O or a channel.

## Local and edge guarantees

### CQ

- Input relation and per-relation column names are unique; arities are positive.
- Every body atom names a declared input with matching positional arity.
- Bare atom arguments bind variables; repeated arguments constrain equality.
- Computed arguments and guards use the preceding lexical environment; `_`
  matches an anonymous column. `let` adds fresh pattern bindings.
- Known logical dependencies are bound before use. Host Rust names and types
  are resolved by rustc. Computed heads have declared result types.

### RelationalPlan

- Every result identifier is unique, every operand was defined earlier, and all
  definitions are reachable from output.
- Rename names a declared relation, covers its source-attribute domain exactly,
  and has distinct targets.
- NaturalJoin infers the heading union.
- Project keeps a distinct subset of its input heading.
- Select retains its complete Rust condition; Extend adds fresh computed columns.
- Output exactly preserves the source head. Its input retains every expression
  dependency and, for computed heads, the witnesses needed before evaluation.
- These are local rules; the separate exact pass contract enforces source
  occurrence order and the canonical left-deep shape.

### IndexRequirements

- It preserves the complete RelationalPlan, including all `rN` DAG references.
- Relation names and key columns are valid; keys are nonempty, strictly
  increasing, and unique.
- Basic HW2 emits exactly the first-use union required by supported right-side
  Renames.

### RustAccessPlan residual

- Each logical occurrence remains paired with one concrete Rust access.
- An occurrence with fresh variables uses `for` and binds exactly those fresh
  variables in relation-column order.
- A fully bound positive occurrence uses `if` and binds nothing.
- The head refers only to bound variables and carries the exact owned Rust
  result expression.

### IteratorPipeline

- Definitions are sequentially named `iter0`, `iter1`, and so on.
- Every non-source operator consumes the immediately preceding stream.
- Complete input and output binding tuples make every operator edge visible.
- There is exactly one projection, immediately followed by one final
  `distinct`; the returned stream is that distinct result.
- The exact edge contract preserves every Rust source, predicate, binding, and
  output from RustAccessPlan.

### Final Rust expression

- `compile_iterator_pipeline` chooses and constructs the Rust combinators.
- One Rust local corresponds to every IteratorPipeline definition.
- The returned cursor is root-lazy and resumable.
- Proc-macro `ToTokens` emission performs no further semantic work.

## Homework boundary

The only required implementation bodies are the three functions in
`crates/cq-compiler/src/passes.rs`:

1. Basic HW1 constructs the complete canonical RelationalPlan.
2. Basic HW2 preserves that plan and derives its exact index overlay.
3. Basic HW3 walks the indexed logical plan and constructs the complete
   sequential Rust residual.

The pull and IteratorPipeline-to-Rust lowerings are supplied so students can
inspect their exact inputs and outputs without also having to invent the
execution protocol. The [negation-and-aggregation option](HW4-OPTIONAL.md)
cumulatively extends CQ and RelationalPlan on its own pick-one branch. The
staff-supplied logical `Unit` source seeds a global clause;
the positive Basic-HW1 pass never emits it, and the baseline Basic-HW2 pass
rejects it. Students cumulatively edit the production grammar and passes; the
legacy-named `hw4` feature gates only that branch's tests. It does not change the
meaning of a baseline macro or select a backend. Once implemented, the
extended checkpoint continues to use `mini_linq!`.

## Verification

```text
cargo test -p cq-ir --test relational_plan
cargo test -p cq-ir --test iterator_pipeline
cargo test -p cq-compiler --test pull_lowering
cargo test -p cq-compiler --test documented_triangle
cargo test -p mini-linq --test endpoint
```
