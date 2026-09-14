# Access refinement and physical choices

The direct physical route lowers existing RA into Rust containing Access plans.
Its environment Γ is compiler state, not a serialized language or an extra stage.
The complete boundary review is in [IR-STAGE-REVIEW.md](IR-STAGE-REVIEW.md);
[the supplement](formal/access_refinement.tex) gives the rules and algorithms.

## Stage placement

```text
CQ → RA → IndexRequirements → Rust[Access] → Rust[Pipeline] → Rust
RA ── configured lowering with compiler-local Γ ──→ Rust[Access]
```

The second edge ends at `Rust[Access]`. The canonical index route records and
realizes named equality-index demands. The direct route accepts RA itself and
constructs local resources as needed. It does not manufacture an empty index
stage or accept and discard written demands.

RA denotation needs a database valuation η. Lowering additionally needs Γ:
physical layouts, selected provider paths, the Rust value representing each
current RA definition, and shared resource bindings. These facts already belong
to a whole-program lowering pass. No RA definition, logical source name, or
proof annotation is duplicated in a local macro input.

The direct implementation currently admits `i64` array rows; Unit, Rename,
Project, Semijoin and NaturalJoin; and variable-only output with at most twelve
tuple fields. Other valid RA uses another physical edge or gets a diagnostic.
This restriction belongs to this implementation, not to RA or Access.

## What the compiler decides

The configuration supplies ordinary macro paths, with defaults and per-result
overrides. It has no catalogue of accepted data-structure capabilities.

| Choice | Configuration | Owner |
| --- | --- | --- |
| Storage trait definitions and Access endpoints | `storage`, `access`, `iterator` | Generated lexical wrappers and continuations |
| Input representation | `inputs` | Provider for each input relation |
| New result representation | `materialize`, `results` | Unit, Project and NaturalJoin result construction |
| Semijoin candidates | `scan`, `scans` | `scan_rows` or `prune_rows`, or a caller macro |
| Exact keys and optional metadata | `key_provider`, `key_providers` | `exact_keys` or `zone_keys`, or a caller macro |
| Key collection family | `keys`, `key_sets` | A lawful exact collection such as HashSet or BTreeSet |
| Join witnesses | `join`, `joins` | `hash_witnesses` or `nested_witnesses`, or a caller macro |

Rename takes its input provider. Semijoin inherits its left provider because it
selects a current view. Result-provider overrides therefore apply only to
materializing operators. These are operation dependencies, not global replacement
of every relation. Lexical wrapper macros bind the configured services once and
forward typed semantic arguments to the selected implementation.

## Compact native operations

The existing provider fragment language is declared in
[`physical_provider.rs`](../crates/cq-ir/src/physical_provider.rs). In addition to
row/storage operations, it includes:

```text
build { AccessPlan }
contains (resource) (key)
may_match (resource) (bounds) (columns)
probe (resource) (key)
candidates ProviderPath { view } (predicate)
```

`build` consumes an ordinary Access plan. It is neither a new relational operator
nor a textual specification of a runtime query. `contains` tests an exact key;
`may_match` tests conservative metadata; `probe` enumerates matching witnesses.
`candidates` supplies the current view and a metadata predicate to a selected
traversal. Storage paths, continuations, collection types and optional counters
are administrative configuration outside these semantic arguments.

A semijoin `s = l ⋉ r` is compiled once into this shape (schematic local names):

```text
keys = key_provider! { build { project shared fields from scan(current r) } }
handles = iterator! {
    for h in scan_choice! {
        candidates left_provider { current l }
        (|bounds| key_provider! { may_match (&keys)(bounds)(left positions) })
    }
    if key_provider! { contains (&keys)(left key at h) }
    yield h
}
s = left_provider! { select (current l)(handles) }
```

The actual source is [direct/lower.rs](../crates/cq-compiler/src/physical/direct/lower.rs).
The schematic `project` is an Access output expression, not an inserted named RA
Project. Keys are constructed from the **current RHS definition**, including its
existing selection. A resource is shared exactly when current RHS identity,
ordered separator, key-provider path and collection-family path agree. Scan
choice is not resource identity: a single zone resource can serve row scans and
block scans. Distinct RHS versions do not share by mere common ancestry.

