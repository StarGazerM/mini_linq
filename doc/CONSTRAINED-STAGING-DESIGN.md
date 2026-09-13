# Staged composition and optional constrained planning

Design and implementation boundary, 12 September 2026. Explicit per-binding
composition uses compact contextual inputs and checked typed edges. The compact revision passes the
[recorded validation](CONTEXTUAL-PHYSICAL-LOWERING.md#implementation-and-validation). Automatic constraint-directed planning
below remains proposed. The [contextual lowering design](CONTEXTUAL-PHYSICAL-LOWERING.md)
specifies the selected interface and resolves the earlier
[boundary audit](PHYSICAL-BOUNDARY-AUDIT.md).

The route preserves CQ → RA → IndexRequirements → physical Rust context → Access
→ Pipeline → Rust. Deferred index selection returns the unchanged RA with an
empty overlay; composed realization accepts only that domain. Contextual inputs
contain an existing RA definition and exactly its free physical bindings within
the realization edge. The composer locally decomposes a semijoin into an existing
RA projection and its consuming semijoin. This serialized interface is not another
whole-program relational algebra.
Algorithm control precedes Access lowering; provider invocations embedded in
native leaves may expand later without changing that control structure.

Revised after review of the proposed capability declarations: **explicit macro
composition may delegate representational compatibility to generated Rust
checking.** A universal physical capability catalogue is not required. The
constraint-directed planner below is an optional extension for choices that need
information before expansion commits to an implementation.

A next macro path identifies an implementation. Its actual emitted operations,
types and bindings can establish type compatibility when rustc checks the result.
Formal statements such as `Partition` and `Bounds` below describe obligations in
the paper; they are not a mandated Rust enum or compiler metadata schema.

## 0. Generated-code checking is a valid boundary

For an explicitly chosen route, give each relation binding its own provider and
construction/lowering entry points, compose the chosen implementation, and let
rustc check the actual operations and types. Missing descriptor operations,
incompatible key types and invalid borrowing can fail there. Earlier rejection
can improve diagnostics, but is not required merely to support heterogeneous
storage. A provider's emitted storage operations should target explicit Rust
traits before relying on their query semantics. These are local operation
interfaces with laws, not a universal runtime relation interface: a change of
layout can preserve them while a different access model needs another interface.

Automatic fallback is a different requirement. An ordinary Rust procedural macro
cannot emit one candidate, catch a later rustc type error and resume expansion
with another candidate. Its interface consumes and produces syntax, and type
checking follows expansion. Rust trait resolution can handle type-directed
selection where its rules fit; general plan retries require a planning protocol
or a separate compiler integration/driver.
[Rust Reference: procedural macros](https://doc.rust-lang.org/reference/procedural-macros.html),
[rustc compilation overview](https://rustc-dev-guide.rust-lang.org/overview.html).

Neither successful ordinary type checking nor a bare `HasBounds` declaration
proves that bounds enclose the rows or that metadata access avoids payload reads.
Those are implementation laws, supported by encapsulated constructors, algorithm
reasoning and validation. Formalizing them does not require duplicating the host
type system in a new property language.

For compiler-time selection, prefer an open interface owned by the relevant
transformation: given an actual operation and its bindings, attempt a lowering
or report that it does not apply. Additional symbolic properties are justified
when a particular optimizer needs to combine or propagate them. This preserves
an extension point without requiring every provider to populate a closed list.

## Theoretical basis

Three existing ideas provide a basis for this design:

- Bidirectional typing distinguishes synthesizing available information from
  checking against an expected type. Here, providers synthesize physical facts
  and consumers impose required contracts. This is an architectural adaptation,
  not an application of the survey's metatheorems to our compiler.
  [Dunfield and Krishnaswami, Bidirectional Typing](https://arxiv.org/abs/1908.05839).
- Rule-based query optimization supplies alternative physical realizations and
  explicit property-enforcing operators. Here, permitted sorting, indexing and
  repartitioning are explicit candidate constructions.
  [Graefe, The Cascades Framework for Query Optimization](https://www.sigmod.org/publications/dblp/db/journals/debu/Graefe95a.html).
- Compositional compiler correctness makes the conditions under which generated
  components can be linked part of the correctness statement. Here, a macro
  replacement must preserve its hole's result, resource and observation contracts.
  [Patterson and Ahmed, The Next 700 Compiler Correctness Theorems](https://dbp.io/pubs/2019/ccc/).

The particular combination below is our proposal. No novelty, completeness or
verified-correctness result is claimed for it yet.

Two concrete precedents clarify the implementation boundary:

- C++ `requires` expressions check the validity of types and expressions chosen
  by a library. Certain substitution failures in their immediate context produce
  `false`. This is expression-based checking, not a built-in catalogue of storage
  capabilities; arbitrary errors deep in a template body are not all recoverable
  in this way.
  [C++ draft: requires expressions](https://eel.is/c++draft/expr.prim.req).
- MLIR supplies extensible operation/type/dialect interfaces and conversion
  legality callbacks. Its Transform dialect also supports alternatives with
  scoped rollback and distinguishes recoverable from irrecoverable failures.
  These are precedents for provider-defined legality and fallible composition,
  not evidence that arbitrary transformation semantics can be inferred.
  [MLIR interfaces](https://mlir.llvm.org/docs/Interfaces/),
  [dialect conversion](https://mlir.llvm.org/docs/DialectConversion/),
  [transform alternatives](https://mlir.llvm.org/docs/Dialects/Transform/#transformalternatives-transformalternativesop).

## 1. Holes are obligations

A hole has the form

```text
h = <language, source term, input bindings Γ, required contract Q, constraints U>
```

The notation in sections 1-7 gives the semantic model and a possible planner;
it does not prescribe that every judgment be implemented as a compiler predicate.
For explicit composition, some obligations are checked by rustc and some remain
the correctness obligations of the selected algorithms/providers.

`Γ` contains symbolic identities and established facts about available resources.
`Q` describes the required result and allowed observations. `U` contains scoped
user commitments and construction permissions. The values of the input rows are
not available during macro expansion.

A relation version may have several physical witnesses:

```text
Rep(R4, storage A, selection σ4, layout λ)
Ordered(A, [k], comparator c)
ExactKeys(I, R4, [k], equality e)
Partition(A, P)
Bounds(Z, P, [k], order c)
```

These declarations do not require a runtime object bundling all those resources.
Primary storage, selections, auxiliary indexes and summaries may have different
Rust types and lifetimes. Input R can use sorted rows, S hash groups, and T block
storage. Derived versions receive their own bindings and justified properties.

The distinction between identities matters. Block bounds describe a partition of
immutable storage, so a narrower selection can reuse them. An exact key resource
must denote the current selected key set. An older larger key set cannot satisfy
that exact demand merely because it came from the same base relation.

## 2. Contracts include denotation and access protocol

A contract has four parts:

```text
Q = <result denotation, resource/provenance obligations,
     access/effect protocol, ownership and host-binding obligations>
```

Examples of denotations are exact rows of a named RA result, exact projected keys,
or candidates enclosing a specified exact result. Equality and containment are
different contracts. Candidate rows cannot escape as an exact RA result without
an exact residual or a checked later completion.

The zone access protocol additionally requires that descriptors can be examined
without opening their payload, rejection precedes opening, and opening enumerates
only current selected rows in that descriptor. A program returning the correct
set after scanning all payload satisfies the denotation but fails this protocol.
Provider operations themselves must satisfy the declared access effects; placing
a method named `metadata()` before `open()` proves nothing about its implementation.

The scope of effects must be explicit: avoiding query-time opens does not imply
avoiding ingestion reads. Constructing metadata by scanning an existing hash
input cannot retroactively save that read.

## 3. Realization rules replace untyped dispatch

In the optional planner, a registered realization consists of

```text
rule ρ = <goal pattern, prerequisites, child holes,
          result/resource guarantees, lowering implementation>
```

Its logical shape is

```text
Γ entails prerequisites(ρ)
Γ; U proves child h1 by p1, ..., child hn by pn
composition of ρ with these results entails Q
------------------------------------------------------------
Γ; U proves h : Q by Cρ[p1, ..., pn]
```

Prerequisites can themselves be construction goals. A permitted builder produces
a fresh resource with a declared guarantee; it does not simply assert that the
old representation acquired a new property. No global test such as
`if data_structure == HashMap { reject_zone() }` is required.

One exact zone-semijoin rule requires:

```text
Rep(L, A, σ, λ)
Partition(A, P) with descriptor enumeration and selected-row opening
Bounds(Z, P, left key projection)
KeySummary(M) conservatively covers keys(current R, right projection)
ExactKeys(I, current R, right projection)
paired projections have compatible equality/order meanings
```

It produces this access computation:

```text
enumerate current descriptors
  -> reject using Z and M
  -> open accepted descriptors under σ
  -> test complete keys with I
  -> retain handles from σ
```

For `E = L semijoin R`, the pruning phase establishes `E ⊆ candidates ⊆ L`;
exact probing then establishes equality with `E`. A normal hash-row provider
supplies no partition/open/bounds witnesses, so this rule cannot be derived.
A future provider using a hash map to index block descriptors is a different
case: applicability depends on its declared operations and laws, not its name.

Conservative reuse may be derived from inclusion: a summary covering an older
superset remains a sound filter for a newer subset. It still cannot replace the
exact current membership witness. Construction rules establish this evidence;
the compiler does not test arbitrary runtime set inclusion during expansion.

## 4. User choices constrain search

Constraints attach to a specific input, result version, operator site or region:

```text
require primary storage(R0) = SortedRows(k, c)
permit auxiliary indexes over R0
forbid copying/repartitioning R0
prefer merge implementation at join7
```

Hard requirements determine legality. Preferences and cost estimates rank legal
alternatives. A storage pin does not automatically ban auxiliary indexes; the
latter permission is separate. An input-only pin does not silently constrain all
future projected/joined results to the same representation.

Sortedness on k does not imply every access operation. A particular provider may
supply equal-range lookup, an ordered scan, or both. Zone pruning still needs
partition and metadata witnesses. If a forced zone route lacks those witnesses
and permitted construction rules cannot produce them, that planner reports a
physical-planning error. With an explicitly wired route, missing operations can
instead be diagnosed when rustc checks the generated code. A preferred zone route
can lose to another legal realization when a selection mechanism is provided.

Claims about existing data also differ from choices about construction. A
constructor that sorts establishes order. A claim that external data is already
sorted needs validation or an explicit trusted input contract.

## 5. Search and scheduling are different operations

Search forms an AND/OR graph: applicable rules are alternatives, and each rule's
prerequisites/child holes are dependencies. Facts and choices for different
alternatives remain branch-local. A speculative branch cannot publish an
unproved guarantee into the shared environment.

A minimal algorithm is:

```text
solve(h, Γ, U):
    identify the obligation by term, contract, resource identities and constraints
    reuse a compatible checked derivation if available
    consider permitted realization rules matching h
    for each candidate in its own constraint environment:
        resolve its prerequisites and permitted resource constructions
        recursively solve the child holes
        check result refinement, provenance, effects and dependencies
        retain the resulting checked derivation
    select among retained derivations using preferences/cost
    otherwise report the failed obligations (or a search-limit result)
```

Demands flow from consumers to still-uncommitted producer choices; available
contracts flow back to consumers. A user can fix a choice to a single candidate.
This is the same mechanism whether the decision originates in a user directive
or a compiler heuristic.

An unresolved resource or implementation choice is a planner variable, not an
already-emitted Rust expression. A hole can suspend on a missing contract;
contract discovery or a checked construction wakes its dependents. Requirements
from all consumers of a shared resource constrain the same branch-local choice.
Additional physical witnesses can satisfy different consumers when permitted.
Propagation alone does not choose between alternatives: incompatible branches
still require rejection or backtracking.

This negotiation is scoped to the boundary that owns the relevant choices. It
does not require one planner to predict every later compiler transformation.
Description/elaboration steps may expose new obligations before commitment;
implementation expansion must respect the contract committed at its boundary.

After a derivation is selected, construct two distinct dependency relations:

1. **Expansion dependencies:** contracts/bindings needed before a compiler hole
   can be elaborated or its chosen lowering expanded.
2. **Generated execution dependencies:** construction before use, publication
   before probing, guard before block opening, and required lifetime/effect order.

A legal macro expansion order does not automatically establish the runtime
order. Both must be represented and checked. Independent expansions can be
scheduled freely only when their compile-time contracts/effects permit it.

Memoization applies to the declarative planner, not arbitrary effectful macro
execution. Resource sharing also requires a reusable immutable-resource contract;
constructor side effects are not silently declared irrelevant.

## 6. Macro continuations carry implementation choices

Each independently replaceable semantic boundary can expose a continuation.
A chosen rule can create several children with different implementations, then
compose their compatible results. There is no universal single next-stage chain.
Ordinary internal helper functions need not become macros.

When a planner needs facts to choose a route, discovery of those facts must
precede that choice. A Rust path or macro alias alone contains no compiler-readable
laws. Providers can supply the relevant information through a description macro,
a fallible lowering attempt, or the planner's input. Explicitly selected routes
can instead emit an opaque downstream call and delegate its type compatibility
to rustc. This is not evidence that the macro expander itself knows those facts
or can retry after a subsequent type error.

Once a contract is fixed, substituting an implementation is allowed only on its
admitted domain and under the same observation/ownership requirements. Pull and
push lowerings need not share runtime cursor types, but replacing an iterator
hole with an eager callback is not automatically compatible. Replace the larger
owning context when those result protocols differ.

## 7. The claims we should prove

The desired soundness statement is conditional:

> If input and provider contracts hold, every selected rule satisfies its stated
> refinement, and resource/context composition checks succeed, the generated
> program satisfies the root hole's denotation, access and ownership contract.

The proof should proceed by induction on the selected derivation DAG, accounting
for shared resources and their dependencies. Rust elaboration/hygiene and native
operation adequacy remain necessary premises until separately established.

For termination, start with a finite admitted rule-instance space, memoize
obligations, bound or forbid conversion cycles, and require terminating lowerings.
Unrestricted metaprogramming is not covered. A search limit means `search
exhausted`, not a proof that no possible Rust implementation exists. Lack of a
rule in the admitted system means `no supported realization under these
constraints`. Alternative valid derivations need not have identical layouts,
traces or costs; global optimality and confluence are not assumed.

Provider implementations and any semantic declarations are a trust boundary.
An optional symbolic planner can check composition using indexed facts and
constructor laws; a fallible-lowering interface can delegate applicability to
provider code. Neither approach infers or proves arbitrary Rust semantics.
Providers require validated/proved laws, with any trusted assumptions explicit.

## 8. Consequence for this repository

`physical::composed` now records a provider macro path alongside each generated
binding and field layout. `PhysicalOptions` supplies per-input providers,
materialization defaults and overrides, semijoin recipes, key-collection families,
and whole join recipes. These paths identify implementations; they do not encode
the optional symbolic obligations above.

The implemented explicit composition makes three changes:

1. `composed::api` delegates each parameter type to its input provider. Native
   hash and sorted sets can coexist with `cq_storage::BlockStore`.
2. Unit, Project and NaturalJoin delegate construction to their result provider;
   Rename and Semijoin inherit the relevant source provider. Block materialization
   is a scoped choice rather than a consequence for every relation.
3. `physical::providers` separates native and block operations, while
   `physical::semijoin` and `physical::joins` expand selected algorithms.
   Their emitted Access regions use existing lower languages;
   rustc checks the completed operations, types and borrowing.

The compiler consumes `cq-ir::physical_context::Context` and
`physical_provider::Operation`, separately from ordinary compiler options.
A context is `Definition { Binding* }`, where a binding is
`Atom [=> Provider] { Expr };`. Row bindings supply a configured provider;
projected-resource bindings omit it and the selected recipe interprets their
representation. Local contracts reject missing, extra or repeated operands and
derive key positions and canonical output layout. Configuration paths, work
expressions, phase suffixes and copied resource-origin annotations are absent.

For `K = heading(L) ∩ heading(R)`, the composer emits `q = project R keep K`
and `L semijoin q`. A key constructor receives only its projected source; a
reduction receives only the left rows and `q`. The whole-source checker establishes
that the actual dominating resource denotes this projection of the current right
version. This checks the algebraic decomposition instead of repeating an unchecked
origin claim in every invocation.

`compile_deferred_indexes` and `compile_composed` provide the checked empty-overlay
route. `physical_plan!` forwards configured IndexRequirements through relative
`mini_linq::composed!`, leaving the physical edge independently replaceable.
Lexical wrappers capture storage, access continuation, key family and optional
instrumentation once per selection. Frontend administrative prefixes are parsed
outside `cq-ir`. `run` emits no instrumentation; `run_with_stats` instantiates the
same semantic calls with counters. No runtime registry participates in selection.
The source-to-invocation checker establishes binding and wrapper correspondence
from the complete source; parsing or successful Rust typing cannot prove their
semantic laws.

The storage boundary now has operation traits with explicit laws.
`BlockRead` and `BlockDescriptor` separate directory traversal and payload opening;
associated row, address, identity and borrowed descriptor types hide layout.
Optional `CoordinateBounds` requires enclosing metadata without payload opening.
`BlockSelection` represents an exact coordinate set, while `MakeSelection<S>`
lets callers choose several masks for one store. `BuildBlocks` governs explicit
construction. `ExactMembership<K>` requires exact lookup; `RangeSummary<K>`
governs conservative rejection and cannot replace that lookup. These interfaces
are in `cq-storage`, alongside their concrete implementations and passive counters.
No trait executes a relational operator or chooses an access path.
Their surface follows semantic needs: block traversal requires no random lookup
or total-count operation, and address types require only copying. Equality and
hashing constraints needed by a selected collection remain at that endpoint.
The supplied types implement query operations directly in traits; public inherent
twins are removed. Ingestion stays public, while packing and mask-layout helpers
remain private or crate-private.

Provider expansion associates storage with a selection and owns filtered access,
fresh selections and provenance checks. Handles carry associated store identity
separately from payload addresses, including for zero-column records; descriptors
verify their parent before opening payload. Recipes must preserve all fields of
the provider's handles when filtering, reordering or deduplicating them; arbitrary
fabrication is outside the protocol and is not checked by a second payload lookup.
A native provider asked for block
operations can reject during expansion without a capability-tag catalogue.
Layout changes satisfying the same laws affect storage implementations and caller
type aliases; the composer and algorithm recipes can remain unchanged. New access
semantics can instead justify new providers or recipes. Rust checks structural
compatibility, while metadata-only access, bounds enclosure and exactness remain
implementation obligations.

Resource sharing is deliberately scoped to equal current RHS version, ordered
separator, recipe path and key-collection path. Different recipes can construct
different resource shapes, so exact and zone requests do not automatically share
one combined build. Sharing additionally requires uniform consumability across
all contexts in that class, including resource shape and summary meaning. Exact
keys and repeated reads alone do not establish resource-shape compatibility.
The compact constructor interface no longer exposes another left operand or
consumer result property; arbitrary replacement macros still owe the stated
resource and lifetime laws.
The composition checker validates the invocation graph and
its versions, fields and paths. It does not prove arbitrary macro expansions or
infer their semantic laws.

A shared whole-program physical plan language remains only a possible later
design for cross-provider search and property propagation. The present extension
has a narrower necessity: macro boundaries require serialized inputs, existing RA
provides the operator, and its physical context supplies the bindings absent from
RA. Access already commits to concrete control and is too late to recover a whole
join choice. Therefore the declared input retains RA under those assumptions;
it does not copy the join into a second operator algebra. Its denotation, local
contracts, CP rules and position are specified in the
[contextual lowering design](CONTEXTUAL-PHYSICAL-LOWERING.md). Additional languages
would still need their own necessity argument and preservation obligations.

The [acceptance cases](../crates/mini-linq/tests/heterogeneous.rs) instantiate
blocked, hash and sorted inputs, alternate joins, custom provider macros and
explicit intermediate block construction. Negative composition cases request
zone operations from a native provider. Current-version exact membership,
conservative bounds reuse and guard-before-payload order require algorithm and
provider evidence beyond successful Rust compilation. The
[implementation guide](BLOCK-ACCESS-IMPLEMENTATION.md) records the validation
commands and their bounded scope. If automatic planning is added, extend this
witness to recoverable lowering attempts and selection of permitted constructions.
