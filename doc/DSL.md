# MiniLinq languages and compilation boundaries

The common query path is `CQ → RA → IX → Physical → Structures → Rust`. The
[crate map](COMPILER-STRUCTURE.md) identifies each stage's implementation
owner. The syntax below distinguishes language validity from the narrower
domain a caller's policy admits.

## Typed boundaries

| Transformation | Typed owner |
|---|---|
| CQ → RA | `cq_query_lowering::{written_order,clause_prefix,hypergraph}::compile` |
| RA → IX | `cq_index_policy::compile` |
| IX + policy → Physical | `cq_physical_lowering::compile` |
| Physical + policy → Structures | `cq_structure_selection::compile` |
| Structures → Rust | `cq_physical_realization::compile` |

`mini-linq` supplies procedural entries and transports each typed input/output.
The policy and the distinct-state provider are caller-resolved macro slots
(`physical_policy!`, `select_structures!`, `structures_to_rust!`,
`distinct_state!`). Syntax types and validity checks live in `cq-ir`.

The printer emits a Rust block in a caller-written host. Input identifiers
refer to bindings in that scope. The CQ `struct Name;` is retained program
metadata; it does not promise a generated `build/run/query` API.

## CQ and Rust leaves

`RustVisibility`, `RustType`, `RustPat`, and `RustExpr` use Syn's Rust parsers.
Declarations remain in source order; `Program::inputs()` and `Program::output()`
classify them without changing the syntax. Typed `let` retains its annotation
beside its pattern. Rust expressions are parsed without token rewriting.

```ebnf
CQModule       ::= Program
Program        ::= RustVisibility? "struct" Ident ";" RelationDecl+ Query
RelationDecl   ::= IndexAdvice? "relation" Ident "(" [ColumnDecl ("," ColumnDecl)* [","]] ")" ";"
IndexAdvice    ::= "#" "[" "index" "(" Ident ")" "]"
ColumnDecl     ::= Ident ":" RustType
Query          ::= Atom ":-" BodyItem ("," BodyItem)* ";"
Atom           ::= Ident "(" [RustExpr ("," RustExpr)* [","]] ")"
BodyItem       ::= Atom | "if" RustExpr
                 | "let" RustPat [":" RustType] "=" RustExpr
                 | "!" Atom | Aggregate
Aggregate      ::= "agg" RustPat "=" Aggregator
                   "(" [Ident ("," Ident)* [","]] ")" "in" Atom
Aggregator     ::= RustPath | "(" RustExpr ")"
```

The parser accepts negation and aggregation, while the baseline semantic
contract rejects them pending the optional extension. Aggregate arguments are
local binder names; Rust expressions occur in the parenthesized aggregator and
input atom arguments. Parsing an empty column list also does not override the
input contract's positive-arity requirement.

A declaration names Rust columns in positional order. A declaration matching
the query head supplies result types and is not an input. Variable-only heads
infer types from input/typed bindings; computed heads require a declared result
schema. There is no fact-insertion syntax.

`#[index(Type)]` on a declaration is storage advice, not meaning: the query
denotes the same set with or without it. `Type` is any Rust type — from any
crate — implementing `cq_storage::Build`; structure selection builds it over
whatever the plan indexes on that relation, and the generated program reads
it only through the runtime traits behind the operations the plan performs
(`IndexRead` for probes, `ExactMembership` for semijoins, `OrderedRead` under
`join = merge`). Nothing is claimed: rustc checks the type where the program
uses it, at a one-line guard beside the structure. A bare path has its
generics inferred from the key and payload tuples; written arguments are
used as is. It names no columns — which columns are indexed is a planner
decision the query cannot see. Without advice the defaults are `HashIndex`
for joins, `HashSet` for semijoins, `BTreeMap` under `merge`.

Likewise the policy states facts about host values, never capabilities:
`R keyed [a]` means the host is a map keyed on column `a` (so a demand on `a`
is answered in place), `R keyed [a] ordered` that the key has an order;
`L;` is a plain collection. Strategy (`join = merge`, `semijoin =
prefer_lazy`) is the policy's decision, and every operation it leads the
program to perform is a trait bound rustc checks on the host value.

