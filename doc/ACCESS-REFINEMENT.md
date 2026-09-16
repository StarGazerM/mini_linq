# Native access and relation realization

Configured relation realization lowers existing RA into Rust containing local
Access and Native macro calls. It supports typed headings, selections, computed
bindings and output expressions. Its environment Γ contains layouts, selected
macro paths, current views and resource identity; Γ is compiler state, not an
extra serialized IR.

```text
RA → Rust[Access, Native]
Access → Pipeline → Rust          or Access → eager push loops
Native → selected Rust operations (possibly containing Access)
```

The equality-demand route remains independently useful: IX describes named
index requirements consumed by the concrete hash realizations. The configured
relation route accepts RA directly; it does not manufacture empty IX or ignore
written demands. See [the boundary specification](IR-STAGE-REVIEW.md) and
[formal rules](formal/access_refinement.tex).

## Representation and algorithm are separate choices

A query can use block rows for a fact relation, tuple hash rows for a dimension,
and tuple sorted rows for a materialized result. The provider owns `type`,
`field`, `record`, handles, traversal, selection and construction. Logical
headings and column types remain in RA. Array providers impose their i64
restriction locally; tuple providers accept heterogeneous Rust fields subject
to their generated collection's type requirements.

`hash_pairs`, `nested_pairs` and `sort_merge_pairs` implement a complete binary
pair producer. Their input expressions and key functions come from resolved
RA headings. The body emits the selected result record through `produce`.
The algorithm owns both traversal schedules and auxiliary structures; the
provider owns output storage and uniqueness. Sort-merge explicitly sorts by
separator keys, because row ordering does not establish that key ordering.

The pair law preserves input occurrences before the result sink removes
duplicates. It assumes finite stable traversal and pure keys. Different
algorithms need not preserve arbitrary host-effect order. Whole-query
materialization placement is still owned by the relation realization, rather
than inferred from the pair algorithm or provider name.

Witness Build/Probe operations remain available for other native consumers.
They are not the configured natural join's replacement boundary.

## Zone metadata and exact semijoin

For a semijoin `L ⋉ R`, lowering constructs the exact projected key resource K
from the current R view. With zone keys it additionally computes a conservative
coordinate range summary. A zone does not map one key directly to a block.

Pruning traverses blocks from the current L view, tests metadata, and only then
opens selected payload:

```text
for block in provider.blocks(current_view):
    if keys.may_match(provider.bounds(block)):
        for handle in provider.block_rows(block, current_view):
            if keys.contains(project(provider.row(current_view, handle))):
                retain(handle)
```

Disjoint ranges reject a block safely. Overlap only admits candidates; exact
membership remains necessary. A block's original immutable bounds can describe
its selected subset conservatively. Current-view traversal prevents a later
reduction from resurrecting previously removed rows.

Let H(v) be the handles in v and p(h) mean exact key membership. Candidate
traversal C must enumerate without duplicate handles and satisfy:

```text
{h ∈ H(v) | p(h)} ⊆ C(v, metadata) ⊆ H(v)
```

Applying the exact residual yields precisely the semijoin. Handle validity,
current-mask preservation, conservative bounds and exact membership are semantic
laws of the selected implementations. Rust checks operations, types and
lifetimes, not those mathematical laws.

`scan_rows` ignores its unused metadata predicate and enumerates the current
view. Thus exact-only keys work with scan. `prune_rows` expands the predicate and
block operations: exact-only keys reject `may_match`, and hash/sorted providers
reject block access. The compiler neither infers capabilities from names nor
silently replaces an explicitly chosen representation.

## Resources, ownership and configuration

Key-resource reuse requires identical current RHS result, ordered key coordinates
and selected macro binding. A different RA version or differently configured
binding gets a separate resource. Path identity is conservative, not an attempt
to prove two wrappers equivalent.

Configured key construction and membership pass borrowed field expressions to
the existing `key (...)` operation. Exact keys retain a tuple of references;
zone keys require referenced `i64` coordinates and copy those values into their
key arrays. Both compare represented values, not reference identity. Join key
expressions also borrow fields. Independently authored pair producers may instead
return owned keys for owned input items, subject to Rust's borrowing rules.

Set-provider `select` retains the supplied row references rather than cloning
their payload. Selecting again may borrow an earlier view; generated bindings
keep that view and its underlying owner alive through their consumers. Block
selection likewise retains payload through its current mask. Selection must
still contain only rows from the current view and preserve their represented
values; a collection of references does not relax that law.

Expression aliases borrow both input and computed fields. A Rust expression
that consumes a field must express its copy or clone; reading a computed field
does not implicitly clone it. Constructing a new owned record or answer remains
an ownership boundary and may clone the fields it uses. These rules do not claim
that every query avoids cloning or that projection removes all earlier work.

Candidate and key choices remain independent. Configured wrapper macros close
over iterator, collection, storage and packing expressions. The relation lowerer
passes operation operands without expanding them early. This preserves discarded
operands, lexical captures and native operation-specific configuration checks.

`materialize owner view (rows)` consumes an iterator. `produce owner view via
emit { body }` writes through an emission function directly into the provider's
result collection. Each binds storage that outlives its derived view. Block
construction receives unique rows through `BuildBlocks::pack_unique`; the trait
accepts any iterator carrier. Packing size is evaluated once per construction,
after rows, not once globally for every relation.

Plain and observed item/callable hosts share the same RA lowering. Observed
wrappers add the actual counter expression; it does not choose a provider.
Counters measure logical visits, not disk I/O. Callable hosts capture ordinary
Rust values and return owned answers.

## Execution observations

`pull` returns a distinct iterator through Pipeline. `pull_set` selects eager
collection; `pull_stream` preserves the pre-Distinct sequence for a consumer
that owns uniqueness. `push_set` compiles Access directly to nested Rust loops
and an exact-set sink. It can replace a set-valued Access hole, but cannot satisfy
a demand-sensitive iterator hole by merely accepting the same tokens.

The checked Pipeline language permits fresh stream names, selected/reordered
bindings and early projections. It remains linear and single-consumer; arbitrary
DAG sharing would need explicit ownership semantics.

This is explicit implementation selection. Automatic cost search, Bloom filters,
region fusion, factorized joins and bag semantics are not implemented by these
boundaries. Their absence is not hidden behind a universal relation API.
