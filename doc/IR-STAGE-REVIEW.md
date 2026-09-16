# Compiler boundaries and replacement laws

This document records the implemented boundaries and the proposed correction
below. The correction is a design, not a claim that the compiler has already
changed. The rule/source correspondence for the implemented compiler is maintained
in [the formal supplement](formal/README.md).

## Proposed execution-boundary redesign

Status: design only, 16 September 2026. The later sections describe the current
implementation. This proposal supersedes treating its Access boundary as the
general place to choose whole-query push or pull execution.

### Decision and scope

For this redesign, keep CQ-to-RA planning independent of push/pull and concrete
storage. The current planner already selects the logical joins, YA reductions,
selections and projections while constructing named RA. The subsequent execution
realization implements that plan using the chosen control model and per-relation
storage/algorithm bindings. Demand, buffering and resource-lifetime obligations
belong to that realization; they are not a reason to add execution parameters or
feedback to the current RA planner.

The intended required order is CQ -> planned named RA -> selected execution
realization -> Rust. The realization may contain smaller selected macro
lowerings. This order needs neither a new shared physical-plan IR nor a mandatory
standalone physical-planning pass. The selected implementation can resolve its
physical schedule internally. Separately selectable passes that exchange a new
semantic schedule would require that representation to be specified as an IR.

Caller aliases may select a backend before compilation starts, but that is
configuration, not evidence that the RA planner consults the execution model.
In the current compiler it does not. Emit concrete query iterator state/next
logic or generated consumer bodies after the selected RA and the relevant
physical decisions are settled. Merely having a storage iterator available
does not establish that the query's control flow has been selected. Lower-level
code optimization remains possible after this commitment.

### Evidence and problem

The actual expansion of `examples/composition/src/main.rs` invokes graph_query,
result_indexes and hash_materialized_query before six pull_set/collect_pipeline
pairs. The third invocation has already normalized inputs to HashSets,
materialized every named RA result, selected hash probes and emitted index-build
loops. Its remaining Access macros cannot revise those decisions. See
`cq-realize/src/hash_materialized/query.rs`, particularly the per-definition
emission and `build_index`.

This is a valid ordering for the particular materialized-hash implementation.
The defect is presenting it as a general stage preceding an independent
whole-query execution choice. It can be a specifically selected realization;
other execution realizations must not be required to pass through it.

The configured relation route also commits to different local control protocols:
Select and Semijoin consume iterable Access regions; NaturalJoin emits native
Produce/Pairs bodies. Combining these is legal, but it does not provide a common
execution-neutral representation. Rust `for` loops over storage are compatible
with either query model; the issue is which compiler owns operator communication,
resource lifetimes and materialization boundaries.

### One shared language; alternative realizations

Use the existing named RA DAG as the last required common representation.
IndexRequirements remains an optional existing overlay for a composition that
requests equality resources. Two realizations may share IX only when both honor
its written demands; another algorithm must not silently ignore them.

```text
CQ -> planned named RA
          |-> selected push realization -> Rust[native fragments] -> Rust
          `-> selected pull realization -> Rust[native fragments] -> Rust
