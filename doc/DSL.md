# MiniLinq languages and compilation boundaries

The common query path is `CQ → RA → IX → Physical → Rust[Access]`. The
[design contract](IX-PHYSICAL-REDESIGN.md) defines the post-IX boundary, and the
[crate map](COMPILER-STRUCTURE.md) identifies its implementation owners.
The syntax below distinguishes language validity from the narrower domain of a
selected physical implementation.

## Typed boundaries

| Transformation | Typed owner |
|---|---|
| CQ → RA | `cq_query_lowering::{written_order,clause_prefix,hypergraph}::compile` |
| RA → IX | `cq_index_policy::compile` |
| IX plus explicit assignments → Physical | `cq_physical_selection::given_bindings` |
| Physical → Rust[Access] | `cq_physical_lowering::{begin,resume}` and selected external macros |
| Existing Access → Pipeline | `cq_access_lowering::to_pipeline::compile` |
| Existing Access → eager Rust set | `cq_access_lowering::to_set_loops::compile` |
| Existing Pipeline → Rust | `cq_pipeline_lowering::{distinct_iter,collect_set,support_iter,exists}::compile` |

`mini-linq` supplies procedural entries and transports each typed input/output.
External providers and RA rules are executable macros. Their callback messages
are administrative syntax inside Physical realization, not a new query stage.
Syntax types and validity checks live in `cq-ir`.

The current realizer emits a Rust expression in a caller-written host. Input
identifiers refer to bindings in that scope. The CQ `struct Name;` is retained
program metadata; it does not promise a generated `build/run/query` API.
Push/pull placement in the redesigned architecture remains open. The existing
Access and Pipeline implementations are independently usable endpoints with
explicit observation contracts.

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

`#[index(Type provides { … })]` on a declaration is storage advice, not
meaning: the query denotes the same set with or without it. `Type` is any
Rust type — from any crate — implementing `cq_storage::Build` and the traits
it claims (`seek`: `IndexRead`, `contains`: `ExactMembership`, `ordered`:
`OrderedRead`); structure selection builds it over whatever the plan indexes
on that relation, and the generated program uses it only through those
traits, so rustc checks the claims. Inference holes stand for the key and
payload tuples (`my::Trie<_, _>`). It names no columns — which columns are
indexed is a planner decision the query cannot see — and it is never a
fallback: a type whose claims do not cover what the plan needs (no `seek`
where a join probes, no `ordered` under `join = merge`) is a compile error at
the demanding definition. Without advice the defaults are `HashIndex` for
joins, `HashSet` for semijoins, `BTreeMap` under `merge`.

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

Column types are operational, not comments. The example providers emit concrete
Rust reference types using the declared columns. Collection and consumer trait
requirements are local choices: hash-map columns require the map's Eq/Hash
operations, sorted-map columns require ordering, and a set consumer requires
Eq/Hash on its result. There is no universal payload `Clone` requirement or
implicit deterministic sorting step.

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

This is a syntax illustration, not a captured complete expansion or a promise
that current physical providers implement its full-column index. Terminal
Project is supported separately; that does not make the entire triangle
physically admitted.
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

## Physical: recorded implementations

```ebnf
Physical ::= IX "physical" "{"
               "relations" "{" (Ident "=" RustPath ";")* "}"
               "indexes" "{" (Ident "=" RustPath ";")* "}"
               "lowerings" "{" (RelId "=" RustPath ";")* "}"
             "}"
```

`RustPath` is a Rust module/name path identifying an external macro. Its spelling
has no semantic interpretation in the common compiler. Relation assignments
cover referenced host inputs and every RA result; index assignments cover IX
identities; lowering assignments cover RA definitions. Selection preserves IX
unchanged and checks complete assignment domains. It does not certify external
representation laws by inspecting names.

The driver invokes the recorded providers/rules through the
[typed protocol](PROVIDER-PROTOCOL.md). It validates reply identity/kind and
region headings while preserving dependency order. A `stored` binding denotes
an established Rust value, an `indexed` binding its exact IX resource, and a
`region` binding unevaluated Access syntax. These administrative records disappear
during realization. A region cannot be treated as iterable storage.

## Access and its independent consumers

```ebnf
RustAccessPlan ::= Output ":-" [Clause ("," Clause)*] ";"
Output         ::= Atom "=>" "(" RustExpr ")"
Clause         ::= BodyItem "=>" RustAccess
RustAccess     ::= "bind" RustPat "<-" "(" RustExpr ")"
                 | "if" "(" RustExpr ")"
```

