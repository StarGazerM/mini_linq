# MiniLinq languages and compilation boundaries

The common query path is `CQ → NF → VO → RA → IX → Physical → Γ → Rust`. The
[crate map](COMPILER-STRUCTURE.md) identifies each stage's implementation
owner. The syntax below distinguishes language validity from the narrower
domain a caller's policy admits.

Every language exports `Program` and `contract::check(&Program)`. Only VO, IX,
and Structures extend another grammar, respectively CQ, RA and Physical.
All other programs contain their own declarations and output metadata.

## Typed boundaries

| Transformation | Typed owner |
|---|---|
| CQ → NF | `cq_lang_cq::normal::normalize` |
| NF → VO | `cq_order::compile` (heuristic × split) |
| VO → VO | `cq_reduce::compile` (guard pushdown, Yannakakis reducer) |
| VO → RA | `cq_elaborate::compile` |
| RA → IX | `cq_index_policy::compile` |
| IX + policy → Physical | `cq_physical_lowering::compile` |
| Physical + policy → Γ | `cq_structure_selection::compile` |
| Γ → Rust | `cq_physical_realization::compile` |

`mini-linq` supplies procedural entries and transports each typed input/output.
The policy and the distinct-state provider are caller-resolved macro slots
(`physical_policy!`, `select_structures!`, `structures_to_rust!`,
`distinct_state!`). Syntax types and validity checks live in the language crates under `languages/`.

The printer emits a Rust block in a caller-written host. Input identifiers
refer to bindings in that scope. The CQ `struct Name;` is metadata through VO; it does not promise a generated `build/run/query` API.

## CQ and Rust leaves

`RustVisibility`, `RustType`, `RustPat`, and `RustExpr` use Syn's Rust parsers.
Declarations remain in source order; `Program::inputs()` and `Program::output()`
classify them without changing the syntax. Typed `let` retains its annotation
beside its pattern. Rust expressions are parsed without token rewriting.

```ebnf
Program        ::= RustVisibility? "struct" Ident ";" RelationDecl* Query
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
prefer_lazy`, `project = canon`) is the policy's decision, and every operation
it leads the program to perform is a trait bound rustc checks on the host value.

```ebnf
Policy ::= "policy" "{" Host* ["join" "=" ("seek" | "merge") ";"]
           ["semijoin" "=" ("eager" | "prefer_lazy") ";"]
           ["project" "=" ("keep" | "canon") ";"]
           ["intersect" "=" ("first" | "smallest") ";"] "}"
Host   ::= Ident ["keyed" "[" Ident,* "]" ["ordered"]] ["canon"] ";"
```

`project = keep` is the default. `project = canon` flags every projection
whose kept columns do not contain an inferred input key, including atom
prefix projections in Fine valuation plans. A flagged projection read through
a key is a `let` materialized as a set; a single-use scanned projection stays
a `pipe` and deduplicates through the distinct wrapper. Shared results still
use `let`. The returned definition's canonicalization rule is unchanged.

`intersect = first` is the default and preserves the written enumerator.
`intersect = smallest` compares candidate counts at runtime for a local binary
case: a pipe probe introducing one column, immediately consumed by a probe or
semijoin that only checks that column. Both relations must be materialized,
with nonempty prefixes and the new column completing each relation's heading.
Selection extends each demand to `[prefix][new column]`, using a `Trie` for
both enumeration and exact membership. The printer enumerates the smaller
candidate set (ties choose the first) and tests the other. Advised structures,
host-only access, ordered reads, incompatible additional keys, and intervening
operators keep their existing realization. This is a binary intersection
optimization, not a general multi-atom smallest-set implementation.
For generic plans, combine `project = canon` with `intersect = smallest`:
repeated prefix rows otherwise repeat the later candidate work. The fan
regression checks the growth bound with both policies enabled.
`Candidates::keys` enumerates distinct next values; trie counts are exact,
while flat bag indexes report bucket length as an upper bound.

CQ validity and expression scheduling preserve these facts:

- Input names and per-relation column names are unique; input arities are positive.
- Each positive atom names a declared input with the correct positional arity.
- Bare arguments bind variables; repeated arguments impose equality. Computed
  arguments and guards use the preceding lexical environment; `_` is anonymous.
