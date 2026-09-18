# Compiler boundaries and replacement laws

The [staged borrowed execution](STAGED-BORROWED-EXECUTION.md) follows the existing
CQ → RA → IndexRequirements → Rust[Access, Native] → Pipeline → Rust interfaces.
The physical implementation does not generate query iterator adapters or loops.
It emits checked local Access regions; the selected execution and Pipeline
implementations compile those regions. The [formal supplement](formal/README.md)
maps the corresponding rules to their implementation sites.

## Decision ownership

| Boundary | Selected decision | What remains for later stages |
| --- | --- | --- |
| CQ → RA | Logical join order, YA reductions, legal expression placement | Index, representation and execution choices |
| RA → IX | Written equality demands, including the explicit empty policy | Their implementation or explicit rejection |
| IX → staged Rust | Input layout, borrowed field bindings, region decomposition and resource lifetimes | Access control lowering and Pipeline adapter emission |
| Native Build/Scan | Replay storage and independently owned cursor construction | Build retains its selected Access continuation |
| Access → Pipeline | Ordered Scan/Join/Filter/Project/Distinct bindings | Concrete Rust adapters and endpoint observation |
| Pipeline → Rust | Iterator adapters and distinct/support/existence/set consumption | Ordinary Rust type and borrow checking |

A caller selects implementations through Rust import aliases. This selection does
not imply that the graph planner inspects storage or execution choices: it does
not. Under Auto, the graph planner uses GYO and YA on acyclic queries; otherwise
it uses greedy next-join column arity. Written, Connected and GreedyWidth are
explicit alternatives. There are no cardinality estimates or global cost search.
Named RA serializes a DAG; names imply neither allocated row sets nor shared
iterator cursors.

## Borrowed scan composition

`no_index_demands!` preserves RA and appends `indexes {}`. `lower_borrowed_nested!` accepts that
IX form, checks its restricted domain, and emits one Access region for each RA
definition. It rejects nonempty demands instead of ignoring them. Unit has an
empty Access body. Rename reads fields through Native providers; Project and
Join construct tuples of references; Semijoin uses a nested Access region whose
Pipeline endpoint answers existence. The final region uses the distinct
iterator endpoint. No new whole-query language is introduced.

Each shared definition and each join/semijoin RHS receives a replay resource.
Single-use streams move into their consumer. This lifetime decision is made by
the physical implementation, while `replay_resource!` delegates storage construction
to `BuildReplay` and cursor access to `ReplayRead`. Its Build contains a real
Access invocation, so its producer still traverses Access and Pipeline stages.
Replay stores only demanded prefixes and returns copied reference tuples. It
owns no relational operator. Payload ownership remains with the caller.

Intermediate support streams can contain duplicates and require no added
payload hashing; the final distinct answer requires lawful `Eq`/`Hash` on its
referenced output values. Matching shared fields requires lawful equality.
Construction can allocate state but does not scan inputs or read fields.
`next()` or `any()` drives computation; `collect()` additionally retains output.
Replay itself is incremental intermediate materialization, may grow to full
inputs, and uses single-threaded `Rc` state. Nested comparisons still repeat.

The Access contract admits an empty body (one empty binding) and a simple
identifier pattern for exactly one fresh logical variable. Other fresh patterns
remain tuples in relation-column order. These are changes to the existing
language contract, reflected in its parser, checks and all stage consumers;
there is no private parser or parallel access protocol.

## Replacement obligations

For `compile_a : D_a ⊆ L → M`, a replacement must admit the actual input and
preserve its consumer's observation and ownership conditions. A support stream,
a distinct iterator, an existential boolean and a completed set are different
contracts. In particular, replacing a lazy source with an eager collector can
preserve completed values while violating partial-demand behavior. Rust checks
concrete types, available operations and lifetimes; it does not prove provider
laws, purity or equivalence of evaluation effects.

The borrowed route admits Unit, Rename, Project, NaturalJoin and Semijoin with
variable-only outputs. Select, Extend and computed outputs require another
realization. Stable finite sources and pure lawful value comparisons justify
sharing demanded prefixes. No claim of equivalence for arbitrary side effects,
panics or destructor traces follows from equality of completed answer sets.

## Languages, implementations and compositions

The implemented compiler families compose through these existing boundaries.
The borrowed scan route uses an explicit empty index overlay and the same
Access and Pipeline interfaces as the hash routes.

```text
CQ → RA → IndexRequirements → Rust[Access, Native] → Rust[Pipeline, Native] → Rust
     RA ── configured relation realization ──→ Rust[Access, Native]
                                Access ── push to set ──→ Rust
                                Native ── selected implementation ──→ Rust[Access] or Rust
```

`Rust[X]` means an ordinary Rust AST containing macro calls whose operands are
in language X. It is not another serialized whole-query IR. Native operations
are local types, expressions and statements after relational names and headings
have been resolved. The relation environment Γ is compiler state, not syntax.