CQ validity and expression scheduling preserve these facts:

- Input names and per-relation column names are unique; input arities are positive.
- Each positive atom names a declared input with the correct positional arity.
- Bare arguments bind variables; repeated arguments impose equality. Computed
  arguments and guards use the preceding lexical environment; `_` is anonymous.
- `let` introduces fresh pattern bindings and retains its complete Rust expression.
- Known logical dependencies must be bound before use. Rust resolves host names,
  type aliases and operations; there is no independent Rust type checker here.
- Rust effects constrain expression scheduling. The dependency-aware prefix
  planner preserves its admitted expression semantics; the graph planner admits
  its own movable fragment, not arbitrary calls merely because they parse.

The graph planner's primitive comparison fragment has terms `Ident` or i32
literals and comparisons `==`, `!=`, `<`, `<=`, `>`, `>=`. Calls/arithmetic are
outside that movable fragment. [Rust-expression helpers](RUST-EXPRESSION-HELPERS.md)
describe lexical free-name and pattern-binding analysis.

Column types are operational, not comments. Host maps and built structures are
typed over the declared columns. Trait requirements are local choices: a
hash-keyed read requires Eq/Hash on its key columns, an ordered read requires
Ord, and a distinct provider requires Eq/Hash or Ord on the rows it retains.
There is no universal payload `Clone` requirement or implicit deterministic
sorting step.

## Named relational algebra

```ebnf
RelationalModule ::= Program "relational" "{" RelationalPlan "}"
RelationalPlan   ::= RelDefinition* RelOutput
RelDefinition    ::= RelId "=" RelOperator ";"
RelOperator      ::= "unit"
                  | "rename" Ident "{" RenameMap ("," RenameMap)* [","] "}"
                  | "natural_join" RelId "with" RelId
                  | "project" RelId "keep" "{" [Ident ("," Ident)* [","]] "}"
                  | "semijoin" RelId "with" RelId
                  | "select" RelId "if" "(" RustExpr ")"
                  | "extend" RelId "with" "{" [ColumnDecl ("," ColumnDecl)*] "}"
                    "value" "(" RustExpr ")"
RenameMap        ::= Ident "->" Ident
RelOutput        ::= "output" RelId "as" Atom ";"
```

`r0`, `r1`, etc. name logical set-valued definitions. They are DAG references,
not execution cursors or a requirement to materialize. Every result name is
unique, operands precede consumers, and all definitions must be reachable from
the output. The local language permits acyclic bushy or shared plans; an
individual planning policy can impose a narrower shape.

`unit` denotes `{()}` and has no operand. A declared input is atomic; Rename
maps every source attribute exactly once to distinct query variables.
NaturalJoin equates shared attributes and returns the heading union, including
when every right attribute is already shared. Semijoin keeps exactly the left
heading with an existential matching right row. Project retains a distinct
subset. Select retains its condition; Extend introduces fresh computed columns.
Output preserves the source head and its required dependencies. Ordered output
layout belongs to this metadata, not the set-valued Project operator.

For example, this logical fragment expresses a triangle:

```text
relational {
    r0 = rename R {c0 -> x, c1 -> y};
    r1 = rename S {c0 -> y, c1 -> z};
    r2 = natural_join r0 with r1;
    r3 = rename T {c0 -> z, c1 -> x};
    r4 = natural_join r2 with r3;
    r5 = project r4 keep {x, y, z};
    output r5 as triangle(x, y, z);
}
```

This is a syntax illustration, not a captured expansion.
The written-order CQ policy emits one Rename per occurrence, a left-deep join
fold, a final Project and output metadata. The other planners own different
scheduling rules, including YA reductions. Planning precedes IX and storage
choices. A logical result's multiple uses require an explicit physical reuse or
recomputation decision; they do not make one iterator reusable.

## IX: required indexes and exact uses

```ebnf
IX              ::= RelationalModule
                    "indexes" "{" (IndexDefinition ";")* "}"
                    "uses" "{" (IndexUse ";")* "}"
IndexDefinition ::= Ident "=" "@" RelId "[" [Column ("," Column)* [","]] "]"
IndexUse        ::= RelId "." "right" "=" Ident
```