- `let` introduces fresh pattern bindings and retains its complete Rust expression.
- Known logical dependencies must be bound before use. Rust resolves host names,
  type aliases and operations; there is no independent Rust type checker here.
- Expression scheduling is by availability only: normalization moves
  expression arguments out of atoms, and a VO plan places a guard or binding
  once its variables are bound — a guard may move earlier than written, never
  later than its variables — with guards (bindings) in query order among
  themselves. `cq_reduce` pushes onto an atom only a guard whose lexical
  analysis is not opaque and whose names all belong to that atom.

[Rust-expression helpers](RUST-EXPRESSION-HELPERS.md) describe lexical
free-name and pattern-binding analysis.

Column types are operational, not comments. Host maps and built structures are
typed over the declared columns. Trait requirements are local choices: a
hash-keyed read requires Eq/Hash on its key columns, an ordered read requires
Ord, and a distinct provider requires Eq/Hash or Ord on the rows it retains.
There is no universal payload `Clone` requirement or implicit deterministic
sorting step.

## Normal form

CQ-NF is the kernel form every plan erases to. A normal query has one
variable per atom argument, distinct within the atom; constants, repeated
variables, wildcards and expression arguments have been moved out of atoms
into `let` bindings and `if` guards. `cq::normal::normalize` is the
canonicalization and records no decision: a constant's binding goes before
every atom (it is a join key), an equality or expression guard right after
its atom. The grammar is the CQ grammar restricted so that every `Atom` is a
relation applied to distinct variables:

```ebnf
NormalBody ::= (NormalItem ",")* NormalItem
NormalItem ::= Ident "(" (Ident ("," Ident)*)? ")" | "if" RustExpr | "let" RustPat [":" RustType] "=" RustExpr
```

The `Kernel` of a program — its atoms, guards, bindings and head, with
variables renamed by first occurrence — is how every later language is
checked against this one by erasure: two programs agree iff their kernels
agree.

## Valuation plans

VO is a valuation plan: the order in which a query's valuations are built.
A plan is a sequence of steps that extend a partial valuation. An atom step
binds the variables of one **subatom** — an atom restricted to some of its
columns, the rest written `_` — and constrains the already-bound ones; a
guard or binding step is placed once its variables are bound. A subatom names
its source clause (`R@2`) so self-joins stay distinct, and must list every
variable of that clause that is already bound. A **reduction** restricts a
clause's relation before the clause is ever joined: `reduce R@0 with S@1(b, _)`
keeps the tuples of `R@0` that agree with some tuple of `S@1` on the shared
variables (the subatom lists exactly those), `reduce R@0 if (e)` keeps the
tuples satisfying `e`, one of the query's guards over `R@0`'s own variables.
Under set semantics a reduction changes nothing the plan denotes, so the
erasure ignores reductions.

```ebnf
Program   ::= cq::Program "valuation" "{" Step* "}"
Step      ::= Subatom ";" | "if" Expr ";" | "let" Pat [":" Type] "=" Expr ";"
            | "reduce" Ident "@" Int Reduction ";"
Reduction ::= "with" Subatom | "if" Expr
Subatom   ::= Ident "@" Int "(" (Ident | "_"),* ")"
```

```text
valuation {
    reduce R@0 with T@2(a, _); // keep the R tuples whose a occurs in T
    R@0(a, _);            // bind a from R
    T@2(a, _);            // check a against T
    R@0(a, b);            // bind b from R given a
    S@1(b, _);
    if (b != 5);
    S@1(b, c);  T@2(a, c);
}
```

One subatom per atom with all its columns is relation-at-a-time (a binary
join plan); one new variable per subatom is variable-at-a-time (generic
join). Nothing here names an operator, a key or a structure; the algebra
below chooses them.

## Named relational algebra