For an implementation a, write `compile_a : D_a ⊆ L → Rust[H]` (or another
existing language). A replacement must accept the emitted source and preserve
the observation its consumer requires. Syntax compatibility alone is insufficient.
A set consumer, an iterator consumer and a repeatable traversal consumer impose
different requirements. Rust checks available operations, types and lifetimes;
semantic laws still require reasoning and tests.

A macro name identifies either an implementation, a composition recipe or a
host adapter. It does not by itself identify a new pass or language.

## Public macro families

The macro families are grouped by the boundary or recipe they own.
The [staged borrowed route](STAGED-BORROWED-EXECUTION.md) is a composition of
these interfaces, with independently selected intermediate, witness and output
observations.

| Family | Entries | Contract and decision owner |
| --- | --- | --- |
| Catalog | `workload_query` | CQ catalog; compiled workloads use the same caller-bound query route. |
| Query composition | `mini_linq`, `choose_graph_or_prefix` | Select canonical/prefix or graph/prefix planning, then invoke the caller-bound planner macro. |
| Logical planning | `cq_to_ra_written_order`, `cq_to_ra_clause_prefix`, `cq_to_ra_hypergraph` | CQ → RA; planner owns legal scheduling and its admitted domain. |
| Default index choice | `choose_index_policy` | RA → invocation of `base_index_demands` or `result_index_demands`. |
| Index policy | `base_index_demands`, `result_index_demands`, `no_index_demands` | RA → IX; base-input, named-result, or empty equality demands. |
| Default realization choice | `choose_relation_lowering` | Inspect actual IX and dispatch to an admitted whole-query realization. |
| Fixed hash realization | `lower_base_hash`, `lower_prefix_hash`, `materialized_hash_program` | IX → Rust[Access]; each explicitly commits to its hash layout and schedule. |
| Borrowed scan realization | `lower_borrowed_nested` | IX → Rust[Access, Native]; empty-index pure RA, borrowed tuples and reusable resource ownership. |
| Hash callable host | `materialized_hash_callable` | Same eager computation in an anonymous callable. |
| Configured relation realization | `configured_program`, `configured_callable` | RA → Rust[Access, Native]; selected per-input/result representations and local algorithms. |
| Observed relation hosts | `observed_configured_program`, `observed_configured_callable` | Same realization with explicit counters/checkpoints. |
| Pull execution | `access_to_distinct_iter`, `access_to_set`, `access_to_support_iter`, `access_to_exists` | One Access → Pipeline lowering; distinct iterator, set, support stream or existential observation. |
| Push execution | `access_to_set_loops` | Access → eager Rust loops and exact-set sink. |
| Pipeline endpoints | `pipeline_to_distinct_iter`, `pipeline_to_set`, `pipeline_to_support_iter`, `pipeline_to_exists` | Pipeline → Rust; distinct iterator, exact set, support stream or first-witness boolean. |
| Candidates | `scan_candidates`, `prune_block_candidates` | Current-view handle enumeration; prune tests metadata before payload. |
| Key resource | `exact_keys`, `summarized_exact_keys` | Key construction and exact membership; zone also provides conservative rejection. |
| Witness resource | `hash_witnesses`, `nested_witnesses` | Coherent Build/Probe resource protocols for consumers that request them. |
| Pair algorithm | `hash_pairs`, `nested_pairs`, `sort_merge_pairs` | Complete equal-key pair enumeration, including both traversals and auxiliary storage. |
| Array rows | `array_hash_rows`, `array_sorted_rows`, `array_block_rows` | Concrete i64-array representations; block operations are provided only by block rows. |
| Replay resource | `replay_resource` | Native Build/Scan; build through an Access continuation and return owning cursors over a shared demanded prefix. |
| Tuple rows | `tuple_hash_rows`, `tuple_sorted_rows` | Heterogeneous Rust tuple representations. |

Convenience recipes are documented separately from transformations. Item and
callable hosts share definition lowering. Witness resources remain useful native
operations, but the configured natural-join rule uses the larger pair boundary.

## Why the native dialect contains producers

The extension is located in `cq-ir::native`. It is needed because neither Rust
field syntax nor a fixed build/probe loop is representation- or algorithm-neutral.
It introduces no RA operator after RA lowering.

```text
type (RustType, ...)                           : Rust type
field (row) (column)                           : Rust place expression
record (RustExpr, ...)                         : Rust row expression
key (RustExpr, ...)                            : Rust key expression
produce owner view via emit { RustStatements } : Rust statements
pairs l in (left) by (left_key),
      r in (right) by (right_key) { RustStatements } : Rust expression ()
```

