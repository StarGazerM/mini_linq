# Compiler ownership and composition

The accepted path is `CQ → NF → VO → RA → IX → Physical → Γ → Rust`.
The pipeline wire format concatenates sections. Each language owns one section;
its contract reads earlier facts through an `Env` trait. IX records demands:
the key sets a plan applies results at. The `structures` section records the
bindings Γ: the value answering each keyed read. A printer consumes Γ and
emits one pull program over the `cq-storage` traits.
See [DSL.md](DSL.md) for the grammars and [SEMANTICS.md](SEMANTICS.md) for
what each section denotes.

## Crates and modules

```text
languages/  shared syntax and language sections, contracts, optional reference prefixes
passes/     typed transformations and shared printer helpers
runtime/    storage traits, state protocol, and distinct-state providers
facade/     library stage ABI and reference procedural macros
```

| Crate | Responsibility | Modules / entry functions |
|---|---|---|
| `languages/cq-syntax` | Shared syntax fragments, input declarations, list aliases, Rust expression helpers, predicates, `Staged<Prefix, Section>` | `rust`, `predicate` |
| `languages/cq-lang-cq` | CQ contract and normalization | `Module`, `normal`, `hypergraph` |
| `languages/cq-lang-vo` | Valuation section and source facts | `Section`, `CqEnv`, `contract` |
| `languages/cq-lang-ra` | RA section, headings and reference erasure | `Section`, `CqEnv`, `contract`, `kernel` |
| `languages/cq-lang-ix` | Demands on exact RA results | `Section`, `RaEnv`, `Join`, `contract` |
| `languages/cq-lang-physical` | Physical operators and read discipline | `Section`, `IxEnv`, `LogicalOp`, `Analysis`, `policy` |
| `languages/cq-lang-structures` | The bindings Γ answering keyed reads with host or built values | `Section`, `PhysicalEnv`, `PhysicalFacts`, `UniqueOp`, `Analysis` |
| `passes/cq-order` | NF → VO; a `Heuristic` (`Written`, `Prefix(Ranking)`, `Elimination`) fixes the valuation order, a `Split` (`Coarse`, `Fine`) the granularity | `compile`, `check` |
| `passes/cq-reduce` | VO → VO; `Pushdown::Atoms` pushes single-atom guards onto their atoms, `Reductions::Tree` is the Yannakakis full reducer over the GYO join tree (acyclic queries) | `compile`, `check` |
| `passes/cq-elaborate` | VO → RA; rename per clause at first mention, join/semijoin per subatom, select/extend per guard/binding, semijoin/select per reduction, one final projection | `compile`, `check` |
| `passes/cq-index-policy` | RA → IX; derive join-column demands | `compile`; `correspondence` checks the edge |
| `passes/cq-physical-lowering` | IX + policy → Physical; one physical operator per result, modes from use, `canon` on the return when uniqueness is not inferred | `compile`, `check` |
| `passes/cq-structure-selection` | Physical + policy → Γ; host facts from the policy, a built type for every read no host key answers, `#[index(Type)]` advice | `compile`, `check` |
| `passes/cq-print` | Shared row layouts, access and expression printing for Γ | `Context::new`, `access`, `expressions` |
| `passes/cq-physical-realization` | Γ → Rust; builds the chosen values, guards the traits each read needs, prints pull control flow | `compile` |
| `runtime/cq-protocol` | Independent state-request token grammar and laws | `state_protocol` |
| `runtime/cq-state-providers` | Distinct-state providers behind `state_protocol` | `hash_distinct`, `ordered_distinct` |
| `runtime/cq-storage` | Runtime data structures and operation traits | `Build`, `IndexRead`, `ExactMembership`, `OrderedRead`/`merge`/`sorted_by`, `Canonical`, `require` guards; `HashIndex` |
| `facade/cq-stage` | Library stage ABI over typed inputs, outputs and continuations | `lib.rs` |
| `facade/mini-linq` | Procedural macro facade and caller-resolved continuations | `lib.rs` converts the proc-macro ABI; `workload` captures catalog CQ |

`mini-linq-course`, `cq-workloads` and the separate `xtask` workspace are
consumers, not compilation stages.

## Sections, environments and reference composition

Language cores depend only on `cq-syntax` within the workspace. Each `Section`
owns its local grammar; later languages use `contract::check(&section, &env)`
to check it against facts supplied by a producer. CQ is the root: its `Section`
aliases its complete `Module`, its contract checks that module directly, and
`()` implements its empty `Env`. Shared syntax contains
`Staged<Prefix, Section>`, input declarations (`RelationDecl`, `ColumnDecl`),
and shared fragments (`Atom`, `Filter`, `Let`, `TypeAnnotation`), without
owning a complete CQ program or language contract.

Contract-only producer traits (`CqEnv`, `RaEnv`, `IxEnv`, `PhysicalEnv`) stay
small. Where a pass needs more facts, its source language extends them through
an `Env` that never names an upstream language type:

| Consumer | Source facts read beyond its section |
|---|---|
| CQ order | None: the root section owns schemas and query; the pass supplies VO's Env |
| VO reduction | Clauses for validation; the source GYO join tree for tree reduction |
| VO elaboration | Clauses, input schemas and output head |
| RA demand derivation | Input schemas and expected head; headings are derived from RA |
| IX Physical lowering | Sorted headings, join demands, host schemas, ordered semantic operations and output |
| Physical binding selection | Logical, heading, host and demand facts, plus host schemas and storage advice |
| Printers consuming Γ | Physical reads, modes, canon flags and scans; host layouts, execution operations, expression bindings, computed-column provenance and output head |