```

An implementation may insert the existing IX overlay when it requests equality
indexes, or retain Pipeline when an independent iterator transformation needs
that representation. Neither is a required stage on all paths. These branch
choices happen after RA planning; no push/pull feedback edge into that planner
is proposed.

This is a graph of alternative compiler edges. It does not instantiate both
branches for the same region, introduce a PushIR, or require a universal
PushOrPull enum. A push realization can generate consumer code as ordinary Rust
statements. A pull realization can generate cursor state and demand handling
directly. The existing Pipeline is optional: retain that edge when an independently
useful iterator transformation or endpoint needs it, not because Volcano requires
another graph language. `Rust[X]` continues to mean ordinary Rust containing
unexpanded macro invocations.

### Join planning and named graph representation

Current `graph_query!` calls `cq_plan::graph::compile`. In graph/plan.rs, GYO
analysis of the atom-variable hypergraph precedes RA emission. Initial renames
and selections are then emitted; an acyclic plan receives YA reductions. The
assembly loop chooses a pair of current groups, applies Combine to emit the
corresponding NaturalJoin, and applies ready selections/projections. Thus join
scheduling and construction of named RA are interleaved. There is no separate
implemented RA-to-RA join-reordering pass. Under Auto, a successful GYO analysis
selects YA's two semijoin sweeps followed by join-tree-guided assembly. If GYO
finds no join tree, Auto instead uses greedy next-join column arity. Explicit
Written, Connected and GreedyWidth policies select their corresponding binary
planning routes without YA. None of these routes uses cardinality estimates or
global cost search; that limitation does not turn YA into written-order planning.

The GYO join tree has atom occurrences as nodes and certifies acyclicity. The
chosen binary join graph has RA operations as nodes. They have different meanings.
The existing RA names serialize the latter graph's definition/use edges; naming
each result once does not force allocation, execution order between independent
nodes, or any particular operator communication protocol.

SSA-like representation and graph optimization are therefore compatible. A
future independently selected join optimizer can rewrite RA to RA, preserving
well-formed references, headings and its admitted observation contract. It does
not require a new graph IR merely because it rewrites join edges. Volcano's
open/next/close protocol and Neumann-style producer/consumer code generation are
execution implementations of a selected physical plan, not consequences of its
textual naming convention. Algorithm, storage and resource choices still have
to be supplied or resolved; logical join connectivity alone does not fix them.

The relevant early commitment in the captured third macro is allocating and
filling collections, emitting hash-specific accesses, and fixing blocking
boundaries. It is not the earlier introduction of r0, r1 and other RA names.

For the captured example, the first two expansions can remain: the replacement
choice is made on their RA/IX output, before the old third invocation emits its
materialized Rust host. Neither swapping the six inner calls nor renaming
hash_materialized_query establishes this new boundary.

Representations supplied by the caller may be known before this choice. Selecting
a storage type does not itself select consumer-driven or producer-driven query
control. The selected realization composes storage operations with its control
protocol and decides resource construction, sharing and barriers before emitting
those commitments into Rust. Missing operations reject the selected composition;
the compiler does not substitute another user-selected representation.

### Semantics and compilation judgments

Let P be an existing RA module, I an optional existing index overlay, rho the
database and Gamma the caller's compile-time implementation bindings. For a
selected compiler C, write:

```text
Gamma |- (P, I) lower_C H
```

H is staged Rust. Gamma and C are parameters of this judgment, not new serialized
IR fields, capability tags or runtime objects. Bindings are selected through
actual macro paths. A compiler's admitted domain includes the source language,
requested observations and the operations of those selected implementations.

The two code-generation interfaces can be written:

```text
Push_Gamma(r, k) = Rust statements delivering rows into consumer code k
Pull_Gamma(r)    = Rust expression constructing resumable result traversal
```

These are mathematical descriptions of compilation functions, not proposed
surface grammar. k is a compiler continuation that supplies generated code;
neither runtime callback objects nor a query interpreter are required.
For a selection over an admissible pure predicate p:

```text
Push(select_p(r), k) = Push(r, x => if p(x) then k(x))
next(Pull(select_p(r))) = advance the child until p(x), then return x;
                         return end when the child ends
```

The push rule does not first create an iterator-valued query operator. The pull
rule must preserve the child state across demands. Both may read the same
physical storage through its existing iterator or other valid access operations.

For finite, stable, pure positive RA, assume Gamma's selected native operations
satisfy their stated laws, equality/hashing/ordering obey the selected algorithms'
contracts, any emitted clones preserve values, and generated borrows are valid.
Under these premises the common completed-set obligation is:

```text
run(Push(P, insert_into_empty_set), rho)
    = set(drain(run(Pull(P), rho)))
    = [[P]](rho)
