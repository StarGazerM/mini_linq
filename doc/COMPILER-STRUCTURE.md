# Compiler ownership and composition

The accepted path is `CQ → RA → IX → Physical → Structures → Rust`. Each
language is the previous one plus one section; each pass records one
decision and is checked by the next language's contract. Below Structures a
printer emits one pull program over the `cq-storage` traits.
See [DSL.md](DSL.md) for the grammars.

## Crates and modules

| Crate | Responsibility | Modules / entry functions |
|---|---|---|
| `cq-ir` | Typed syntax and local validity | `cq`, `relational_plan`, `index_requirements`, `physical`, `structures`, `policy`, `state_protocol` |
| `cq-query-lowering` | CQ planning and RA construction | `written_order`, `clause_prefix`, `hypergraph`; each exports `compile`; `dependencies`, `model`, `proof` support planning |
| `cq-index-policy` | RA → IX; derive required join-column indexes and their uses | `compile`; `correspondence` checks the edge |
| `cq-physical-lowering` | IX + policy → Physical; one physical operator per result, modes from use, `canon` on the return when uniqueness is not inferred | `compile`, `check` |
| `cq-structure-selection` | Physical + policy → Structures; host facts from the policy, a built type for every read no host key answers, `#[index(Type)]` advice | `compile`, `check` |
| `cq-physical-realization` | Structures → Rust; builds the chosen structures, guards the traits each read needs, prints the pull program | `compile` |
| `cq-state-providers` | Distinct-state providers behind `state_protocol` | `hash_distinct`, `ordered_distinct` |
| `cq-storage` | Runtime data structures and operation traits | `Build`, `IndexRead`, `ExactMembership`, `OrderedRead`/`merge`/`sorted_by`, `Canonical`, `require` guards; `HashIndex` |
| `mini-linq` | Procedural macro facade and caller-resolved continuations | `lib.rs` exports entries; `entry` transports typed stages; `workload` captures catalog CQ |

Every family crate depends only on `cq-ir` (`tools/check_dependencies.py`
enforces it); test dependencies may compose stages. `mini-linq-course` is the
workspace/example root, and `cq-workloads` is a typed query/data catalog.
Neither is a compilation stage. The separate `xtask` workspace contains
development tooling.

## Explicit composition boundaries

| Caller-resolved path | Boundary | Current binding example |
|---|---|---|
| `mini_linq::choose_index_policy!` | RA → IX | Import `index_demands` under this name |
| `mini_linq::physical_policy!` | IX → Physical | A caller macro appends `policy { … }` and invokes `lower_physical!` |
| `mini_linq::select_structures!` | Physical → Structures | Import `select_structures` |
| `mini_linq::structures_to_rust!` | Structures → Rust | Import `structures_to_rust` |
| `mini_linq::distinct_state!` | canonical return | Import a `cq-state-providers` distinct provider |

The policy is query-independent: host facts (`R keyed [a] ordered canon`)
and strategy (`join = merge`, `semijoin = prefer_lazy`). Advice on a relation
declaration (`#[index(Type)]`) names the Rust type built over that relation
when the plan indexes it; it is erased from the query's meaning.

## Current execution domain

The [composition example](../examples/composition/README.md) and the
[full-chain test](../crates/mini-linq/tests/physical_query.rs) exercise every
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
each occurrence. State provider syntax and laws live in `cq_ir::state_protocol`;
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
