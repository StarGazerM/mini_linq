# Heterogeneous physical composition and block access

Status: compact contextual inputs and checked typed stages are implemented;
the compact revision passes its final validation. The
[contextual lowering design](CONTEXTUAL-PHYSICAL-LOWERING.md) specifies its grammar,
semantic rules and stage placement. The [boundary audit](PHYSICAL-BOUNDARY-AUDIT.md)
records the earlier undeclared protocols and bypass that motivated this change.

`physical_plan!` composes exact integer RA with independently selected storage,
semijoin and join implementations. A query can borrow a `BlockStore`, a native
`HashSet` and a native `BTreeSet` at the same time. Only bindings assigned a block
provider use block storage or masks. `cq-storage` supplies data structures;
compiled code owns traversal, construction and query execution. Rust operation
traits define the storage semantics on which that compiled code relies.

## Select providers and algorithms

The [blocked-ya example](../examples/blocked-ya/main.rs) configures its authored
RA using this wrapper:

```rust
macro_rules! configured {
    ($reduction:path; $($ra:tt)*) => {
        ::mini_linq::physical_plan! {
            storage ::cq_storage;
            access ::mini_linq::collect_access;
            materialize ::mini_linq::hash_rows;
            inputs {
                R => ::mini_linq::block_rows;
                S => ::mini_linq::hash_rows;
                T => ::mini_linq::sorted_rows;
            }
            semijoin ::mini_linq::exact_semijoin;
            semijoins { r1 => $reduction; }
            keys ::std::collections::HashSet;
            key_sets { r1 => ::std::collections::BTreeSet; }
            join ::mini_linq::hash_join;
            $($ra)*
        }
    };
}
```

The example's facade alias chooses `exact_semijoin` or `zone_semijoin` for `r1`.
`S` and `T` retain their independent native types. Its joins and projections use
the configured hash-set materialization default. `results { live => provider; }`
overrides an individual Unit, Project or NaturalJoin result, and
`joins { joined => ::mini_linq::nested_loop_join; }` selects another complete
join recipe. Rename inherits its input provider; Semijoin inherits its left
provider. Each input must appear exactly once in `inputs`; result/operation
maps may be omitted or empty and may name only the corresponding operators.

Generated `run` and `run_with_stats` methods take borrowed, provider-selected
input types and a final `NonZeroUsize` block-size argument. That argument affects
explicit block materialization; an all-native query does not acquire a block
layout. The generated `<Query>Stats` owns per-result observations and final-output
work. `run` emits a separate body without counters, statistics cardinality
requests or instrumentation adapters. `run_with_stats` uses the same semantic
operator calls under instrumented lexical wrappers. There is no shared runtime
query object.

Providers and recipes are macro paths, including caller-owned forwarding macros.
The compiler does not inspect a `SupportsBlocks` tag. The supplied native providers
reject a requested descriptor operation during macro expansion. Other incompatible
compositions can fail ordinary Rust type or borrow checking. Automatic search and
retry after a later rustc failure are not implemented.

For intentionally uniform blocks, `blocked_plan!` and `zonemap_plan!` remain
conveniences accepting `storage path; access path;` and optional `keys path;`.
Typed callers can use `BlockOptions`, `BlockChoices` and `compile_blocked_with`.
`compile_physical` is an RA convenience that composes `compile_deferred_indexes`
and `compile_composed`. The first preserves the complete source in an empty
IndexRequirements overlay. The second accepts only a well-formed empty overlay;
every written index requirement is rejected rather than ignored. Selected recipes
own their auxiliary construction and sharing. Canonical indexed routes keep their
original contracts. The public `physical_plan!` stage forwards the configured
IndexRequirements input to relative `mini_linq::composed!`; a caller facade can
replace that physical continuation with a compatible implementation.

Operator semantic inputs have the declared shape `Definition { Binding* }`.
A binding is `Atom [=> Path] { Expr };`: rows name a configured provider and a
projected-key resource omits it. The braces permit caller-owned declarative macros
to capture a provider as a Rust `path`. Local contracts derive positions and
canonical output layouts from the existing RA operation and ordered bindings.
Only free operands appear; source configuration and instrumentation are separate.

A key resource realizes an existing RA projection `q = project r keep { k };`;
its reduction is `result = semijoin l with q;`. The constructor receives only `r`
and the reduction only `l` and `q`. No phase suffix or duplicate origin assertion
remains. The complete-source checker verifies this local algebraic decomposition,
resource construction and use. Provider semantic inputs are
`physical_provider::Operation`, with no storage path or work argument. Type widths
use the same unsuffixed `Column` syntax as IndexRequirements.

`composed::wiring` emits lexical wrappers once per distinct selection. Direct
public entries accept `storage Path; [work (Expr);] Operation`, or
`storage Path; access Path; [keys Path;] [work (Expr);] Context` for a recipe.
These administrative prefixes are parsed in `mini-linq`; compiler consumers
receive a typed source and ordinary options separately. There is no query grammar
parser in the physical compiler modules and no global name registry.