The local Access contract currently supports positive occurrences, Rust guards,
and `let`; accepting a CQ body form in the parser does not imply an Access
lowering for optional negation/aggregation.

Each logical occurrence is paired with concrete Rust access. Storage types,
index selection and lookup expressions are already supplied. Fresh-variable
occurrences require `bind`, whose pattern binds exactly the fresh variables in
occurrence order. One fresh variable may use a simple identifier; otherwise
an exact tuple pattern is required. A fully bound occurrence uses `if` and
binds nothing. A guard requires already-bound logical dependencies. The head
uses bound variables and carries the selected Rust output expression.

`bind p <- (e)` extends the current logical environment with each item supplied
by `e`; the source may depend on preceding bindings. It specifies neither a
Rust loop nor an iterator adapter. A pull consumer demands the source only when
polled; the eager consumer traverses it while executing the region. Exact-family
Rows/Matches providers return only an iterator expression, its item pattern,
and a logical witness. The selected RA lowering assembles the Access bindings;
an Output provider returns only its witness.

An empty body denotes one empty binding: `out() => (()) :- ;`. Access labels
retain the logical binding structure for checking; they do not re-plan RA or
select storage. Handwritten Access can be broader than the current example
providers' output domain.

The current Physical realizer requires each region's logical head to equal its
RA heading in sorted order. Final output metadata must have that normalized
order too. Its physical carrier is selected independently: the tests include
both a flat reference tuple and a caller-defined nested reference tuple.

## Existing IteratorPipeline

```ebnf
IteratorPipeline ::= IterDefinition* "return" IterId ";"
IterDefinition   ::= IterId "=" IterOperator ";"
IterOperator     ::= "unit" "yield" RustExpr
                   | "scan" RustPat "in" "(" RustExpr ")" "yield" RustExpr
                   | "join" IterId "as" RustPat "with" RustPat
                     "in" "(" RustExpr ")" "yield" RustExpr
                   | "filter" IterId "as" RustPat "if" "(" RustExpr ")"
                   | "project" IterId "as" RustPat "yield" "(" RustExpr ")"
                   | "distinct" IterId
```

Each definition names a lazy stream. Every non-source consumes the immediately
preceding stream; the returned stream is terminal distinct. Binding patterns
and retained tuples make edges explicit; names need not be `iterN`. Projection,
renaming and unused-binding removal can occur before another join/filter.
This language denotes a single-consumer chain, not shared RA relations.

Access-to-Pipeline emits these named definitions, not nested Rust adapters.
The final Pipeline-to-Rust lowering emits one Rust adapter local per definition;
dependent `join` becomes `flat_map` only there. The distinct
iterator endpoint defers source-expression evaluation and distinct-state
construction until first polling, resumes on later polls, and does not drain
remaining rows on drop. Iteration executes when advanced (`next`, `any`, full
consumption); collection is optional. Support-stream, existence and set endpoints
have their own completed-result and observation contracts. `ToTokens` then emits
the resulting Rust AST without performing another semantic transformation.

`access_to_set_loops!` is an existing separate Access implementation that emits
eager loops and a set directly. It is not the same typed edge or observation
contract as Access → Pipeline → lazy iterator. Their independent availability
does not settle push/pull placement in the new architecture.

## Implemented domain and evidence

The current external family implements direct input Rename, indexed binary
NaturalJoin, and terminal Project over borrowed scalar slices and two-column
hash/sorted maps. IX
ordinals are translated through Rename to reuse a physical first-column index.
A prior left Access region can be composed into the next join. Unsupported
operator/storage combinations reject; there is no implicit clone, intermediate
collection, scan fallback, or generated host API.

General materialization, indexes over intermediate regions, semijoin/YA block
access, nonterminal projection, reordered/computed output metadata and broader
operator support remain migration
work. The logical grammar and planner tests are not evidence of their physical
implementation. The [runnable example](../examples/composition/README.md),
[realization tests](../crates/mini-linq/tests/physical_realization.rs), and
[provider variants](../crates/mini-linq/tests/physical_variants.rs) exercise the
implemented domain. The [full-CQ trace](ix-physical/query-trace/review.md) also
covers terminal projection; the selected rule rejects intermediate projection
because dropped logical-variable scope needs separate handling. Authored
expectation fixtures are distinguished from actual
macro captures in those tests.

```sh
cargo test -p cq-ir
cargo test -p cq-index-policy -p cq-physical-selection -p cq-physical-lowering
cargo test -p mini-linq --test physical_realization --test physical_variants --test physical_query --test physical_projection --test physical_pull
cargo test -p mini-linq --test endpoint --test collect_endpoints --test execution_models
```
