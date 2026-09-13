# Compiler modules and decision ownership

Physical composition now has declared contextual inputs in `cq-ir` and typed
compiler consumers. Its public RA convenience route composes two checked edges:
`compile_deferred_indexes` preserves the source and produces an empty
IndexRequirements overlay; `compile_composed` realizes that overlay and rejects
every written index requirement. Canonical indexed backends keep their own
coverage contracts. `physical_plan!` forwards configured IndexRequirements through
relative `mini_linq::composed!`; that independently replaceable macro parses the
typed physical edge. The compact revision passes the [recorded validation](CONTEXTUAL-PHYSICAL-LOWERING.md#implementation-and-validation); the [complete stage review](IR-STAGE-REVIEW.md) records remaining debt.

The order remains CQ → RA → IndexRequirements → physical Rust context → Access
→ Pipeline → Rust. `physical_context` refines the physical realization edge:
it supplies an existing RA definition under exactly its free Rust-value and
ordered-layout assumptions. Row bindings select a provider; projected-resource
bindings are interpreted by the selected recipe. It is serialized operator input,
not another whole-program relational algebra. Whole-operator control is chosen before its
Access region is lowered; provider calls inside native expressions may expand
later in the surrounding Rust context.

The [contextual lowering design](CONTEXTUAL-PHYSICAL-LOWERING.md) specifies the
necessity, grammar, semantic rules and pass correspondence. The
[boundary audit](PHYSICAL-BOUNDARY-AUDIT.md) records the defects that motivated
this migration. Per-binding providers and per-operator recipes remain independently
selected. Storage access uses operation traits with semantic laws; `cq-storage`
contains data structures without a query-wide relation interface.

## Module and crate boundaries

```text
crates/
  cq-ir/src/                    existing languages and local contracts
    physical_context.rs         retained RA definitions and contextual assumptions
    physical_provider.rs        typed Rust-fragment provider operations
  cq-compiler/src/
    passes.rs                   checked deferred-index and physical edges, pass wrappers
    cq_to_relational/           logical facts, rules, and strategy
    relational_to_indexes.rs    equality-index requirements
    physical/
      materialized.rs           existing per-result sets and equality indexes
      lazy.rs                   existing lazy realization over base indexes
      rust.rs                   shared Rust syntax construction
      options.rs                ordinary provider/recipe configuration, outside IR
      composed/
        mod.rs                  PhysicalOptions and composition entry
        layout.rs               admitted domain, fields and binding providers
        resources.rs            projected-relation construction and versioned reuse
        wiring.rs               lexical partial application of macro configuration
        lower.rs                operator invocation and materialization wiring
        api.rs                  per-input types, methods and query-local stats
        check.rs                source-to-invocation composition audit
      providers/
        mod.rs                  typed provider fragment entry exports
        native.rs               hash/sorted types, construction and access
        block.rs                block descriptors, selected access and masks
      semijoin.rs               exact and zone semijoin recipes
      joins.rs                  hash and nested-loop join recipes
    pull.rs                     Access to IteratorPipeline
    pipeline.rs                 iterator-valued and set-valued Rust endpoints
    pass_contracts/             independent source-target relations
  cq-storage/src/
    block_access.rs             block access, descriptors, bounds and construction
    blocks.rs                   immutable BlockStore and metadata descriptors
    selection.rs                exact coordinate selection and independent makers
    mask.rs                     independent BlockMask bitmap
    membership.rs               exact key membership
    summary.rs                  conservative coordinate summaries
    metrics.rs                  passive work counters
  mini-linq/src/
    lib.rs                      macro-boundary parsing and typed continuation wiring
    physical_input.rs           administrative whole-program options syntax
    recipe_input.rs             administrative provider/recipe entry prefixes
```

`cq-storage` exports `BlockStore`, `Block`, `BlockMask`, `ZoneSummary`, `Counters`
and `Work`. `BlockStore` has a metadata directory separate from payload chunks.
Descriptor enumeration and bounds lookup do not enumerate payload. The crate
has no `Relation`, `View`, witness index, semijoin executor or query statistics
container. Generated code owns the association of a block store with a mask,
algorithm sequencing, native collections and query-local statistics.

## Decisions and owners

| Decision | Owner |
|---|---|
| Logical schedule and YA reductions | CQ planner and its administrative rules |
| Empty-overlay selection and realization domain | `compile_deferred_indexes`, `compile_composed`, and `pass_contracts::composed` |
| Context grammar, operand scope and derived field roles | `cq-ir::physical_context` and its local contracts |
| Source type and access implementation per input | `PhysicalOptions.inputs` provider macro paths |
| Unit/Project/NaturalJoin result representation | `materialize` default and `results` overrides |
| Key projection and its consuming semijoin | `semijoin` default and `semijoins` overrides |
| Exact-key collection family | `keys` default and `key_sets` per-semijoin overrides |
| Natural-join construction and traversal | `join` default and `joins` overrides |
| Admitted domain and physical field positions | `composed::layout` |
| Versioned projected-resource reuse | `composed::resources` |
| Lexical macro configuration and optional instrumentation | `composed::wiring`, `composed::api`, and frontend entry prefixes |
| Concrete types, construction and access fragments | Selected provider macro |
| Layout-independent storage operations and semantic laws | `cq-storage` access traits and their implementations |
| Iterator versus owned-set endpoint | `iterator_pipeline!` versus `collect_pipeline!` |
| Correct invocation bindings and field maps | `composed::check` |
| Expanded types, methods and borrowing | Rust compiler |

These choices are Rust paths, including caller-owned aliases or macros. They
are not a closed capability enumeration. `hash_rows!` and `sorted_rows!` expose
ordinary `HashSet<[i64; N]>` and `BTreeSet<[i64; N]>`; `block_rows!` exposes
`BlockStore<N>` inputs and emits a local store/mask association for selected
versions. Requesting block descriptor operations from a supplied native row-set
provider produces a macro diagnostic. A replacement implementation can also
fail when Rust checks its emitted operations.

Rename inherits its input provider. Semijoin inherits its left provider; it
changes the selected set without requesting unrelated inputs to change storage.
Unit, Project and NaturalJoin have explicit construction choices. An authored
project can therefore materialize a block store for a later zone semijoin while
other results remain native sets.

## Storage operation traits

The [block interfaces](../crates/cq-storage/src/block_access.rs) separate three
operations. `BlockRead` enumerates metadata descriptors and identifies their owner;
its associated row, block-ID, row-ID, identity and borrowed descriptor types hide
the concrete layout. `BlockDescriptor` opens a particular descriptor's payload.
The optional `CoordinateBounds` supplies conservative integer bounds without
opening payload. Block access alone therefore does not imply zone-map support.
`BuildBlocks` separately packs an already distinct set for explicit construction.
Repeated traversal preserves each address-to-row association. The protocol needs
no independent random lookup, block-count helper or cached total row count.

The [selection interfaces](../crates/cq-storage/src/selection.rs) keep mask choice
independent of the store: `BlockSelection` is an exact coordinate set, and
`MakeSelection<S>` constructs empty or full selections from `S`'s metadata.
Multiple mask implementations can serve one store. Sparse identifiers need not
be converted to the supplied bitmap's dense offsets.
Address types require only `Copy`; identity additionally requires equality.
The selected HashSet endpoint imposes its own equality and hashing requirements
when collecting handles; those requirements do not belong to block access itself.

The [membership interface](../crates/cq-storage/src/membership.rs),
`ExactMembership<K>`, requires exact immutable lookup, including on borrowed
collections. `RangeSummary<K>` instead permits false positives and requires an
exact residual after its range guard. A Bloom filter returning possible membership
cannot satisfy the exact lookup law. A future Bloom access family can supply its
own operations without changing the meaning of these traits.

The block provider and resource recipes call these traits rather than the
custom data structures' inherent methods; native row providers retain standard
collection APIs.
The supplied data structures implement access directly in trait implementations,
without public inherent twins. `BlockStore::from_rows` remains an ingestion entry;
packing helpers and raw mask construction are private. The bitmap's block-length
bridge is crate-private, so an implementation helper does not become a semantic
operation merely to serve that implementation.
A layout change that preserves their laws changes the data-structure implementation
and, when selecting new types, the caller's provider alias. It does not require
changes to RA composition, Access/Pipeline lowering or algorithm recipes. A new
access model with different laws can still require a new provider or recipe;
these interfaces deliberately do not promise to unify every physical algorithm.

Rust checks the methods, associated types and borrowing. Implementation evidence
must additionally establish stable coordinate provenance, exact selections and
membership, bounds enclosure, and metadata access without payload opening.
Compiler evidence establishes guard order, current-mask restriction, exact
residuals and resource versions. Trait conformance alone proves none of those
semantic laws.

## Resource construction and access

For `K = heading(L) ∩ heading(R)`, the composer uses existing RA to decompose
`L semijoin R` into `q = project R keep K` and `L semijoin q`. A projected
resource denotes `q`. Its construction sees only `R`; reduction sees only `L`
and `q`. The complete-source checker verifies the projection identity and current
version instead of trusting a repeated origin annotation in the reduction input.
The RA-to-IndexRequirements edge still preserves the original RA exactly; the
physical realization edge owns and checks this local decomposition.

Resources are shared only for equal `(current right RA version, ordered
projection, recipe path, key-collection path)`. A recipe owns both the Project and
Semijoin lowerings, so different recipes can realize different resource shapes.
The zone implementation constructs exact keys and a conservative summary together.
A builder can borrow an existing exact key collection without a forced scan.
Exact and zone selections do not automatically share resources; macro paths are
compared syntactically without resolving Rust alias equality. The constructor no
longer receives a left consumer or its output name. Lawful immutable reads,
compatible lifetime and uniform resource meaning/shape across all uses are still
required. Witness groups for hash joins remain separate generated hash maps;
nested-loop join has no witness-map construction.

The supplied exact semijoin enumerates current rows and tests exact keys. The
zone recipe enumerates current block descriptors, rejects disjoint summaries,
opens accepted blocks and performs an exact residual. The block provider emits
mask-filtered row access and fresh mask construction. Handles carry explicit
store identity, coordinates and payload references. Descriptor access checks its
parent store before opening payload; selection checks store identity and current
mask membership. Separate store identity is necessary even for zero-column rows,
whose payload addresses need not distinguish allocations. Storage primitives
themselves do not know RA versions or reductions.
Recipes may filter, reorder or deduplicate the provider's handles, but must
preserve their identity, coordinates and borrowed row together. Fabricating or
recombining tuple fields violates this macro protocol; selection does not perform
a second payload lookup to validate an arbitrary fabricated handle.

`pull!` retains its iterator-valued contract. `collect_access!` uses the same
Access and Pipeline languages but chooses a set-valued Rust endpoint, folding
terminal Distinct into a HashSet sink. Composed materialization consumes that
owned set. The hash provider keeps it; other providers perform their selected
construction. `BlockStore::from_rows` establishes input uniqueness and bounds;
`BuildBlocks::pack_unique` packs distinct rows through private construction code
without another duplicate-elimination pass.

## Compatibility and validation boundary

`compile_blocked`, `compile_zonemap`, `compile_blocked_with`, `BlockOptions` and
`BlockChoices` remain uniform-block conveniences over `compile_physical`.
`blocked_plan!` and `zonemap_plan!` use `storage path; access path;` and optional
`keys path;`. Their all-block defaults do not constrain `physical_plan!`.

The source-target contracts separately check unchanged RA with the empty overlay,
the composed edge's admitted IndexRequirements domain, and the generated
invocation graph. The composition checker reconstructs source bindings, layouts,
provider paths and resource identities independently and parses declared contextual
inputs. It verifies original results, synthesized projections, actual resource
bindings, lexical wrapper definitions and dominance. The local contextual contract derives shared fields and output order from the
actual operand layouts; callers no longer supply copied join-column vectors or
output permutations. Neither checker expands arbitrary replacement macros or
proves their storage laws.
Typed Access and Pipeline entry points check their source domains before
lowering, including when called directly without a frontend macro. The supplied
recipes separately build checked Access regions; provider laws,
expanded Rust checking, execution parity and observed pruning provide distinct
evidence. A successful composition audit does not certify a caller's dishonest
bounds or an implementation that performs an unreported full scan.

Semantic calls contain no storage dependency, continuation path, key-family
choice or instrumentation field. `composed::wiring` captures those administrative
choices once in lexical wrappers. The plain `run` body emits no counters,
statistics cardinality requests or instrumentation adapters; `run_with_stats`
instantiates the same semantic calls with observations. Neither route consults a
runtime registry or a capability tag.

No automatic cost search, fallible rustc retry or universal storage trait is
implemented. The new contextual syntax has a stated physical-stage position and
conditional semantic contract; correctness still depends on provider and host
operation laws. See the [contextual lowering design](CONTEXTUAL-PHYSICAL-LOWERING.md),
[implementation guide](BLOCK-ACCESS-IMPLEMENTATION.md) and
[staging discussion](CONSTRAINED-STAGING-DESIGN.md).