## Construction, access and exactness

1. Check RA and preserve it in an empty IndexRequirements overlay. Check that
   realization receives exactly that domain, then validate the `i64` array domain,
   field positions and option scopes. Build a provider binding for every result.
2. Group semijoin demands by `(current right result, ordered separator, recipe
   path, key-collection path)`. Emit a fresh Project definition through the
   selected recipe at first use. Its one binding is the current right input;
   the produced resource denotes that projected relation.
3. Emit a Semijoin definition using the current left binding and projected
   resource. The source semijoin is preserved by the projection identity.
   The exact recipe uses selected rows → exact keys → survivor handles. The zone
   recipe uses descriptors → summary guard → accepted block rows → exact keys.
4. Fix operator control and lower its regions through existing Access and Pipeline.
   Provider calls in native expressions can expand later in the Rust context.
   The block provider constructs a fresh mask after checking current-mask
   membership and store identity; native providers produce their native sets.
5. Dispatch each Project/Unit construction and whole join recipe independently.
   Hash joins generate witness-group construction and probing. Nested-loop joins
   generate direct traversal. The selected result provider owns materialization.
6. Audit the generated composition's invocation graph, versions, fields and
   paths; Rust checks expanded types and borrowing. Recipe and storage laws
   remain separate correctness obligations.

Exact keys must denote `S = { kr(r) | r ∈ R }` for the current right result.
A zone summary may reject block `b` only when its bounds cannot contain a key in
`S`; overlap merely admits candidates. Every admitted row receives the exact
whole-key test. Empty `S` differs from a singleton empty key, preserving
zero-column existence semantics. The zone build drains its source iterator after
collection to complete the summary even with a replacement collector.

The composed route shares resources at equal source versions, separators, recipe
paths and collection paths. Correct sharing additionally requires lawful immutable
builders, compatible lifetimes and uniform consumability by every context in the
class. Construction no longer receives any left consumer or output-name context.
Resource shape and summary meaning must still remain compatible across uses;
exact keys alone do not establish that property. Path
equality does not prove these laws. A different RHS version needs another resource.
Exact and zone builders have different resource shapes and are not automatically
merged into one construction. Join witness groups are not shared with semijoin
membership resources.

## Data structures and compiler ownership

- [BlockRead and BlockDescriptor](../crates/cq-storage/src/block_access.rs)
  separate metadata enumeration from payload opening. Associated row, block-ID,
  row-ID, identity and borrowed descriptor types hide storage layout. Optional
  `CoordinateBounds` adds metadata-only enclosing intervals; `BuildBlocks`
  separately constructs blocks from an already distinct set.
- [BlockSelection and MakeSelection](../crates/cq-storage/src/selection.rs)
  describe an exact coordinate set and its metadata-only construction. Selection
  choice belongs to the caller: one store can admit several masks, and IDs need
  not be dense integers.
- [ExactMembership](../crates/cq-storage/src/membership.rs) supplies immutable
  exact key lookup for standard, borrowed and caller-defined collections.
  `RangeSummary<K>` supplies conservative range rejection. Possible membership
  cannot substitute for the exact residual required by the semijoin recipe.
- [BlockStore](../crates/cq-storage/src/blocks.rs) owns immutable, deduplicated
  chunks and a separate metadata directory. Trait operations enumerate metadata
  and inspect bounds; a descriptor's `rows` explicitly opens payload.
- [BlockMask](../crates/cq-storage/src/mask.rs) is an independent bitmap over a
  fixed block shape. It has no relation identity, source-version knowledge or
  query execution. The compiled provider owns its association with storage.
- [ZoneSummary](../crates/cq-storage/src/summary.rs) and passive counters are
  reusable primitives. Storage methods do not update query work observations.
- [Composition](../crates/cq-compiler/src/physical/composed/mod.rs) owns options,
  binding layouts, resource scheduling and generated APIs.
  [Providers](../crates/cq-compiler/src/physical/providers/mod.rs) separate
  [native](../crates/cq-compiler/src/physical/providers/native.rs) and
  [block](../crates/cq-compiler/src/physical/providers/block.rs) operation expansion.
  [Semijoin recipes](../crates/cq-compiler/src/physical/semijoin.rs) and
  [join recipes](../crates/cq-compiler/src/physical/joins.rs) own their algorithms.

There is no runtime `Relation`, selected `View`, witness-index wrapper or
semijoin executor. A block descriptor can skip its payload because of its actual
storage layout; ordinary hash and sorted inputs do not undergo a relation-wrapper
conversion. `BlockStore::from_rows` deduplicates ingestion in first-occurrence
order. `BuildBlocks::pack_unique` packs the compiled distinct set; `BlockStore`
implements it through private packing code without rededuplicating it.
Generated block handles carry store identity separately from coordinates and
payload references. Descriptor access verifies its parent before opening payload;
selection verifies store identity and current-mask membership. This also rejects
cross-store handles for zero-column rows, whose payload addresses can coincide.
Recipes can filter, reorder or deduplicate these provider-produced handles; they
must preserve each handle's identity, coordinates and row reference together.
Invented or recombined tuples are outside the protocol. Selection needs no second
payload lookup: repeated descriptor traversal must preserve address-to-row identity.