`type` supplies logical column types; the selected provider chooses their
representation. `field` and `record` expose only logical coordinates and row
construction. Configured lowering supplies borrowed coordinates to `key`:
exact keys retain typed reference tuples, while zone keys copy referenced `i64`
coordinates into integer arrays. Their equality concerns values, not addresses.
Array-specific restrictions belong to array providers, not RA validity.

`produce` binds the named emission function and executes the body. The provider
binds `owner` and `view` to exactly the emitted row set, deduplicating once at its
construction boundary. The algorithm writes directly into that result
representation. `materialize owner view (iterator)` remains useful for an
already-iterator-valued producer; neither operation adds a relational interpreter.

`pairs` binds l and r to borrowed input items. Each key expression sees only its
own item, and the body sees both. For finite stable traversals and pure keys,
the body executes once for each input occurrence pair (x,y) whose keys are equal.
Multiplicity is preserved until the chosen result consumer removes duplicates.
Hash, nested and sort-merge own their different traversal orders, rescans and
auxiliary structures. The stock sort-merge implementation explicitly sorts both
inputs; it does not assume BTreeSet row order implies separator-key order.
Arbitrary Rust effect traces are not equivalent under algorithm replacement.

The RA join rule computes separator coordinates, passes two row traversals and
borrowed-field key expressions, then emits a constructed output row to the selected sink.
It does not impose a hash witness resource. Whole-query materialization remains
a separate choice made by the selected relation realization.

This does not restrict independently authored pair keys to borrowed values:
pair algorithms also accept owned keys derived from owned traversal items when
Rust permits the required moves and borrows. Keys stored in auxiliary structures
must remain valid for those structures' lifetime.

## Semijoin law and unsupported compositions

Let H(v) be the handles in the current left view, K the exact projected current
right view, and p(h) mean `key(row(h)) ∈ K`. Candidates C must satisfy:

```text
{h ∈ H(v) | p(h)} ⊆ C(v, metadata) ⊆ H(v)
```

The result is `{h ∈ C | p(h)}`. This proves exact semijoin despite conservative
metadata. Candidate traversal preserves current selection and handle validity.
The exact residual is required even after a metadata test succeeds.

Zone metadata is not a key-to-block lookup. For each block, its coordinate bounds
are compared to the RHS key summary. Disjoint ranges reject the block; overlapping
ranges require selected payload access and exact key membership. There is no
false-negative allowance. A later semijoin cannot resurrect handles removed by
an earlier one. Reusing original immutable block bounds remains conservative.

Choosing pruning with hash rows expands unsupported block operations and fails.
Choosing it with exact-only keys expands unsupported `may_match` and fails. Scan
can discard its unused metadata operand before expansion. The compiler does not
silently swap user-selected storage or maintain a predefined capability registry.

Key resources are shared only for the same current RHS result, ordered key
coordinates and selected macro binding. This is conservative identity, not
semantic equivalence inference across aliases.

Native set selection stores the supplied row references without cloning payloads.
A later selection can therefore borrow an earlier selected view. Generated
bindings retain the view chain and its owners until every dependent resource
and traversal completes. Borrowing changes representation, not the current-view
or exact-membership laws.

## What each boundary deliberately fixes

- Logical planners own relational scheduling. Index policies describe equality
  demands, not storage capabilities or control-flow tags.
- `lower_base_hash`, `lower_prefix_hash` and `materialized_hash_program` are concrete implementations;
  their later Access holes cannot undo earlier hash layout decisions.
- Configured relation realization supports typed headings, Select, Extend and
  computed output. It materializes producing RA definitions, selects views for
  filtering and semijoin, and uses a deterministic set-valued answer host.
  Expression aliases borrow both source and computed fields. Consuming Rust
  expressions must request copy/clone explicitly; owned record and answer
  construction may still clone retained fields. No general clone-elimination or
  projection-liveness guarantee follows from borrowed access.
- Pair algorithms own one binary join's schedule. Region fusion, global
  materialization placement and cost optimization require another realization;
  they are not claimed as features of the pair seam.
- Pipeline is a linear single-consumer representation. Names, intermediate
  projections and retained bindings are flexible; stream duplication is not an
  implicit cloning/ownership operation.
- Push and pull are execution alternatives only where the observation matches.
  `access_to_set_loops` can replace an eager exact-set hole, not a demand-sensitive iterator
  hole merely because both accept Access syntax.

## Validation obligations

Domain checks and independent correspondence checks run on typed boundaries.
Runtime tests additionally exercise emitted Rust, because AST equality cannot
prove native expansion, ownership or results. Coverage includes caller alias
routing, alternate index policies, graph defaults, workload routes, typed/block
compositions, pair algorithms, current-mask pruning, resource versions, expression
capture, output ownership, endpoint multiplicity and demand-sensitive effects.

Feature isolation, source-removal builds and teaching extraction are separate
buildability checks. They do not prove arbitrary replacement laws. Exact commands
and the validated source identity belong to [the snapshot](formal/snapshot.txt).
