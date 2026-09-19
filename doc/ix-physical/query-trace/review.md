# ProjectedChain: full CQ expansion

This report contains only `physical_query.rs::run`:

```text
answer(a,c) :- L(a), R(a,b), S(b,c);
```

The caller supplies a borrowed slice, hash map and B-tree map. The selected
planner is `cq_to_ra_written_order!`; the explicit policy chooses providers for
each resulting RA version and index, plus a lowering for every definition.
The caller writes the Rust function signature and selects the set consumer.

The retained boundaries are **CQ → RA → IX → Physical → Rust[Access]**.
Calls 004–041 are administrative exchanges inside realization. Project is an
existing RA operator; its selected external rule uses the existing Output
protocol. This change adds no IR or stage. Intermediate projection is rejected
until discarded-variable scope is handled explicitly.

[All actual inputs and outputs](steps.md) · [Completed Access output](completed.rs)
· [Source hashes](source-hashes.json) · [Capture verification](verification.txt)

| Call | Actual implementation | Effect |
| --- | --- | --- |
| [001](raw/2172002/001-cq_to_ra_written_order.md) | `cq_to_ra_written_order` | CQ → RA: Rename, natural joins, then final Project. |
| [002](raw/2172002/002-index_demands.md) | `index_demands` | RA → IX: i0=@r1[0], i1=@r3[0], with r2.right and r4.right uses. |
| [003](raw/2172002/003-given_physical_bindings.md) | `given_physical_bindings` | Explicit assignments → frozen Physical. |
| [004](raw/2172002/004-realize_physical.md) | `realize_physical` | Dispatch first host input L. |
| [005](raw/2172002/005-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | Establish borrowed unary slice L. |
| [006](raw/2172002/006-resume_physical.md) | `resume_physical` | Dispatch host input R. |
| [007](raw/2172002/007-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Establish borrowed hash relation R. |
| [008](raw/2172002/008-resume_physical.md) | `resume_physical` | Dispatch host input S. |
| [009](raw/2172002/009-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Establish borrowed sorted relation S. |
| [010](raw/2172002/010-resume_physical.md) | `resume_physical` | Dispatch Rename r0. |
| [011](raw/2172002/011-cq-example-providers::rename.md) | `cq-example-providers::rename` | Rename delegates to Φ_R(r0). |
| [012](raw/2172002/012-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | Establish slice alias r0. |
| [013](raw/2172002/013-resume_physical.md) | `resume_physical` | Dispatch Rename r1. |
| [014](raw/2172002/014-cq-example-providers::rename.md) | `cq-example-providers::rename` | Rename delegates to Φ_R(r1). |
| [015](raw/2172002/015-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Establish hash alias r1. |
| [016](raw/2172002/016-resume_physical.md) | `resume_physical` | Dispatch required index i0 before r2. |
| [017](raw/2172002/017-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Reuse r1’s existing physical index for IX demand i0. |
| [018](raw/2172002/018-resume_physical.md) | `resume_physical` | Dispatch selected join rule Λ(r2). |
| [019](raw/2172002/019-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | Request Rows for r0. |
| [020](raw/2172002/020-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | Return slice iterator source, item pattern and logical witness. |
| [021](raw/2172002/021-cq-example-providers::indexed_join_rows.md) | `cq-example-providers::indexed_join_rows` | Assemble the slice bind clause; request i0 Matches using the written IX use. |
| [022](raw/2172002/022-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Return exact hash lookup source, item pattern and decoded witness. |
| [023](raw/2172002/023-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | Assemble the dependent hash bind clause; request r2 carrier from Φ_R(r2). |
| [024](raw/2172002/024-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | Return borrowed pair carrier. |
| [025](raw/2172002/025-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | Return r2 as Access Region. |
| [026](raw/2172002/026-resume_physical.md) | `resume_physical` | Dispatch Rename r3 (in the written RA definition order). |
| [027](raw/2172002/027-cq-example-providers::rename.md) | `cq-example-providers::rename` | Rename delegates to Φ_R(r3). |
| [028](raw/2172002/028-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Establish sorted alias r3. |
| [029](raw/2172002/029-resume_physical.md) | `resume_physical` | Dispatch required index i1 before r4. |
| [030](raw/2172002/030-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Reuse r3’s existing physical index for IX demand i1. |
| [031](raw/2172002/031-resume_physical.md) | `resume_physical` | Dispatch selected join rule Λ(r4). |
| [032](raw/2172002/032-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | Inline prior region and request i1 Matches using b. |
| [033](raw/2172002/033-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Return exact sorted lookup source, item pattern and decoded witness. |
| [034](raw/2172002/034-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | Assemble the dependent sorted bind clause; request r4 carrier from Φ_R(r4). |
| [035](raw/2172002/035-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | Return borrowed triple carrier. |
| [036](raw/2172002/036-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | Return r4 as Access Region. |
| [037](raw/2172002/037-resume_physical.md) | `resume_physical` | Dispatch selected terminal Project Λ(r5). |
| [038](raw/2172002/038-cq-example-providers::project.md) | `cq-example-providers::project` | Keep source clauses; request Φ_R(r5) output fields a,c. |
| [039](raw/2172002/039-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | Construct the selected borrowed pair carrier. |
| [040](raw/2172002/040-cq-example-providers::project_output.md) | `cq-example-providers::project_output` | Return projected r5 as Access Region. |
| [041](raw/2172002/041-resume_physical.md) | `resume_physical` | Finish Physical → Rust[Access] with typed borrowed aliases. |
| [042](raw/2172002/042-access_to_set_loops.md) | `access_to_set_loops` | Caller-selected set consumer emits ordinary Rust loops. |

Calls 001, 002, 003 and 041 match the independently authored RA, IX, Physical
and Rust[Access] fixtures. Answers also match an independent relational oracle
over 256 input subsets; non-Clone output fields retain input addresses.
Projection deduplicates only through the explicit final set consumer.

The [direct-RA query](../realization-trace/review.md) has its own report. Its
numbering and result schema are different; the two invocation sequences are
not combined.

Rows/Matches providers return `source`, `item` and `witness`; the selected
RA rule constructs the `bind` clauses. Output providers return only witnesses.
The [separate pull query](../pull-trace/review.md) shows the neutral Access
bindings lowering to named Pipeline definitions before query-level `flat_map`
appears in Rust. Call numbers belong to each report independently.

Reproduce using a new output directory:

```sh
python tools/trace_macros.py --test physical_query --package mini-linq \
  --macro-package cq-example-providers \
  --features written-order,index-demands,physical-selection,physical-realization,access-to-set-loops \
  --out /tmp/mini-linq-query-trace --declarative -- \
  cq_to_projected_access_preserves_set_results_and_borrows_non_clone_data
CQ_REALIZATION_TRACE=/tmp/mini-linq-query-trace cargo test -p mini-linq \
  --no-default-features \
  --features written-order,index-demands,physical-selection,physical-realization,access-to-set-loops \
  --test physical_query captured_boundaries_equal_authored_ix_physical_and_access -- --ignored
```
