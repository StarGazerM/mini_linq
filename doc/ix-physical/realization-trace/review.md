# Heterogeneous query: real expansion review

This report contains one query, from `physical_realization.rs::run`. It joins
`L(a)`, `R(a,b)`, and `S(b,c)` using a borrowed slice, hash map, and B-tree map.
`Value` deliberately has no `Clone` implementation. The Rust host signature is
written by the caller. The consumer for this fixture is explicitly
`access_to_set_loops`; this does not settle general push/pull placement.

The [complete raw report](steps.md) prints every actual input/output.
[Source hashes](source-hashes.json), [instrumentation](instrumentation.patch),
and [build output](compile.log) record how it was obtained. The source copy was
unchanged during capture. These are rustc proc-macro invocations, not a simulated
pass chain. Declarative policy expansion is in the separate [log](declarative.log).

The retained stages are RA → IX → Physical → Rust[Access]. Calls 003–036 are
administrative exchanges within realization. Their repeated `plan` and `state`
tokens carry compile-time continuations; no runtime relation interpreter is emitted.

| Call | Actual implementation | Effect |
| --- | --- | --- |
| [001](raw/2172042/001-index_demands.md) | `index_demands` | RA → IX: write i0=@r[0], i1=@s[0] and both join-use references. |
| [002](raw/2172042/002-given_physical_bindings.md) | `given_physical_bindings` | IX plus explicit policy assignments → Physical; preserve IX unchanged. |
| [003](raw/2172042/003-realize_physical.md) | `realize_physical` | Start realization: dispatch L to its selected slice provider. |
| [004](raw/2172042/004-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | Establish the borrowed slice L with its declared element type. |
| [005](raw/2172042/005-resume_physical.md) | `resume_physical` | Resume and dispatch R to its selected hash provider. |
| [006](raw/2172042/006-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Establish the borrowed hash relation R with declared column types. |
| [007](raw/2172042/007-resume_physical.md) | `resume_physical` | Resume and dispatch S to its selected sorted provider. |
| [008](raw/2172042/008-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Establish the borrowed B-tree relation S with declared column types. |
| [009](raw/2172042/009-resume_physical.md) | `resume_physical` | Resume and dispatch the selected Rename rule for l. |
| [010](raw/2172042/010-cq-example-providers::rename.md) | `cq-example-providers::rename` | Rename l delegates the result representation to Φ_R(l). |
| [011](raw/2172042/011-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | The selected slice provider establishes l from L. |
| [012](raw/2172042/012-resume_physical.md) | `resume_physical` | Resume and dispatch the selected Rename rule for r. |
| [013](raw/2172042/013-cq-example-providers::rename.md) | `cq-example-providers::rename` | Rename r delegates the result representation to Φ_R(r). |
| [014](raw/2172042/014-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | The selected hash provider establishes r from R. |
| [015](raw/2172042/015-resume_physical.md) | `resume_physical` | Resume and dispatch the selected Rename rule for s. |
| [016](raw/2172042/016-cq-example-providers::rename.md) | `cq-example-providers::rename` | Rename s delegates the result representation to Φ_R(s). |
| [017](raw/2172042/017-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | The selected sorted provider establishes s from S. |
| [018](raw/2172042/018-resume_physical.md) | `resume_physical` | Before j0 uses i0, dispatch that explicit index demand. |
| [019](raw/2172042/019-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Reuse r’s existing exact hash index; no auxiliary map construction. |
| [020](raw/2172042/020-resume_physical.md) | `resume_physical` | Resume and dispatch Λ(j0), the selected indexed-join rule. |
| [021](raw/2172042/021-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | Request Rows from the established l representation. |
| [022](raw/2172042/022-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | Return l’s iterator source, item pattern and logical field witness. |
| [023](raw/2172042/023-cq-example-providers::indexed_join_rows.md) | `cq-example-providers::indexed_join_rows` | Assemble l’s bind clause; use U(j0.right)=i0 and D(i0) to request Matches. |
| [024](raw/2172042/024-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Return the exact hash lookup source, item pattern and decoded (a,b) witness. |
| [025](raw/2172042/025-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | Assemble the dependent hash bind clause; request j0’s carrier from Φ_R(j0). |
| [026](raw/2172042/026-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | Return the borrowed pair carrier and unchanged logical field mapping. |
| [027](raw/2172042/027-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | Return j0 as an Access region to the common driver. |
| [028](raw/2172042/028-resume_physical.md) | `resume_physical` | Before j1 uses i1, dispatch that explicit index demand. |
| [029](raw/2172042/029-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Reuse s’s existing exact B-tree index. |
| [030](raw/2172042/030-resume_physical.md) | `resume_physical` | Resume and dispatch Λ(j1), the selected indexed-join rule. |
| [031](raw/2172042/031-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | Compose the prior Access region and request i1 Matches using b. No intermediate collection. |
| [032](raw/2172042/032-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Return the sorted lookup source, item pattern and decoded (b,c) witness. |
| [033](raw/2172042/033-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | Assemble the dependent sorted bind clause; request j1’s carrier from its selected provider. |
| [034](raw/2172042/034-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | Return the borrowed triple carrier and logical field mapping. |
| [035](raw/2172042/035-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | Return j1 as the completed Access region. |
| [036](raw/2172042/036-resume_physical.md) | `resume_physical` | Finish Physical → Rust[Access]: emit typed borrowed aliases and the explicit Access continuation. |
| [037](raw/2172042/037-access_to_set_loops.md) | `access_to_set_loops` | The caller-selected Access consumer emits ordinary Rust loops and the final set. |

The [completed Rust[Access] output](completed.rs) is call 036, before the
consumer expands. Calls 001, 002 and 036 match independently authored boundary
fixtures. The comparison parses the nested Access payload as well as Rust, so
punctuation spacing in `:-` does not masquerade as a language change.

The final consumer copies reference bindings when it nests loops; it does not
clone input payloads. Pointer assertions and the non-Clone schema check that
the output fields refer to the original inputs. Runtime equality is checked
against independent relation enumeration, separate from token agreement.

Rows/Matches providers return `source`, `item` and `witness`; the selected
RA rule constructs the `bind` clauses. Output providers return only witnesses.
The [separate pull query](../pull-trace/review.md) shows the neutral Access
bindings lowering to named Pipeline definitions before query-level `flat_map`
appears in Rust. Call numbers belong to each report independently.

Reproduce (choose a fresh output directory):

```sh
python tools/trace_macros.py --test physical_realization --package mini-linq \
  --macro-package cq-example-providers \
  --features index-demands,physical-selection,physical-realization,access-to-set-loops \
  --out /tmp/mini-linq-physical-trace --declarative -- \
  mixed_providers_preserve_join_results_and_borrow_non_clone_fields
CQ_REALIZATION_TRACE=/tmp/mini-linq-physical-trace cargo test -p mini-linq \
  --no-default-features \
  --features index-demands,physical-selection,physical-realization,access-to-set-loops \
  --test physical_realization captured_boundaries_equal_authored_ix_physical_and_access -- --ignored
```
