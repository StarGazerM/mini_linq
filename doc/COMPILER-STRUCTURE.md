# Compiler ownership and composition

The pipeline is `CQ → NF → VO → RA → IX → Physical → Structures → Rust`.
Every boundary carries one complete `Program`. Every program supports
parsing, token emission, cloning, debugging and equality.

## Crates and dependencies

| Language | Program and contract | Language base |
|---|---|---|
| `cq-lang-cq` | Declarations and query; normalization and hypergraph analysis | None |
| `cq-lang-vo` | `Program { cq, valuation }`; complete schedule over its normal CQ | `vo::cq` |
| `cq-lang-ra` | Declarations, named algebra and output inside `relational { … }` | None |
| `cq-lang-ix` | `Program { ra, indexes }`; demands on exact RA results | `ix::ra` |
| `cq-lang-physical` | Declarations, detailed operators, modes and output inside `physical { … }` | None |
| `cq-lang-structures` | `Program { physical, structures }`; storage choices discharge Physical premises | `structures::physical` |

A language depends on `cq-syntax` and at most its declared base. These three
extensions re-export their base under exactly the names above. RA and Physical
are standalone. `cq-syntax::algebra` owns the common Unit, Rename, Select,
Extend and Project details; it contains no whole-language contract.
Language and pass crates have no Cargo features.

| Pass | Source → target | Additional input |
|---|---|---|
| `cq-order` | CQ → VO | Heuristic and split |
| `cq-reduce` | VO → VO | Guard pushdown and tree reduction options |
| `cq-elaborate` | VO → RA | None |
| `cq-index-policy` | RA → IX | None |
| `cq-physical-lowering` | IX → Physical | Physical policy |
| `cq-structure-selection` | Physical → Structures | Physical policy |
| `cq-physical-realization` | Structures → Rust expression | Printer endpoints |

Each pass exports `compile(&source, options…) -> syn::Result<Target::Program>`
and `check(&source, &target, options…) -> syn::Result<()>`. Realization
exports only `compile`, returning `syn::Expr`. Physical erasure maps
probe/merge to natural join, semi to semijoin and range to select, discarding
modes and canon.

Each pass depends directly on exactly its endpoint languages, with shared
syntax permitted. Realization also uses `cq-print`, whose `Context::new`
reads only a Structures program and its checked analysis. Physical analysis
owns headings, host mappings, source identifier bindings, computed columns,
reads and modes. Printers use these facts and the Physical operators directly.
Source identifiers retain their hygiene through all boundaries.

Runtime crates own storage, the state protocol and providers independently of
compiler languages and passes. Compiler passes emit runtime names as Rust
tokens. `cq-stage` supplies typed macro continuations and `Policed<Program,
Policy>`; `mini-linq` supplies the procedural entries. Consumers include the
course package, workloads, `xtask`, and examples.

## Contracts and evidence

Four kinds of evidence are kept apart; none substitutes for another.

| Evidence | Owner | Question it answers |
|---|---|---|
| Language contract | `contract::check(&Program)` in each language crate | Is this one program well formed? It returns the analysis (headings, reads, modes) consumers use. |
| Pass correspondence | `check(&source, &target, options…)` in each pass crate except realization | Does this target correspond to this source under these options? |
| Fixtures | Per-stage scenario files and authored expansions | Is this the output the compiler is meant to produce? |
| Oracles | Evaluators, SQL and independent reference answers | Does the executed plan compute the query's answers? |

A language contract sees only its own program; an extension's contract
(VO, IX, Structures) also checks its base program. `compile` ends by checking
the target contract, so an invalid program never leaves a pass. It does not
invoke `check`; tests, tools and callers invoke it explicitly.