IX owns its token-free `Operation` vocabulary. The `structures` language owns
`PhysicalFacts`, `ModeKind`, `UniqueOp` and printer execution facts. Neither
requires producer parser types to be moved into shared syntax. Reference Env
adapters cache validated headings and row facts; in particular, IX and
Physical can return `Option<&[String]>` from `heading` without borrowing a
temporary RA analysis.

Every pass takes its source `Section` and the source language's `Env`, followed
by its options, policy or endpoints, and returns only the target section
(or `syn::Expr` for the pull printer). Passes expose `Env::new` adapters for
their target contracts, carrying binding and expression facts onward. Facade
code constructs source Envs explicitly and reassembles consecutive sections.
`cq_print::Context::new` takes a `structures` section, Env and checked analysis;
`cq-physical-realization` adds pull control flow to its shared helpers.

Later languages expose the reference composition as `Module` through the
default-on `reference-prefix` feature. `reference::Module` parses the prefix
followed by the local section, and `contract::check_module` checks the composed
payload. The feature propagates through the optional prefix chain; disabling
it removes the producer language dependency.

Pass production dependencies disable language defaults. Their default-on
`reference` feature alone enables endpoint `reference-prefix` features (and
`cq-print/reference` for the pull printer). Composed-module conveniences live
under `pass::reference::{compile, check}`. The reference elaboration check
retains its independent CQ/RA kernel comparison; the standalone check requires
the exact elaboration. Test and driver adapters can opt into reference
composition while the facade uses the core APIs.

## Strict isolation rule

Each pass depends on exactly its source and target language cores, plus their
shared syntax; a printer may also use `cq-print`. VO → VO has one distinct
language endpoint; a Rust printer has the `structures` source and external
`syn` target. Passes have no runtime dependency: runtime names are emitted as
Rust tokens.

[`tools/check_dependencies.py`](../tools/check_dependencies.py) enforces these
direct edges, including optional, renamed, build and target-specific
dependencies. With `--isolate`, it builds and tests every pass using
`--no-default-features` with **only its source/target language directories,
shared syntax, and printer support where needed**. It selects endpoints
directly, never their reference-prefix dependency closure. Language cores are
also tested without their optional prefixes. Resolved pass features must
contain no `reference-prefix`, and every pass must actually execute a test.

The isolation copier removes unavailable optional path manifests (Cargo
resolves them even when disabled) and dev-dependencies, while preserving and
checking production non-optional edges. It copies integration tests and their
`required-features` metadata: composed tests require `reference` and are
skipped by Cargo; each pass's `tests/core.rs` uses an independent environment.
Isolation, generated Rust execution and expansion equality are separate checks.

## Composition scenarios

- [own-dialect](../examples/own-dialect/README.md) proves that a push printer can consume Γ and pass oracle tests without depending on the reference pull printer.
- [stacked-ir](../examples/stacked-ir/README.md) proves that an independent `trace {}` section can be added and erased without editing reference crates, preserving execution and byte-identical expansions.
- [diverged-ra](../examples/diverged-ra/README.md) proves that an independent `theta_join` surface can supply IX demands and an Env, reuse unchanged downstream passes without reference CQ/VO/RA or facade dependencies, and execute generated probe/merge Rust against an oracle.
- [composition](../examples/composition/README.md) proves that caller-bound macro slots and a query-independent policy compose the chain with host maps answering every keyed read in place.

## Explicit composition boundaries

| Caller-resolved path | Boundary | Current binding example |
|---|---|---|
| `mini_linq::choose_index_policy!` | RA → IX | Import `index_demands` under this name |
| `mini_linq::physical_policy!` | IX → Physical | A caller macro appends `policy { … }` and invokes `lower_physical!` |
| `mini_linq::select_structures!` | Physical → Γ | Import `select_structures` |
| `mini_linq::structures_to_rust!` | Γ → Rust | Import `structures_to_rust` |
| `mini_linq::distinct_state!` | canonical return | Import a `cq-state-providers` distinct provider |

The policy is query-independent: host facts (`R keyed [a] ordered canon`)
and strategy (`join = merge`, `semijoin = prefer_lazy`). Advice on a relation
declaration (`#[index(Type)]`) names the Rust type built over that relation
when the plan indexes it; it is erased from the query's meaning.

## Current execution domain

The [composition example](../examples/composition/README.md) and the
[full-chain test](../facade/mini-linq/tests/physical_query.rs) exercise every
stage: hosts read in place through their declared keys, compiler-built hash
indexes, a user-defined index type through `#[index(Type)]`, merge joins over
built ordered maps with a sorted pipe side, Yannakakis semijoin passes as
membership tests, a bounded scan of an ordered host, and a computed column.
The printed program is one Rust block evaluating to an iterator of answer
tuples in head order; the caller collects it however it likes.

Recursion, aggregation, negation and incremental evaluation are outside the
language; deltas are ordinary hosts under a rewrite tool, not IR constructs.

## Execution state selection

The canonical return is deduplicated through the caller-bound
`mini_linq::distinct_state!` slot; there is no HashSet fallback. Each request
carries its local operator name, so a caller policy can choose separately for
each occurrence. State provider syntax and laws live in `cq_protocol`;
`cq-state-providers` supplies hash and ordered implementations, which own
retained copies of the rows they deduplicate.

The printer owns polling and loop order. No execution strategy copies a
binding: a value bound by an outer source is used inside its dependent joins
and inner loops as the value the source produced. Hosts and built structures
yield references, so those are free; an owned value (a computed column) that
one outer row must hand to several inner rows is a Rust move error, and the
head expression alone decides what the result owns. The remaining copies are
the ones a type imposes: probing a host map keyed by owned tuples constructs
the key, and a distinct provider retains its own copy of each row.