```

Here running Push means executing its statements and observing the final sink
set. These premises are proof obligations; they are not inferred merely from
successful Rust typing, arbitrary macro bindings or opaque Rust expressions.

An internal support producer may repeat witnesses only when its enclosing
consumer establishes the required set semantics. An exact-set boundary still
requires distinctness. An iterator advertised as distinct must itself provide
that observation; merely returning duplicate support does not satisfy it.
Partial demand, evaluation order, panics and destruction are additional contracts,
not consequences of completed-set equality. Opaque Rust expressions are not
assumed pure: transformations must preserve established evaluation boundaries or
reject a domain for which their effect obligations cannot be established.

A named RA result denotes a relation and its dependencies, not an instruction to
allocate a collection. Shared uses require an explicit realization decision:
reuse stored state, recompute under justified purity/effect conditions, or reject
unsupported sharing. YA still requires its exact named reduced versions; moving
the execution choice earlier does not eliminate its possible blocking work.

Borrowing remains checked by Rust. A pull result may not borrow a locally built
resource that dies on return. Its generated state must own the necessary resource
or borrow an owner with a sufficient lifetime. Unsupported combinations fail;
they do not justify cloning unused payload or adding a runtime Relation wrapper.

### Existing lower IR and native operations

RustAccessPlan is a concrete ordered `for`/`if` region over selected Rust sources.
It is not needed as the mandatory shared input to the two new realization edges.
Remove that role and migrate its query-route users rather than preserving the old
stage order with forwarding aliases. IteratorPipeline remains confined to a
realization that has actually selected its cursor/iterator semantics.

Native operations must be assigned their actual contracts:

| Current operations | Existing commitment |
| --- | --- |
| Type, Field, Record, Key, Contains, Bounds | Value, representation or predicate operations; often shareable |
| Scan, Rows, Blocks, BlockRows, Probe | Current implementations return iterable traversals |
| Build, Candidates/prune | Current implementations construct nested iterator-valued Access regions |
| Select, Materialize | Current implementations consume iterable rows/handles |
| Produce, Pairs | Current implementations execute consumer bodies |

Retain shareable representation operations. Place enumeration, resource building,
pair production and pruning behind the selected realization's corresponding
operation implementations. A producer implementation must not be forced to make
an iterator of query results solely to satisfy the old shared Access interface.
A cursor implementation must not eagerly compute its whole result merely to wrap
that result in an iterator. Genuine blocking resource preparation is permitted
when stated by the chosen algorithm and observation contract.

This does not require duplicating every storage primitive or immediately adding
two native grammars. Typed operation inputs and Rust expression/statement outputs
can delimit implementation APIs. Any new cross-pass semantic representation,
whether persisted or in memory, must be specified as an IR; calling it a context
or request does not remove that obligation.

### Migration and acceptance criteria

1. Define the admitted RA/IX domain and result observation of each realization
   before changing emitted code. Reuse heading, key and dependency facts; do not
   hide one branch's scheduling decisions in a shared helper.
2. Implement the checked-in composition query from the same RA/IX through both
   choices. Preserve proc-macro input/output captures at every actual boundary.
   Selection must occur before input normalization and result materialization.
3. Move the needed native traversal and resource operations under the selected
   control implementation. Verify a nonblocking filter/project chain without a
   collection per RA node; a resumable result must obey its declared demand
   contract. A storage `.iter()` call or an index-build loop is not a failure.
4. Test empty/duplicate inputs, shared RA uses, exact semijoin versions, borrowed
   non-Clone fields, explicit result ownership and unsupported lifetime/operation
   combinations. For pruning, retain metadata-before-payload and exact residual
   membership tests for both selected control implementations.
5. A push-only composition must build without the query pull/Pipeline compiler;
   a pull-only composition must not call the push query compiler as a mandatory
   adapter. Explicitly chosen blocking phases have their own tested contracts.
6. Migrate examples, feature/source-isolation checks, teaching extraction and
   formal rule/source mappings. Remove superseded wrappers and mandatory Access
   stages without backward-compatibility aliases. Do not report the migration
   complete while an old route is still presented as the general architecture.

A further common physical IR is justified only by a demonstrated need to inspect
and transform resource construction, probes, pruning, sharing or consumption
between independent passes before choosing control. RA already expresses the
relational operation and version dependencies. A new language must add an actual
missing distinction, with lifetime/effect and preservation rules; repeating RA
with scan/join names or wrapping push and pull variants in one enum is insufficient.

## Languages, implementations and compositions

```text
CQ → RA → IndexRequirements → Rust[Access] → Rust[Pipeline] → Rust
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

The frontend exports 39 macros, grouped here by the boundary or recipe they own.

| Family | Entries | Contract and decision owner |
| --- | --- | --- |
| Catalog | `workload_query` | CQ catalog; compiled workloads use the same caller-bound query route. |
| Query composition | `mini_linq`, `optimized_query` | Select canonical/prefix or graph/prefix planning, then invoke the caller-bound planner macro. |
| Logical planning | `canonical_query`, `prefix_query`, `graph_query` | CQ → RA; planner owns legal scheduling and its admitted domain. |
| Default index choice | `choose_indexes` | RA → invocation of `input_indexes` or `result_indexes`. |
| Index policy | `input_indexes`, `result_indexes` | RA → IX; base-input or named-result equality demands. |
| Default realization choice | `choose_realization` | Inspect actual IX and dispatch to an admitted whole-query realization. |
| Fixed hash realization | `hash_inputs`, `hash_prefix`, `hash_materialized` | IX → Rust[Access]; each explicitly commits to its hash layout and schedule. |
| Hash callable host | `hash_materialized_query` | Same eager computation in an anonymous callable. |
| Configured relation realization | `relation_program`, `relation_query` | RA → Rust[Access, Native]; selected per-input/result representations and local algorithms. |
| Observed relation hosts | `observed_relation_program`, `observed_relation_query` | Same realization with explicit counters/checkpoints. |
| Pull execution | `pull`, `pull_set`, `pull_stream` | One Access → Pipeline lowering, with distinct-iterator, set or support-stream continuation. |
| Push execution | `push_set` | Access → eager Rust loops and exact-set sink. |
| Pipeline endpoints | `iterator_pipeline`, `collect_pipeline`, `stream_pipeline` | Pipeline → Rust; distinct iterator, exact set or pre-Distinct stream. |
| Candidates | `scan_rows`, `prune_rows` | Current-view handle enumeration; prune tests metadata before payload. |
| Key resource | `exact_keys`, `zone_keys` | Key construction and exact membership; zone also provides conservative rejection. |
| Witness resource | `hash_witnesses`, `nested_witnesses` | Coherent Build/Probe resource protocols for consumers that request them. |
| Pair algorithm | `hash_pairs`, `nested_pairs`, `sort_merge_pairs` | Complete equal-key pair enumeration, including both traversals and auxiliary storage. |
| Array rows | `hash_rows`, `sorted_rows`, `block_rows` | Concrete i64-array representations; block operations are provided only by block rows. |
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
- `hash_inputs`, `hash_prefix` and `hash_materialized` are concrete implementations;
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
  `push_set` can replace an eager exact-set hole, not a demand-sensitive iterator
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
