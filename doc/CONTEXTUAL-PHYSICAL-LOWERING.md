# Compact physical lowering within the existing IR order

Design and implementation contract, 12 September 2026. This supersedes the
configuration-bearing contexts recorded by the [historical audit](PHYSICAL-BOUNDARY-AUDIT.md).
The source of an operator lowering contains existing RA and only its free binding
assumptions. Macro configuration is separate. No additional relational algebra
or whole-program IR is introduced.

## Placement and necessity

The composed route refines two edges in the existing graph:

```text
RelationalPlan --defer auxiliary construction--> IndexRequirements
IndexRequirements --composed realization--> Rust[operator holes]
  --operator realization--> Rust[Access, native provider fragments]
  --Access lowering--> Rust[Pipeline] --pipeline lowering--> Rust
```

Contextual operator inputs are serialized inputs to deferred physical lowering,
not a lower relational algebra. Existing RA identifies the computation; free
bindings identify the actual Rust values and their physical layouts. Access is
already committed to its concrete leaves, so it cannot recover a different whole
join algorithm. The operator recipe chooses that control before Access lowering.
Provider macros inside native Rust leaves can expand later in their Rust scope.

IndexRequirements lists promised named indexes. Its local contract permits the
empty overlay. `Defer(D) = (D, ∅)` preserves every source definition;
`compile_composed` accepts only well-formed `(D, ∅)` and rejects every written
index requirement. Canonical indexed backends retain their own coverage contracts.
The composer owns local auxiliary scheduling for its selected recipes; it neither
ignores demanded indexes nor forces an unused hash index into a nested-loop plan.
A future optimizer manipulating unresolved resource schedules needs its own
necessity argument and checked edge.

## Minimal source and an existing algebraic decomposition

The grammar belongs to `cq-ir`. `Definition` is the existing RA definition and
`Atom` is an existing CQ atom restricted to distinct bare variables. Rust
expressions and paths use `syn`; punctuation is preserved by derived parsing and
printing.

```text
Context ::= Definition { Binding* }
Binding ::= Atom [=> Path] { Expr } ;
```

A provider path makes a row representation explicit. An omitted provider binds a
projected relation represented by a resource whose shape the selected recipe owns.
Neither form changes the named relation's denotation. Braces following a provider
permit ordinary `$provider:path { $value:expr }` forwarding patterns.

For `K = heading(L) ∩ heading(R)`, existing relational algebra gives

```text
L semijoin R = L semijoin project_K(R).
```

The composer therefore emits a projection and a semijoin, using fresh logical
resource names. For example, the semantic arguments to two configured macros are:

```rust
q = project r keep { k };
{
    r(y,k) => right_rows { right_value };
}

result = semijoin l with q;
{
    l(k,x) => left_rows { left_value };
    q(k) { key_resource };
}
```

The projection receives only `r`; the reduction receives only `l` and `q`.
There are no `build`/`reduce` phase suffixes, unused opposite operands, or repeated
resource-origin assertions. Crucially, `key_resource` denotes `q`, the projected
relation, rather than pretending to denote all of `r`. The complete producer
checker establishes its construction, source version, projection and dominance.
The local syntax contract does not infer those facts from an arbitrary Rust value.

This is a checked local decomposition during physical realization. The preceding
RA-to-IndexRequirements edge still preserves the original RA exactly. The physical
edge checks its synthesized projection and rewritten semijoin against that source;
it does not claim that the rewritten operator is textually unchanged.

The binding domain is exactly the definition's distinct operands. Duplicate,
missing or extra operands, repeated/non-variable layout arguments and result-name
capture are rejected. Natural joins require two row representations; a self-join
requires one binding. A key projection requires its one row representation. A
reduction requires left rows and a canonically ordered projected-key resource.
Empty headings retain the distinction between no keys and one empty key.

Positions are derived, never copied into the input. Given row layouts `ell_l`
and `ell_r`, derive `kappa = sort(V_l ∩ V_r)` and the positions
`ell_l^-1(kappa), ell_r^-1(kappa)`. A natural join emits `sort(V_l ∪ V_r)`;
a semijoin preserves its left layout. Projection positions follow its existing
`keep` list and input layout. Every used operand expression is evaluated once per
operand use; a self-join uses its one bound expression twice.

## Configuration and instrumentation are outside the source

`PhysicalOptions` selects providers per relation and recipes per operator.
`composed::wiring` partially applies dependency paths, continuation, key family
and optional instrumentation in ordinary lexical `macro_rules!` wrappers. One
wrapper is emitted per distinct selection in each generated method. Operator
calls pass only the semantic context; provider calls pass only an operation.
There is no global registry, path classification or implicit storage default.

Direct public macro entry points accept an administrative prefix:

```text
Provider entry: storage Path ; [work (Expr) ;] Operation
Recipe entry:   storage Path ; access Path ; [keys Path ;] [work (Expr) ;] Context
```

The prefix parser accepts any directive order and rejects duplicates; provider
entries reject recipe-only directives. It lives in `mini-linq::recipe_input`, not
in `cq-ir`. Compiler functions receive `&Context` / `&Operation` and ordinary
`RecipeOptions` / `ProviderOptions` separately. `keys` is needed for construction,
not for a membership resource already supplied by a reduction.

`run` expands an uninstrumented body. It allocates no counters, requests no
per-result cardinalities for statistics and emits no counter updates or inspection
adapters. `run_with_stats` instantiates the same semantic calls with instrumentation
and records the observations. Used instrumentation expressions are evaluated once
at their invocation boundary; unused operations do not evaluate them. Scope and
borrowing remain ordinary Rust obligations.

## Contextual denotation and rules