```ebnf
Program          ::= "relational" "{" RelationDecl* RelationalPlan "}"
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
Output checks its required dependencies and the input heading. The elaboration
pass checks that it preserves the CQ head. Ordered output
layout belongs to this metadata, not the set-valued Project operator.

For example, this logical fragment expresses a triangle:

```text
relational {
    relation R(c0:i32,c1:i32);
    relation S(c0:i32,c1:i32);
    relation T(c0:i32,c1:i32);
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
`cq_elaborate` emits one Rename per clause at its first mention, a join per
full subatom in step order, a Select per guard, an Extend per binding, a
Semijoin or Select per reduction, a final Project and output metadata; the
schedule and the reductions are the earlier passes' decisions. Planning
precedes IX and storage choices. A logical result's multiple uses require an explicit physical reuse or
recomputation decision; they do not make one iterator reusable.

## IX: demands

An IX entry is a demand: the key set a plan applies a result at. The syntax
uses the keyword `indexes`; its entries do not select storage.

```ebnf
Program         ::= ra::Program
                    "indexes" "{" (IndexDefinition ";")* "}"
IndexDefinition ::= Ident "=" "@" RelId "[" [Column ("," Column)* [","]] "]"
```

`Column` is a nonnegative integer ordinal. IX preserves the whole RA payload.
Demand analysis records each NaturalJoin/Semijoin's right version and joined-column
ordinals in that result's sorted logical heading. They are not source-declaration
positions. Identical `(version, columns)` demands share one identity. Physical
derives uses from its operands. Empty shared headings use an explicit `[]`
demand. No scan fallback follows from an empty list.

The triangle above needs `i0 = @r1[0]` and `i1 = @r3[0,1]` for the right operands
of `r2` and `r4`. IX validity checks distinct identities and descriptors, exact
RA result versions, and strictly increasing in-range ordinals. Every demand must
match some RA natural join or semijoin's right operand and join columns; otherwise
it reports ``index `i0` is demanded by no join``. IX permits partial demand sets;
demand-derivation correspondence requires the complete set with canonical names chosen in first-use order.
Physical lowering correspondence checks that each probe/semi's right operand and join columns have a
matching demand. Index allocation and data structure selection have not occurred.

## Physical: one physical operator per result

```ebnf
Program    ::= "physical" "{" RelationDecl* (Definition ";")* "return" RelId "as" Atom ";" "}"
Definition ::= RelId "=" Mode ["canon"] Operator
Mode       ::= "pipe" | "let" | "alias"
Operator   ::= Unit | Rename | Select | Extend | Project
             | "probe" "[" Ident,* "]" RelId "," RelId
             | "merge" "[" Ident,* "]" RelId "," RelId
             | "semi"  "[" Ident,* "]" RelId "," RelId
             | "range" "[" Ident "]" RelId "if" "(" RustExpr ")"
```

Physical restates every RA result, by name and in RA order, with the
physical form the caller's policy chose: a natural join is a `probe` of the
RA right operand or a `merge` of both sides in key order, a semijoin is `semi`,
a select bounding an ordered host key is a `range`; the other operators keep
their full logical operator detail inside Physical. Join columns
are attribute names. Denotation is IX's set.

What Physical fixes is representation discipline, in linear ANF: a `pipe` is
read exactly once, as the input of the operator consuming it; a `let` is a
value read through a key (probe, semi, range, or a merge not fed by a pipe
already sorted on that key) or read more than once; an `alias` is a host
rename read in place. `canon` marks a result inserted into a canonical index
— the endpoint's distinct sink on a pipe, a materialized set on a let. The
returned result must be canonical, by this flag or by key inference from
`canon` hosts. Order and uniqueness are premises here; Γ discharges
them.

## The bindings Γ: what answers each keyed read

The Structures program records the bindings Γ: the value answering each
keyed read.

```ebnf
Program    ::= physical::Program "structures" "{" (Entry ";")* "}"
Entry      ::= Ident ["keyed" "[" Ident,* "]" ["ordered"]] ["canon"]
             | Ident "=" RustType ("[" Ident,* "]")+
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

The [full-chain test](../facade/mini-linq/tests/physical_query.rs) and the
[runnable example](../examples/composition/README.md) exercise every stage:
hosts read in place through their declared keys, compiler-built hash indexes,
a user-defined index type through `#[index(Type)]`, merge joins over built
ordered maps with a sorted pipe side, Yannakakis semijoin passes as membership
tests, a bounded scan of an ordered host, and a computed column. Each pass
also has a `check` over its source and target, and the languages have parser
round-trip tests in the language crates under `languages/`. The logical grammar and planning-pass tests are not
evidence of physical implementation.

```sh
cargo test -p cq-lang-cq
cargo test -p cq-order -p cq-reduce -p cq-elaborate
cargo test -p cq-index-policy -p cq-physical-lowering -p cq-structure-selection -p cq-physical-realization
cargo test -p mini-linq --test physical_query
```
