# MiniLinq IR formalization

For the broader rule specification and its current proof status, see the
[extended compiler research manuscript](formal/README.md).
It distinguishes mathematical preservation claims from implementation
conformance and records counterexamples to unrestricted backend equivalence.
The [terminology correspondence](formal/TERMINOLOGY.md) follows AHV's
distinction between sorts, named tuples, and valuations. The source spelling
`heading` denotes a sort.

This note states the invariants of the basic compiler as judgments over typed
syntax. It complements the [six complete Triangle
artifacts](DSL.md#complete-triangle-expansion): the artifacts show the whole
query at each boundary; this note explains how the boundaries relate.

For the implemented planner alongside the canonical R1 route, see
[Administrative join planning](ADMINISTRATIVE-JOIN-PLANNING.md). It gives
occurrence-based assembly and safe-projection rules, join-tree extraction,
and a semijoin extension while retaining the basic contracts below. Its
[typed pass composition](PLANNED-PASS-IMPLEMENTATION.md) uses the same IRs,
with separate edge contracts for the alternative algorithms. In the planned
route, the staged file contains one local physical region per named RA result.

The compilation sequence is:

```text
CQ
  -> RelationalPlan
  -> IndexRequirements(RelationalPlan)
  -> staged syn::File[pull! RustAccessPlan]
  -> staged syn::File[iterator_pipeline! IteratorPipeline]
  -> final syn::File[syn::Expr]
  -> tokens
```

There are three student transformations followed by two supplied semantic
lowerings. The last arrow is not a transformation: `ToTokens` is faithful AST
emission.

## Notation

The baseline schema maps every declared relation name to its ordered,
nonempty list of named Rust-typed columns:

\[
\Sigma(R) = \langle a_0:\tau_0, \ldots, a_{n-1}:\tau_{n-1}\rangle,
\qquad \operatorname{arity}(R)=n.
\]

The source spelling is `relation R(a0: T0, ..., an: Tn);`; it declares an
atomic relation input, not a database fact or a logical Scan. The host supplies
the relation instance through the corresponding generated `Program::build` or
`Program::run` input. Generated rows, keys, complements, and results use
always-trailing Rust tuples, so declaration order gives the concrete row type
`(T0, ..., Tn,)`. Logical contracts do not compare `syn::Type` syntax across
attributes: Rust type checking resolves aliases and qualified spellings after
lowering.

An atom is written

\[
A = R(v_0, \ldots, v_{n-1}).
\]

`vars(A)` is its variable set; the argument list retains positional order.
Its complete Rename maps each declared
source attribute `a_i` to positional query attribute `v_i`. A logical result
has an inferred sort `heading(r)`, which is a set of attribute names.
Natural-join equality is inferred from sort intersection; no logical key is
serialized.

All successful baseline inputs are syntactically valid Rust-backed IR values.
We write

\[
\Gamma \models_L x
\]

for an object `x` satisfying language `L`'s local well-formedness contract in
the environment `Γ`. An edge relation records the stronger statement that a
target has exactly the facts a particular source requires.

## Layer 1: CQ

A source program contains declarations and one positive conjunctive query:

```text
H(h0) :- A0, A1, A2;
```

The baseline CQ judgment requires:

1. every declared input relation has positive arity and a unique identifier;
2. each body atom names one declaration and has the correct arity;
3. every baseline atom has the required nonempty list of distinct variables;
4. the head has a positive list of distinct variables; and
5. every head variable occurs in the body.

For source query `q = H(h̄) :- A₀, …, Aₖ;`, the CQ denotes a set:

\[
\llbracket H(\bar{h}) :- A_0, \ldots, A_k \rrbracket_I
=
\{\nu(\bar{h})\mid
\nu:\operatorname{var}(q)\to\mathsf{dom},\quad
I\models A_i[\nu]\text{ for every }i\}.
\]

Here `ν` is a valuation over the query variables, extended to fix constants;
the answer is its action on the head tuple. Named tuples in the RA have
attribute sets as their domains. The course's valuation-relation construction
connects the two by using query variables as attribute names.

Source order is used by the baseline canonical plan, but `CQ` does not itself
contain a NaturalJoin or index choice.

## Layer 2: RelationalPlan

RelationalPlan is named, set-valued relational algebra. Every definition has
only a result ID and an operator:

```text
rN = operator;
```

The result identifier is an explicit DAG edge. Its sort is inferred as an
attribute set; it is not duplicated as an ordered left-hand schema. The
preserved head fixes answer tuple order; the physical lowering chooses an
ordered representation for each intermediate sort. The logical output is:

```text
output rN as H(h0, ..., hk);
```

The baseline operators have these set semantics:

\[
\begin{aligned}
\llbracket\operatorname{unit}\rrbracket_I
  &= \{\langle\rangle\},\\
\llbracket\operatorname{rename}\;R\;\rho\rrbracket_I
  &= \rho(I(R)),\\
\llbracket\operatorname{naturalJoin}\;l\;\operatorname{with}\;r\rrbracket
  &= \llbracket l\rrbracket \bowtie \llbracket r\rrbracket,\\
\llbracket\operatorname{project}\;l\;\operatorname{keep}\;A\rrbracket
  &= \pi_A(\llbracket l\rrbracket).
\end{aligned}
\]

A declared relation name is the atomic input to `Rename`; there is no logical
`Scan` or `Base` operator. Each Rename maps every declared source attribute
exactly once to a distinct query attribute. `NaturalJoin` infers equality from
the intersection of operand sorts and has no written key. `Project` keeps a
set of attributes, so its written order is semantically irrelevant.

`Unit` has no operand and denotes the singleton relation over the empty
sort: one empty binding, not an empty relation. Its concrete syntax is
`rN = unit;`. This identity lets an extended logical pass apply a global
operator before any source attribute has been bound.

The local judgment records more than denotation:

- every result identifier is unique and every operand is an earlier result;
- every Rename names a declared atomic input, maps every declared source
  attribute exactly once, and has distinct targets;
- a Rename sort is its target-attribute set;
- `Unit` has the empty sort;
- a NaturalJoin sort is the union of its operand sorts;
- a Project lists distinct attributes drawn from its input and its sort is
  exactly that attribute set;
- output metadata names a defined result, repeats the preserved source head
  exactly, and its input sort is exactly the head's attribute set; and
- every definition is reachable from the output result.

### Canonical CQ edge

The local RelationalPlan language permits any acyclic dependency graph whose
operands are earlier definitions, including bushy plans and a `Unit` source.
Basic HW1 handles only the positive baseline CQ and therefore never emits
`Unit`; it chooses one canonical, source-ordered left-deep plan.

For each body occurrence `A = R(v_0, ..., v_{n-1})`, where
`Σ(R) = <a_0:T_0, ..., a_{n-1}:T_{n-1}>`:

```text
occurrence = rename R {a0 -> v0, ..., an-1 -> vn-1}
if no prefix exists:
    current = occurrence
else:
    current = natural_join current with occurrence
```

This emits one complete Rename for every occurrence, including a fully bound
one. After the last occurrence:

```text
result = project current keep {head attributes}
output result as the original ordered head.
```

Each Rename and algebra operator receives the next `rN` identifier in
definition order. The exact edge relation requires the target to be
alpha-equal to this construction and to preserve the source `Program`.
Projection order is not compared as syntax because its braces denote a set;
the separate output head owns tuple order.

There is no canonical logical Semijoin. A fully bound right occurrence remains
a NaturalJoin whose union sort equals its left sort. The later physical
lowering observes that the right Rename introduces no fresh attributes and
selects membership rather than enumeration.

## Layer 3: IndexRequirements

An IndexRequirements module is the product

\[
\langle P_{rel}, K\rangle,
\]

where `P_rel` is the complete unchanged RelationalPlan and `K` is an ordered
list of index signatures. It is not a physical command list and does not
replace the set algebra.

For each supported NaturalJoin, resolve its right operand to the direct earlier
Rename

\[
r = \operatorname{rename}\;R\;\{a_i \mapsto v_i\}.
\]

Let the inferred shared sort be

\[
S = \operatorname{sort}(left) \cap \operatorname{sort}(r).
\]

Invert the Rename targets to declared source-column positions:

\[
\operatorname{keyColumns}(R, \rho, S)
=
\langle i\mid \rho(a_i) \in S\rangle.
\]

Columns are enumerated in increasing declaration order, independent of Rename
mapping order or attribute-set iteration order. The canonical key list is the
stable first-use union of all nonempty `(R, keyColumns)` signatures.

The local judgment requires every qualified relation name to be a declared
input, every key column to be in range, and each key to be nonempty and
strictly increasing. Every logical operator remains in the embedded plan;
duplicate index signatures are rejected.

The Basic-HW2 edge requires:

\[
\operatorname{target}.\operatorname{relational\_plan}
=
\operatorname{source}
\]

and

\[
\operatorname{target}.\operatorname{indexes}
=
\operatorname{requirements}(\operatorname{source}).
\]

The basic index and physical backends also require a left-deep dependency
shape with a root Rename and a direct right Rename at each NaturalJoin. They
explicitly reject `Unit`, even though it satisfies the general RelationalPlan
local contract. A locally valid but unsupported plan produces a span-aware
error; its definition is not silently rewritten into the canonical shape.
Optional HW4 extends those two edges with the global-clause rules below.

## Layer 4: staged Rust residual and RustAccessPlan

Basic HW3 creates a concrete `syn::File` containing the complete generated
program. The file already owns:

- public visibility and generated names;
- the exact Rust row type, storage type, and method signatures;
- input normalization and index construction;
- storage access expressions, lookup keys, membership conditions, and output
  tuple construction;
- eager `build`, lazy `query`, sorted `materialize`, and `run` behavior.

Therefore the whole file is not usefully modeled as another physical index
descriptor. It is ordinary Rust code plus one typed local hole:

```text
syn::File[query { pull! { RustAccessPlan } }]
```

The configured direct edge also ends at `Rust[Access]`, consuming RA rather than
IndexRequirements. It keeps layouts, current versions and shared resources in a
compiler-local environment. Its native provider operations can contain existing
Access plans or refine candidate traversal into local block/guard/row Access;
these fragments are not another whole-program stage. Their syntax and laws are
specified in [Access refinement](ACCESS-REFINEMENT.md). The following storage
ownership discussion describes the basic canonical backend specifically.

### Storage ownership

Let `N_i` be the set-normalized `HashSet<Row>` for public input `i`. Let `O_i`
be the set of concrete physical objects needed for that input. The physical
representation pass chooses one final consumer for every nonempty `O_i`:

\[
\operatorname{final}(i) \in \{\operatorname{Rows},
\operatorname{ProperIndex}, \operatorname{FullIndex}\}.
\]

All additional objects borrow `N_i` while creating their owned values. The
final consumer moves the normalized relation. Full replacement indexes are
already the correct `HashSet` and are therefore direct moved values rather
than re-collected copies.

No data operation belongs in the lazy result cursor except query traversal and
result distinctness. In particular, every index is completely built before
`Program::build` returns.

### RustAccessPlan grammar and judgment

Each logical Rename reconstructs its source atom occurrence in declaration
column order and pairs that label with one already chosen Rust effect. The head
comes from separate output metadata and is paired with its exact value:

```text
H(vars) => (result_expr) :-
    A0 => for pattern0 in (source0),
    A1 => for pattern1 in (source1),
    A2 => if (condition2);
```

At each NaturalJoin, let `B` be the current left sort and `A` the sort
of its direct right Rename. `RustAccessPlan` requires:

\[
A \setminus B \ne \varnothing
\Longleftrightarrow
\operatorname{access}(A)=\operatorname{for}.
\]

The `for` pattern binds only the attributes in `A` but not `B`, in declared
source-column order. If the
right Rename contributes no fresh attribute, the access must be a Boolean
membership `if`. Source variables are references to stored tuples; their uses
in lookup keys, membership rows, and the owned output explicitly clone only
the fields needed for owned values.

The checker deliberately does not reverse-check arbitrary Rust expressions to
guess where the storage representation came from. Exact source and output
expression correspondence belongs to the Basic-HW3 structural edge tests.

## Layer 5: IteratorPipeline

The supplied `compile_pull` pass transforms the occurrence-annotated result
cursor into named lazy physical operators. A pipeline is an ordered list of

```text
iterN = Operator;
```

definitions plus a final `return iterN`.

Each operator records the inputs and values that make ownership and binding
flow explicit. The triangle-level data shape is:

```text
iter0: Iterator binding (x, y,)
iter1: Iterator binding (x, y, z,)
iter2: Iterator binding (x, y, z,)
iter3: Iterator item (i32, i32, i32,)
iter4: Iterator item (i32, i32, i32,), distinct
```

The pipeline contract says:

1. definitions are named `iter0`, `iter1`, and so on in order;
2. the first operator is `unit` or `scan`;
3. every later non-source operator names the immediately preceding stream;
4. every input pattern is the complete predecessor binding;
5. `join` binds the fresh item variables and yields the complete new binding;
6. `filter` preserves the binding;
7. exactly one `project` follows all joins and filters;
8. one `distinct` immediately follows projection and is final; and
9. the returned stream is the final distinct result.

The exact RustAccessPlan-to-IteratorPipeline edge requires the ordered
correspondence:

| `RustAccessPlan` operation | `IteratorPipeline` operation |
|---|---|
| first clause is a non-enumerating predicate | synthetic `Unit` source, then `Filter` |
| first `for` source | `Scan` |
| later `for` source | `Join` |
| `if` predicate | `Filter` |
| head output | `Project` |
| CQ set semantics | final `Distinct` |

No source, pattern, condition, or result expression is changed. `compile_pull`
only changes the participating IR and introduces explicit physical edge names.

### Why the pipeline is not yet Rust's `Iterator` itself

The boundary is intentionally one level above the concrete host
representation. It states the physical join order and active binding but does
not yet say whether the Rust spelling uses `flat_map`, a custom join iterator
struct, or another faithful cursor implementation. That selection is the next
semantic lowering.

This separation is what makes a code generator honest. If the final emitter
had to decide join structure or invent intermediate results while printing,
the previous IR would be incomplete.

## Layer 6: concrete Rust operator expression

`compile_iterator_pipeline` maps each operator structurally to one named Rust
local:

\[
\begin{aligned}
\operatorname{Unit}(e) &\mapsto \operatorname{once}(e),\\
\operatorname{Scan}(s,p,e) &\mapsto \operatorname{map}(\operatorname{IntoIterator}(s),p\mapsto e),\\
\operatorname{Join}(l,s,p,e) &\mapsto
  \operatorname{flatMap}(l,\lambda b.\operatorname{map}(s(b),p\mapsto e)),\\
\operatorname{Filter}(l,e) &\mapsto \operatorname{filter}(l,e),\\
\operatorname{Project}(l,e) &\mapsto \operatorname{map}(l,e),\\
\operatorname{Distinct}(l) &\mapsto
  \operatorname{filter}(l,\lambda row.\operatorname{seen.insert}(row)).
\end{aligned}
\]

In Rust, `once` is the allocation-free wrapper that plays the role of
`pure`/`return` for this iterator construction, and `flat_map` is the
`bind`-shaped operation used for parameterized nested loops. Rust does not
expose a general higher-kinded `Monad` or monad-transformer trait in the
standard library; MiniLinq does not need one. The concrete iterator adapters
are the correct host-language representation.

Each source definition is emitted as one `let iterN`. The entire block is
wrapped as

```rust
::core::iter::Iterator::flatten(::core::iter::once_with(move || {
    let iter0 = ::core::iter::once(());
    iter0
}))
```

This minimal valid example shows the wrapper shape. In a complete query, the
block contains every named operator local shown in the sixth Triangle
artifact. `once_with` defers evaluation of the root source expression and
construction of stateful distinctness until the first client `next()`.

The result is a `syn::Expr`. The target representation already specifies Rust
closures, combinators, and local statements. The proc-macro entry is only:

```rust
let target: syn::Expr = cq_compiler::compile_iterator_pipeline(&source)?;
quote::quote!(#target).into()
```

`quote!` here invokes `ToTokens` on the concrete Rust AST. It must not perform
a second transformation or rederive facts from the IteratorPipeline.

## Stream semantics versus set semantics

The logical layers denote sets. Ordinary iterator combinators can temporarily
enumerate more than one derivation for the same projected row. Consequently
`Project` and `Distinct` are separate explicit physical operators:

\[
\operatorname{resultStream}
=
\operatorname{distinct}(\operatorname{project}(\operatorname{bindings})).
\]

The stateful distinct filter owns a `HashSet<ResultRow>`. It yields a result
only on the first successful insertion. This is lazy and compatible with
partial consumption, but it is also a blocking/state-retaining step in the
sense that memory grows with the observed output set.

`Storage::query()` returns that whole distinct result cursor. It is not a
`for` loop over an already-created result stream; it is the result stream
itself. A client can request one row, retain the object, and resume it later.

`Storage::materialize()` is a separate eager contract:

\[
\operatorname{materialize}(S)
=
\operatorname{sort}(\operatorname{collect}(S.query())).
\]

It does not repeat distinctness and it never builds indexes.

## Validation and contracts

Every nominal IR has two small APIs:

```text
contract::check(input)       -> span-aware syn::Result<()>
contract::well_formed(input) -> bool
```

The Boolean form is suitable for pass postconditions and tests. Public typed
compiler entries run the diagnostic source check before lowering and return
`syn::Result`, including the Access and Pipeline entries. Invalid parsed IR
therefore produces the same span-aware error in debug and release builds.
Macro entries parse and route through those checked functions; they need no
duplicate source traversal. Debug postconditions additionally check selected
source-to-target relations on successful compilation.

The exact edge predicates are intentionally different from local checks:

| Edge | Required property |
|---|---|
| CQ -> RelationalPlan | one complete Rename per occurrence, source-order NaturalJoins, final Project, output metadata, and preserved Program |
| RelationalPlan -> IndexRequirements | identical embedded plan plus exact stable first-use keys |
| IndexRequirements -> staged Rust | supported input maps to `Ok`; missing required physical key maps to a controlled `Err`; structural tests inspect the complete file |
| Configured RA -> Rust[Access] | declared source domain, current physical bindings, projected resource identity and emitted Access/source correspondence |
| RustAccessPlan -> IteratorPipeline | exact ordered operator, Rust source, predicate, binding, output, and explicit distinct correspondence |
| IteratorPipeline -> `syn::Expr` | endpoint-specific correspondence: distinct iterator, fused set sink, or explicitly support-preserving stream; rustc and runtime tests establish behavior |

A local checker is not a whole-compiler proof. Physical edge checkers inspect
generated Rust and parse declared nested macro inputs to establish specific
source-to-target obligations. The direct checker audits provider selection,
current bindings, projections and resource uses; it does not validate arbitrary
provider expansion or establish complete Rust adequacy. Rust typing, provider laws
and execution tests supply separate evidence for those remaining obligations.

## Proc-macro routing

The direct public entries are mechanical:

| Macro input | Parsed type | Typed function | Result emitted or routed |
|---|---|---|---|
| `mini_linq!` | `cq::Module` | `compile_cq` | `relational_plan!` |
| `relational_plan!` | `relational_plan::Module` | `compile_relational_plan` | `index_requirements!` |
| `index_requirements!` | `index_requirements::Module` | `compile_index_requirements` | staged `syn::File` |
| `physical_plan!` | RA module plus separate physical options | `compile_physical` | staged `syn::File` with Access/native operation holes |
| `pull!` | `rust_access_plan::Plan` | `compile_pull` | `iterator_pipeline!` |
| `collect_access!` / `stream_access!` | `rust_access_plan::Plan` | `compile_pull` | `collect_pipeline!` / `stream_pipeline!` |
| `iterator_pipeline!` | `iterator_pipeline::Pipeline` | `compile_iterator_pipeline` | ordinary distinct iterator expression |
| `collect_pipeline!` / `stream_pipeline!` | `iterator_pipeline::Pipeline` | `compile_collect_pipeline` / `compile_stream_pipeline` | exact set / support-preserving stream |

The typed compiler functions are reusable without procedural macros, which is
how unit and structural tests exercise them. The proc-macro functions accept
`TokenStream` because Rust requires that ABI at the language boundary; it does
not follow that compiler passes should also accept token streams.

## Optional Homework 4 boundary

Optional HW4 is a cumulative language extension, not an execution-backend
switch. It adds Ascent-style negation and aggregation to `cq::BodyItem` and
extends RelationalPlan with semantic operators such as `AntiSemijoin` and
`AggregateApply`. Basic logical, index, and physical passes are extended at
their declared boundaries; the supplied pull route then continues from the
resulting concrete Rust accesses.

If a negation or aggregation occurs before any positive atom, the extended
logical pass creates one `Unit` as the current left relation. Correlation is
inferred from sort intersection just as for NaturalJoin. A global
AntiSemijoin and global AggregateApply therefore have an empty intersection.
The former may preserve the empty sort of its `Unit` input; the checker must
allow that exact preservation, not make arbitrary empty Project sorts
valid. AggregateApply appends its fresh output, so a global `sum` result has
the one-attribute sort `{total}`.

An empty inferred correlation produces no IndexRequirements entry. The
extended Basic-HW2 pass accepts the `Unit`, walks the two global operators, and
skips their empty intersections. The baseline Basic-HW2 pass continues to reject `Unit` so a
partially implemented HW4 plan cannot silently enter the positive-only
physical route.

Students cumulatively edit the production grammar and passes on that
checkpoint; the `hw4` feature gates only the optional test targets. It does
not reinterpret `mini_linq!`, select pull versus push, or invent a second
query-language entry. Push/Tokio and Rayon are different optional physical
routes and require explicitly named targets and functions when they are
implemented.

## Review checklist

1. Does every logical query fact appear in RelationalPlan before index
   selection?
2. Does every IndexRequirements target preserve the complete plan it indexes?
3. Does Basic HW3 consume that plan instead of reconstructing CQ logic?
4. Are storage types and builders already ordinary Rust in the staged file?
5. Does every RustAccessPlan occurrence carry its exact concrete access?
6. Does every IteratorPipeline operator define one explicit `iterN` result and
   name its predecessor?
7. Are `Project` and `Distinct` separate and visible?
8. Does `compile_iterator_pipeline` contain the only semantic choice of Rust
   iterator combinators?
9. Is token emission a pure traversal of the final `syn::Expr`?
10. Does `query()` return the whole lazy result cursor while `materialize()`
    merely drains and sorts it?