`check` is an independent correspondence, not a replay of `compile`. It
checks the source is in the pass's domain, the target contract, the relation
between them (unchanged base program, reduction erasure, CQ-NF kernel
agreement, RA erasure of Physical, preserved Physical), and the obligations
of every explicit choice (split granularity, reduction tree edges, demand
derivation and names, policy-mandated operators and host facts, read
admissibility and `#[index]` advice). Choices the options leave free — a
prefix or elimination ranking, sibling reduction order, alias/let/pipe when
several are admissible, structure entry order — are accepted in any
admissible form; the selector is never re-run to compare.

The printer has no `check`: nothing independent can decide whether Rust means
a Structures program. Its evidence is oracle answers from executing the
expansion, and the `cq_storage::require` guards through which rustc checks
that every built type implements the traits its reads use.

Fixtures pin the choices `check` leaves free.
[test-support/relational/scenarios](../test-support/relational/scenarios) holds
30 scenarios, each as `.cq`, `.vo`, `.ra`, `.ix`, `.physical`, `.policy` and
`.structures`; the `.ix` and `.structures` files are suffixes assembled onto
`.ra` and `.physical` by `Scenario::ix` and `Scenario::structured`. Each pass
test pins `compile(stage)` to the next stage, requires `check` to accept it,
requires `check` to reject targeted mutations, and compares answers with an
oracle. `tools/check_expansions.py` compares the full-chain expansions byte for
byte with a saved baseline.

## Validation and composition

`tools/check_dependencies.py --isolate --frontends --consumers` checks direct
edges, rejects forbidden dependency mutations, and copies every language with
only its base chain and every pass with only its two endpoint chains. It runs
their real tests, including shared scenario fixtures. The teaching HW4 target
is enabled by the course package's `hw4` feature; language crates stay featureless.

- [own-dialect](../examples/own-dialect/README.md) prints nested loops from a complete Structures program.
- [stacked-ir](../examples/stacked-ir/README.md) extends Physical with Trace and erases it, preserving expanded Rust bytes.
- [diverged-ra](../examples/diverged-ra/README.md) desugars its theta join into RA, constructs IX demands itself, and reuses downstream passes.
- [composition](../examples/composition/README.md) binds the public macro continuations and host storage policy.

## Explicit composition boundaries

| Caller-resolved path | Boundary | Current binding example |
|---|---|---|
| `mini_linq::cq_to_ra!` | CQ → RA, selected by `mini_linq!` after it checks the CQ contract | Import `cq_to_ra`, `cq_to_ra_generic` or `cq_to_ra_hypergraph` |
| `mini_linq::choose_index_policy!` | RA → IX | Import `index_demands` under this name |
| `mini_linq::physical_policy!` | IX → Physical | A caller macro appends `policy { … }` and invokes `lower_physical!` |
| `mini_linq::select_structures!` | Physical → Structures | Import `select_structures` |
| `mini_linq::structures_to_rust!` | Structures → Rust | Import `structures_to_rust` |
| `mini_linq::distinct_state!` | canon pipe or alias return | Import a `cq-state-providers` distinct provider |

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

Negation and aggregation are parsed optional syntax, not supported baseline
semantics; recursion and incremental evaluation are not implemented. Deltas
are ordinary hosts under a rewrite tool, not IR constructs.

## Execution state selection

A canon pipe or alias is deduplicated where it is scanned through the
caller-bound `mini_linq::distinct_state!` slot; there is no HashSet fallback.
A canon let is read from its materialized canonical storage instead. Each
request carries its local operator name, so a caller policy can choose
separately for each occurrence. State provider syntax and laws live in
`cq_protocol`; `cq-state-providers` supplies hash and ordered
implementations, which own retained copies of the rows they deduplicate.

The printer owns polling and loop order. No execution strategy copies a
binding: a value bound by an outer source is used inside its dependent joins
and inner loops as the value the source produced. Hosts and built structures
yield references, so those are free; an owned value (a computed column) that
one outer row must hand to several inner rows is a Rust move error, and the
head expression alone decides what the result owns. The remaining copies are
the ones a type imposes: probing a host map keyed by owned tuples constructs
the key, and a distinct provider retains its own copy of each row.