`Column` is a nonnegative integer ordinal. IX preserves the whole RA payload.
For each NaturalJoin/Semijoin it records the right version and the joined-column
ordinals in that result's sorted logical heading. They are not source-declaration
positions. Identical `(version, columns)` demands share one identity; each
consumer refers explicitly to it. Empty shared headings use an explicit `[]`
demand. No scan fallback follows from an empty list.

The triangle above needs `i0 = @r1[0]` and `i1 = @r3[0,1]`, used by `r2.right`
and `r4.right`. IX validity checks exact demand/use coverage, distinct identities
and descriptors, strictly increasing in-range ordinals, and absence of unused
definitions. Index allocation and data structure selection have not occurred.

## Physical: one physical operator per result

```ebnf
Physical   ::= IX "physical" "{" (Definition ";")* "return" RelId ";" "}"
Definition ::= RelId "=" Mode ["canon"] Operator
Mode       ::= "pipe" | "let" | "alias"
Operator   ::= "unit" | "rename" | "select" | "extend" | "project"
             | "probe" "[" Ident,* "]" RelId "," RelId
             | "merge" "[" Ident,* "]" RelId "," RelId
             | "semi"  "[" Ident,* "]" RelId "," RelId
             | "range" "[" Ident "]"
```

Physical restates every RA result, by name and in RA order, with the
physical form the caller's policy chose: a natural join is a `probe` of the
IX-used side or a `merge` of both sides in key order, a semijoin is `semi`,
a select bounding an ordered host key is a `range`; the other operators keep
their logical name and their detail stays in the RA definition. Join columns
are attribute names. Denotation is IX's set.

What Physical fixes is representation discipline, in linear ANF: a `pipe` is
read exactly once, as the input of the operator consuming it; a `let` is a
value read through a key (probe, semi, range, or a merge not fed by a pipe
already sorted on that key) or read more than once; an `alias` is a host
rename read in place. `canon` marks a result inserted into a canonical index
— the endpoint's distinct sink on a pipe, a materialized set on a let. The
returned result must be canonical, by this flag or by key inference from
`canon` hosts. Order and uniqueness are premises here; Structures discharges
them.

## Structures: what answers each read

```ebnf
Structures ::= Physical "structures" "{" (Entry ";")* "}"
Entry      ::= Ident ["keyed" "[" Ident,* "]" ["ordered"]] ["canon"]
             | Ident "=" RustType "[" Ident,* "]"
```

A host entry states facts about the host value: the key it is stored under
(`Map<(key columns,), Collection<(remaining declared columns,)>>`), whether
that key is ordered, whether the value is a canonical set of its rows. A
built entry names a Rust type — any type implementing `cq_storage::Build` —
constructed over the result and keyed on the listed columns, so
`α(built) = ⟦r⟧`. Nothing is claimed: whether a value implements the trait
each read needs (`IndexRead`, `ExactMembership`, `OrderedRead`, `Canonical`)
is rustc's check at a guard beside the value. The contract requires every
probe/semi/range read to be answered by a host key through the rename or a
built entry on those columns, every merge side ordered on its key (or a pipe
sorted on it by its own merge), and the return canonical.

## Implemented domain and evidence

The [full-chain test](../crates/mini-linq/tests/physical_query.rs) and the
[runnable example](../examples/composition/README.md) exercise every stage:
hosts read in place through their declared keys, compiler-built hash indexes,
a user-defined index type through `#[index(Type)]`, merge joins over built
ordered maps with a sorted pipe side, Yannakakis semijoin passes as membership
tests, a bounded scan of an ordered host, and a computed column. Each pass
also has a `check` over its source and target, and the languages have parser
round-trip tests in `cq-ir`. The logical grammar and planner tests are not
evidence of physical implementation.

```sh
cargo test -p cq-ir
cargo test -p cq-index-policy -p cq-physical-lowering -p cq-structure-selection -p cq-physical-realization
cargo test -p mini-linq --test physical_query
```