## How keys reject blocks

`exact_keys` builds an exact collection. `zone_keys` builds that collection and a
coordinate-wise summary of its keys. The latter does not map keys directly to
blocks: it compares the key summary against each block's stored bounds.

For shared fields `c₀,…,cₖ₋₁`, block bounds `B`, and nonempty key summary `Q`,
`may_match` is the conjunction of interval overlap at each paired coordinate:

```text
Q[j].min ≤ B[c[j]].max  and  B[c[j]].min ≤ Q[j].max
```

An empty key resource rejects every block. A nonempty zero-column key resource
admits every nonempty current block: its key is the empty tuple and a semijoin
then means RHS existence. Coordinate overlap is only a necessary condition;
keys `(0,1)` and `(1,0)` cannot establish membership of `(0,0)`. The exact residual
always checks the complete tuple.

`scan_rows` discards the metadata predicate tokens before expansion and returns
all current handles. Thus exact-only keys and native row providers work without
implementing metadata operations. `prune_rows` emits another local Access plan:

```text
for block in provider! { blocks (current_view) }
if metadata_predicate(provider! { bounds (block) })
for handle in provider! { block_rows (block)(current_view) }
yield handle
```

The metadata guard precedes payload access. `block_rows` respects the current
selection; a later reduction cannot resurrect an earlier rejected handle. Bounds
may still describe the original immutable block, since those bounds enclose all
of its surviving rows. Tightening metadata is an optional optimization.

## Correctness obligation

Let `H(v)` be the handles selected by the current view, `row(h)` their values,
`K` the exact projection of the current RHS, and `p(h)` mean `key(row(h)) ∈ K`.
A candidate traversal `C(v,m)` must enumerate each handle at most once and satisfy:

```text
{ h in H(v) | p(h) } ⊆ C(v,m) ⊆ H(v)
```

The selected output is `{h in C(v,m) | p(h)}`. These inclusions prove it equals
`{h in H(v) | p(h)}`, hence the semijoin. The law belongs to the compound candidate
traversal plus its exact residual: an arbitrary metadata predicate is not safe.
Candidate handles must remain valid for the current view until selection. Block
views retain payload and store identity; native sets may copy survivors into new
storage. Current-mask access, conservative bounds and exact membership are
provider laws. Rust trait checking establishes available operations and types;
it cannot establish these semantic laws automatically.

Selecting `prune_rows` with a native HashMap-style provider fails when its block
operations are expanded or type checked. Selecting it with `exact_keys` fails
because that key provider has no `may_match`. There is no fallback that silently
changes the requested access, and no runtime relation interpreter.

## Join and endpoint choices

Join lowering emits a `(key,row)` Access construction and an Access probe for
each left key. `hash_witnesses` builds exact key-to-row buckets; `nested_witnesses`
builds a closure without scanning and evaluates the source Access plan at each
probe. Both enumerate precisely the matching RHS rows, under immutable provider
and host-expression laws. Their construction and traversal costs differ.

The ordinary iterator endpoint realizes Pipeline distinctness. `collect_access`
fuses terminal distinctness into its set sink. `stream_access` uses the same
checked Access-to-Pipeline pass, then an endpoint that omits a sole-use terminal
Distinct and promises the same **support** (set of produced values), allowing
duplicates. Exact collection preserves support under repeated keys, and a lawful
summary continues to cover every observed key; the stock coordinate summary is
also idempotent. Semijoin selection consumes handles directly into its mask or
set without an intermediate handle HashSet. For candidates and witnesses, the
stock endpoint preserves the pre-Distinct sequence, so unique provider enumeration
stays unique. Support equality alone would not ensure that property for an
arbitrary replacement endpoint. This endpoint adds no IR.

## Limits

This is explicit physical selection, not a cost optimizer. Bloom filtering,
approximate predicate-transfer scheduling, automatic backend search, factorized
joins, bags and general host-expression RA are outside this direct implementation.
The existing native Access example demonstrates input-dependent bitmap/hash
selection, not an implemented cost policy. Representation-specific traits stay
minimal and separate; shared names do not promise that all representations support
the same operation. The [storage guide](BLOCK-ACCESS-IMPLEMENTATION.md) records the
concrete traits and reproduction commands.