Let `Gamma(v) = (P,e,ell)` assert that provider `P` decodes the selected rows of
Rust value `e`, in layout `ell`, to exactly `D(v)`. Let `Xi(q) = (e,kappa)` assert
that the selected recipe interprets `e` as exact membership in `D(q)`, with any
additional lawful fields. For a constructed key resource the producer establishes
`D(q) = project_kappa D(r)`. These equations are semantic premises, not consequences
of merely parsing tokens or implementing a Rust trait.

```text
Gamma ; Xi |- Context ==> Rust[Access] : Rep(result)
```

| Rule | Premises | Algorithm and conclusion |
|---|---|---|
| CP-Defer | `RA-WF(D)` | Return `(D,∅)` with unchanged RA and valid IndexRequirements |
| CP-Realize | `IX-WF(D,I)`, `I=∅`, checked local decompositions and bindings | Emit Rust representing every original named result |
| CP-Bind | Correct represented value, layout and dominating binding | Introduce a row or projected-resource assumption |
| CP-Build | Existing Project definition, current input and valid projection | Realize its exact projected relation, possibly by borrowing; zone construction also supplies a conservative summary |
| CP-Reuse | Same source version, projection, recipe and key family; lawful immutable repeated reads and lifetime | Reuse the established projected relation |
| CP-Exact | Semijoin with an exact projected-resource assumption | Enumerate left handles, test membership and select survivors |
| CP-Zone | CP-Exact premises; complete descriptors and sound metadata rejection | Guard descriptors before opening payload, apply the exact residual and select survivors |
| CP-Hash | NaturalJoin and both row assumptions | Build right witness groups, probe from left rows and emit canonical joined rows |
| CP-Nested | NaturalJoin and both row assumptions | Enumerate pairs, test derived shared fields and emit canonical joined rows |
| CP-Store | Distinct rows denote the result and selected construction preserves their set | Establish a fresh row representation |

Resource sharing remains scoped to `(current right version, ordered projection,
recipe path, key-family path)`. Syntactic path equality does not prove alias identity
or a replacement recipe's laws. Construction receives no left consumer or result
context; this removes that dependency from its interface. The resource must still
have uniform meaning and compatible shape at every use in its sharing class.
Exact and zone recipes are not automatically combined or shared across selections.

For a descriptor's selected rows `B`, sound rejection requires
`(exists x in B: x|kappa in D(q)) => mayMatch(q,B)`.
Both exact and zone reductions then denote
`{x in D(l) | x|kappa in D(q)}`; the projection equation establishes preservation
of the original semijoin. Hash and nested-loop implementations both denote the
natural join. Induction over source results and resource constructions, combined
with checked Access/Pipeline lowering, gives conditional answer preservation.
It does not prove arbitrary replacement macro bodies or Rust operations.

Zone pruning additionally requires that each rejected descriptor's payload remain
unopened by that consumer. Enumeration and bounds lookup themselves must avoid
payload access. Another consumer or key constructor may legitimately open the
same storage. This trace property is separate from answer equality.

## Provider operation fragments

Provider operations are typed fragment inputs, not a relational algebra or a
universal capability catalogue. Each selected implementation can reject operations
it cannot realize.

```text
Operation ::= type Column | input (Expr) | len (Expr) | row (Expr)
            | scan (Expr) | rows (Expr) | select (Expr) (Expr)
            | materialize Ident Ident (Expr) (Expr)
            | blocks (Expr) | bounds (Expr) | block_rows (Expr) (Expr)
```

`type` emits a Rust type; `materialize` emits owner/value bindings; the others emit
expressions. `scan` returns current rows and `rows` returns valid current handles.
`row` decodes a handle and `select` preserves exactly supplied current handles.
`blocks` and `bounds` inspect metadata; `block_rows` opens only the identified
payload and restricts it by the current selection. It needs both descriptor and
current value to check owner identity and the current mask. Type widths use the
existing unsuffixed `Column` syntax.

Handle identity, coordinates and row reference stay together. Storage internals
remain behind operation traits. Native materialization does not evaluate the
block-size expression needed by the block implementation; the common construction
interface still admits that family-specific parameter. No operation carries
instrumentation, a runtime relation wrapper or an execution-policy tag.

## Implementation and validation

Syntax and local contracts live in
[`physical_context.rs`](../crates/cq-ir/src/physical_context.rs) and
[`physical_provider.rs`](../crates/cq-ir/src/physical_provider.rs).
[`passes.rs`](../crates/cq-compiler/src/passes.rs) exposes the checked stage edges;
[`pass_contracts::composed`](../crates/cq-compiler/src/pass_contracts/composed.rs)
and [`composed::check`](../crates/cq-compiler/src/physical/composed/check.rs)
independently check domain preservation and source-to-invocation correspondence.
The [paper](formal/block_implementation.tex) and [rule inventory](formal/rules.tsv)
map the conditional rules to their implementation.

Validation: 398 workspace tests, 37 focused release integration tests, and
16 compiler boundary cases in both debug and release pass. Workspace Clippy
with warnings denied, formatting, all five formal example drivers and both
manuscript builds pass. The main manuscript remains 32 pages and the physical
supplement 21. The regenerated fact-star instantiation matches all 26 golden
rows in both methods of both exact and mixed-zone routes; its composed input
uses the explicitly documented i64 widening. Expanded normal methods contain
no counters or deferred operator/provider calls. Logs are `target/compact-*.log`.
The [complete IR/stage review](IR-STAGE-REVIEW.md) records remaining interface
and Pipeline compactness debt. Acceptance includes malformed contexts, stale/mismatched resource use,
lexical alias scope, source-preserving index selection, rejection of written
index requirements, borrowed key construction, mixed-store exact/zone/hash/nested
answers, optional instrumentation and observed guard-before-payload behavior.
These checks establish bounded behavior and structural obligations, not a proof
of arbitrary replacement macros.
