# Selected access and future pruning

The active boundary is `IX → Physical → Rust[Access]`, specified in
[the design](IX-PHYSICAL-REDESIGN.md). Providers are selected independently per
relation and index; an external lowering rule is selected for each RA
operation. There is no retained Native operation language.

## Implemented exact indexed joins

The selected `indexed_join!` rule consumes IX's right-index reference. It asks
the left stored provider for Rows, or composes an existing left Access region.
It passes the joined-column expressions to the exact index's Matches operation,
then asks the selected result provider to construct Output. The
[typed protocol](PROVIDER-PROTOCOL.md) records the Rows/Matches iterator source,
item pattern, logical fields and optional whole-row witness. The selected RA
rule constructs `=> bind PAT <- (EXPR)` clauses; providers do not join query
streams. Output returns only its carrier/field witness. Access describes
dependent bindings, with execution left to its consumer.

Hash/sorted example providers reuse `HashMap<K, Vec<V>>` or
`BTreeMap<K, Vec<V>>`. A match enumerates the payload vector with a concrete Rust
`get(...).into_iter().flatten()` expression. The already-bound indexed field and
the borrowed payload together denote the two-column row. No row clone, universal
row wrapper, or temporary reconstruction is needed to probe the index.

Rename translates between declared physical field order and sorted logical
heading ordinals. Only a demand corresponding to the map's physical first column
is admitted. A wrongly selected representation is rejected by emitted concrete
Rust type annotations; unsupported index columns are rejected during expansion.
The compiler does not silently scan or replace the selected provider.

Terminal Project uses the same Rows/Output protocol: it preserves the source
clauses, retains the projected logical fields, and delegates the result carrier
to its recorded provider. The [full-CQ trace](ix-physical/query-trace/review.md)
shows this selected rule after the joins. Intermediate Project is rejected;
retaining its old Access bindings would otherwise expose names already dropped
from the logical heading.

## Block pruning is a future provider family

YA exists in logical planning, but the new provider family does not implement
block-pruned Semijoin. The following obligation describes the needed extension;
it is not a claim about a currently generated zone-map query.

For a current relation version `v`, let `H(v)` be its live row handles and `p(h)`
be exact membership of the joined columns in the current right relation's
required index. Candidate traversal must satisfy:

```text
{h in H(v) | p(h)} subseteq Candidates(v, metadata) subseteq H(v)
```

Applying `p` to those candidates gives the exact semijoin. A zone-map rule must
inspect each block's conservative metadata before opening payload, and skip
payload when the metadata guard rejects. Overlap admits candidates and requires
an exact residual; it does not locate one key's unique block. Original immutable
bounds remain conservative for a selected subset, but traversal must honor the
current selection to avoid restoring removed rows.

`cq-storage` retains independent traits for block descriptors, coordinate bounds,
selections, exact membership and range summaries. Its tests cover these storage
laws. They do not establish a new Physical lowering's preservation theorem or
prove that a hash map supports block access. A future selected rule/provider
must supply that admission and its actual expansion evidence.

No common build/probe dialect or extra stage is introduced merely to add a data
structure. If an implementation needs retained physical operations for another
transformation, their syntax and semantics need explicit review first.