The block provider calls these traits with qualified paths. Changing chunk
storage, descriptor representation or coordinate types while preserving the
laws requires new trait implementations and type selection in a caller provider
alias. The RA composer, Access/Pipeline stages and semijoin/join recipes retain
their code. This block protocol borrows whole rows; columnar, compressed or
asynchronous access can require a different provider, recipe or operation trait.
Native providers continue to use the standard collections' own APIs.

The supplied stores, descriptors, masks and summaries expose their query
operations through trait implementations, with no public inherent duplicates.
`BlockStore::from_rows` remains public for ingestion. Packing and raw mask-shape
construction are private; the stock bitmap's length bridge is crate-private.
No block-count, cached total-count or random-lookup API is promoted into a trait
to support a helper. Address types require only `Copy`; identity also requires
equality. Equality and hashing bounds needed to collect handles belong to the
selected collection endpoint.

The trait laws are part of the correctness argument: directory traversal,
bounds and mask creation must avoid payload opening; IDs and payload remain
stable while borrowed; equal identities denote the same immutable address space;
repeated descriptor traversal preserves row references; selections and key lookup are exact;
bounds and range rejection are conservative. Rust checks operation availability,
associated types and lifetimes. Constructors, implementation review and tests
must establish these semantic laws. The compiler remains responsible for source
versions, mask restriction, guard-before-open order and the exact residual.

`collect_access!` and `collect_pipeline!` retain their owned-set contract;
`pull!` and `iterator_pipeline!` remain iterator-valued. Projection and
many-to-many joins still need set collection. A macro alias with compatible
syntax must also provide the required result, evaluation and ownership behavior.

## Validation and remaining scope

```sh
cargo run -p mini-linq --example blocked-ya --locked
cargo test -p cq-storage -p cq-compiler -p mini-linq --lib --tests --locked
cargo test -p mini-linq --test heterogeneous --test block_access --test provider_provenance --test storage_traits --release --locked
```

[Heterogeneous acceptance cases](../crates/mini-linq/tests/heterogeneous.rs)
compare exact, zone and alternate-join routes against an independent oracle over
4,096 binary-domain databases. They also exercise per-result materialization,
caller-owned provider macros, selected-row non-resurrection, current RHS keys,
composite-key false positives, native-only execution, zero-column resources and a
caller-owned recipe borrowing an existing key set without a forced scan.
Storage tests exercise primitive bounds/masks separately; compiler tests cover
composition rejection. The commands above reproduce these separate checks.
[External-consumer privacy checks](../crates/cq-storage/tests/privacy.rs) compile
trait-based access and opaque address types, then separately reject access to
private storage details, method calls without required trait imports, and removed
random-access and raw-construction APIs. These checks exercise Rust visibility
and operation availability; they do not prove semantic laws.

[Storage-trait substitution cases](../crates/mini-linq/tests/storage_traits.rs)
use a [flat payload with sparse IDs and a sparse mask](../crates/mini-linq/tests/support/alternate_blocks.rs)
beside the supplied `BlockStore` in one query. The unchanged block provider and
semijoin recipes match an independent oracle on 1,024 databases. Independent
instrumentation observes zero descriptor payload opens for rejected metadata,
where the exact route opens three blocks; this counter measures descriptor openings.
Further cases check reduction preservation, trait-only materialization and
zero-column provenance. Caller-defined exact keys and range summaries implement
the operation traits; deliberately panicking inherent constructors establish that
qualified trait calls cannot be shadowed. An alternative summary retains
key correlations and rejects one block admitted by independent coordinate ranges;
the unchanged zone recipe returns the same answers with one fewer exact probe.

The compact revision passes 398 workspace tests, 37 focused release integration
tests, workspace Clippy with warnings denied, formatting, all five formal examples,
and both manuscript builds. Sixteen compiler boundary cases also pass in debug
and release. The complete [IR/stage review](IR-STAGE-REVIEW.md) records remaining
Pipeline and administrative-interface debt. IR tests cover binding/resource contracts and the
configuration-free provider grammar. Stage tests
establish exact RA preservation and rejection of written index requirements;
macro tests verify independent continuation aliases, borrowed builders, operand
evaluation counts and optional instrumentation. Provider tests compare observed
and unobserved access and check one evaluation of each used counter expression. Storage and algorithm laws remain explicit assumptions.

Counters report logical payload visits, opened/skipped blocks, exact probes and
build rows. Ingestion, bounds construction and mask bookkeeping are outside
these counters. They do not establish disk I/O savings or speedup.

Bloom filters, persistent block storage, automatic cost selection, cross-recipe
resource sharing and the full proposed PA-* administrative overlay remain outside
this implementation. The [module design](COMPILER-STRUCTURE.md) and
[formal supplement](formal/README.md) distinguish these proposals from supplied code.
