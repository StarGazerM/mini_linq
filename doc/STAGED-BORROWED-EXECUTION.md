# Borrowed heterogeneous realization

The current implementation realizes an open Rust expression through
`RA → IX → Physical → Rust[Access]`. The
[runnable composition example](../examples/composition/README.md) supplies the
host function and immutable inputs. See the [design](IX-PHYSICAL-REDESIGN.md)
and [provider protocol](PROVIDER-PROTOCOL.md) for the contracts.

| Object | Concrete example |
|---|---|
| First input | `&[T]` with `slice_rows!` |
| Second input/index | `&HashMap<K, Vec<V>>` with `hash_relation!` |
| Third input/index | `&BTreeMap<K, Vec<V>>` with `sorted_relation!` |
| Result carrier | `borrowed_fields!`, or a caller-defined Output macro |
| Binary RA rule | `indexed_join!` selected independently for each result |
| Terminal projection rule | `project!` delegates the projected carrier to the recorded result provider |
| Access consumer | Explicit caller import, `access_to_set_loops!` in the runnable example |

IX names the exact index for each join position. Physical records providers and
rules before realization starts. The driver carries those choices through
administrative callbacks without interpreting provider names. Direct Rename
asks the selected result provider to establish a typed borrowed alias. Index
realization reuses the existing map when its physical first column corresponds
to the written IX columns.

Rows and Matches return Access fragments plus logical field expressions.
Results retain references to caller-owned values; neither the driver nor the
example providers require payload `Clone`. A previous left join result remains
Access syntax and can be composed into the next join. It is not allocated as a
runtime relation or implicitly collected. Indexing such an intermediate region
is unsupported today and rejects.

The final region must expose the normalized logical heading in sorted order;
current output metadata uses that same order. A caller-defined result provider
can choose a nested carrier while preserving that field map. Borrowing and
lifetimes remain checked by Rust.

The integrated tests exercise independent join oracles, non-Clone field pointer
identity, empty/missing matches, mixed providers, reversed Rename ordinals,
caller-defined macros, name hygiene and incompatible selections. Actual
expansion checks distinguish the initial Physical call from its many provider
callbacks and the completed Access result.

```sh
cargo test -p mini-linq --no-default-features \
  --features index-demands,physical-selection,physical-realization,access-to-set-loops \
  --test physical_realization --test physical_variants
```

Shared replay/index storage still has independent tests in `cq-storage`, but
this new provider family does not select it. Semijoin/YA, zone-map pruning,
materialized intermediates, nonterminal projection, reordered/computed output
metadata and generated host APIs
remain migration work. Existing Access/Pipeline consumers retain their separate
lazy/eager observation contracts; their placement in the redesigned compiler is
not settled by this example.

A separate [full-CQ trace](ix-physical/query-trace/review.md) starts at the
written-order planner and includes its terminal projection.
[Full-CQ tests](../crates/mini-linq/tests/physical_query.rs) compare projected
results with independent relational enumeration over 256 input subsets;
[stored-projection tests](../crates/mini-linq/tests/physical_projection.rs) cover
the selected Rows request. Intermediate projection rejects explicitly.
