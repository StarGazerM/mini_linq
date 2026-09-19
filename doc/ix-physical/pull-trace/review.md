# PullChain: full CQ pull expansion

This report contains only `physical_pull.rs::run`:

```text
answer(a,c) :- L(a), R(a,b), S(b,c);
```

The host supplies a borrowed slice, hash map and B-tree map. It selects
`access_to_distinct_iter as access` and `pipeline_to_distinct_iter`; provider
assignments remain per relation/index. Caller-written observation macros add
counters to each provider's source expression. Their actual expansions are in
[declarative.log](declarative.log), recorded separately from these 43 procedural
invocations. A provider reply may pass through that declarative callback before
becoming the next procedural input.

The stages are **CQ → RA → IX → Physical → Rust[Access] → Pipeline → Rust**.
Calls 004–041 are realization's administrative exchanges. Rows/Matches return
an iterator expression, item pattern and witness. The selected RA rule builds
neutral `bind` clauses; Output replies cannot add sources or clauses.

[All actual input/output pairs](steps.md) · [Completed Access](completed.rs)
· [Pipeline output](pipeline.rs) · [Final Rust](rust.rs)
· [Source hashes](source-hashes.json) · [Capture verification](verification.txt)

| Call | Actual implementation | Effect |
| --- | --- | --- |
| [001](raw/2172007/001-cq_to_ra_written_order.md) | `cq_to_ra_written_order` | CQ → RA: Rename, natural joins, then final Project. |
| [002](raw/2172007/002-index_demands.md) | `index_demands` | RA → IX: i0=@r1[0], i1=@r3[0], with r2.right and r4.right uses. |
| [003](raw/2172007/003-given_physical_bindings.md) | `given_physical_bindings` | Explicit assignments → frozen Physical. |
| [004](raw/2172007/004-realize_physical.md) | `realize_physical` | Dispatch first host input L. |
| [005](raw/2172007/005-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | Establish borrowed unary slice L. |
| [006](raw/2172007/006-resume_physical.md) | `resume_physical` | Dispatch host input R. |
| [007](raw/2172007/007-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Establish borrowed hash relation R. |
| [008](raw/2172007/008-resume_physical.md) | `resume_physical` | Dispatch host input S. |
| [009](raw/2172007/009-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Establish borrowed sorted relation S. |
| [010](raw/2172007/010-resume_physical.md) | `resume_physical` | Dispatch Rename r0. |
| [011](raw/2172007/011-cq-example-providers::rename.md) | `cq-example-providers::rename` | Rename delegates to Φ_R(r0). |
| [012](raw/2172007/012-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | Establish slice alias r0. |
| [013](raw/2172007/013-resume_physical.md) | `resume_physical` | Dispatch Rename r1. |
| [014](raw/2172007/014-cq-example-providers::rename.md) | `cq-example-providers::rename` | Rename delegates to Φ_R(r1). |
| [015](raw/2172007/015-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Establish hash alias r1. |
| [016](raw/2172007/016-resume_physical.md) | `resume_physical` | Dispatch required index i0 before r2. |
| [017](raw/2172007/017-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Reuse r1’s existing physical index for IX demand i0. |
| [018](raw/2172007/018-resume_physical.md) | `resume_physical` | Dispatch selected join rule Λ(r2). |
| [019](raw/2172007/019-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | Request Rows for r0. |
| [020](raw/2172007/020-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | Return slice iterator source, item pattern and logical witness. |
| [021](raw/2172007/021-cq-example-providers::indexed_join_rows.md) | `cq-example-providers::indexed_join_rows` | Assemble the slice bind clause; request i0 Matches using the written IX use. |
| [022](raw/2172007/022-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | Return exact hash lookup source, item pattern and decoded witness. |
| [023](raw/2172007/023-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | Assemble the dependent hash bind clause; request r2 carrier from Φ_R(r2). |
| [024](raw/2172007/024-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | Return borrowed pair carrier. |
| [025](raw/2172007/025-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | Return r2 as Access Region. |
| [026](raw/2172007/026-resume_physical.md) | `resume_physical` | Dispatch Rename r3 (in the written RA definition order). |
| [027](raw/2172007/027-cq-example-providers::rename.md) | `cq-example-providers::rename` | Rename delegates to Φ_R(r3). |
| [028](raw/2172007/028-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Establish sorted alias r3. |
| [029](raw/2172007/029-resume_physical.md) | `resume_physical` | Dispatch required index i1 before r4. |
| [030](raw/2172007/030-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Reuse r3’s existing physical index for IX demand i1. |
| [031](raw/2172007/031-resume_physical.md) | `resume_physical` | Dispatch selected join rule Λ(r4). |
| [032](raw/2172007/032-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | Inline prior region and request i1 Matches using b. |
| [033](raw/2172007/033-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | Return exact sorted lookup source, item pattern and decoded witness. |
| [034](raw/2172007/034-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | Assemble the dependent sorted bind clause; request r4 carrier from Φ_R(r4). |
| [035](raw/2172007/035-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | Return borrowed triple carrier. |
| [036](raw/2172007/036-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | Return r4 as Access Region. |
| [037](raw/2172007/037-resume_physical.md) | `resume_physical` | Dispatch selected terminal Project Λ(r5). |
| [038](raw/2172007/038-cq-example-providers::project.md) | `cq-example-providers::project` | Keep source clauses; request Φ_R(r5) output fields a,c. |
| [039](raw/2172007/039-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | Construct the selected borrowed pair carrier. |
| [040](raw/2172007/040-cq-example-providers::project_output.md) | `cq-example-providers::project_output` | Return projected r5 as Access Region. |
| [041](raw/2172007/041-resume_physical.md) | `resume_physical` | Finish Physical → Rust[Access] with typed borrowed aliases. |
| [042](raw/2172007/042-access_to_distinct_iter.md) | `access_to_distinct_iter` | Access → named Pipeline: Scan, two dependent Joins, Project and Distinct; no query adapter nesting. |
| [043](raw/2172007/043-pipeline_to_distinct_iter.md) | `pipeline_to_distinct_iter` | Pipeline → Rust: lazy once_with boundary, one adapter local per named stream, dependent flat_map and deferred distinct state. |

Call 042's actual output is:

```rust
mini_linq :: pipeline_to_distinct_iter!
{
    iter0 = scan a in (observe(0, (__cq_store_r0).iter())) yield (a,); iter1 =
    join iter0 as (a,) with b in
    (observe(1, (__cq_index_i0).get(a).into_iter().flatten())) yield (a, b,);
    iter2 = join iter1 as (a, b,) with c in
    (observe(2, (__cq_index_i1).get(b).into_iter().flatten())) yield
    (a, b, c,); iter3 = project iter2 as (a, b, c,) yield ((a, c,)); iter4 =
    distinct iter3; return iter4;
}
```

Each definition refers to the previous named stream. The expressions for the
hash and sorted sources depend on earlier bindings; they have not been executed
or nested into a whole-query Rust adapter. Call 043 introduces the two query
`flat_map` adapters and wraps stream construction in `once_with`. The final
Rust `Clone::clone(&a)` copies a reference: `Value` implements neither `Clone`
nor `Copy`, and output pointer assertions check original-input provenance.

The captured test checks zero source evaluations and zero returned rows after
construction and unpolled drop. Its first successful `any` evaluates each of the
three sources once and takes one row from each. Resuming with `next` takes only
a second row from the innermost sorted bucket. Complete execution separately
matches independent relational enumeration on 256 input subsets and produces
distinct borrowed answers. These observations are specific to this selected
pull route; equal complete sets alone would not prove deferred execution.

Authored RA, IX, Physical, Access and Pipeline fixtures match these actual
boundaries. The [eager CQ query](../query-trace/review.md) and
[direct-RA query](../realization-trace/review.md) have separate reports and
invocation numbers.

Reproduce in a new directory:

```sh
python3 tools/trace_macros.py --test physical_pull --package mini-linq \
  --macro-package cq-example-providers \
  --features written-order,index-demands,physical-selection,physical-realization,access-to-pipeline,pipeline-distinct-iter \
  --out /tmp/mini-linq-pull-trace --declarative -- \
  full_cq_pull_defers_every_source_and_short_circuits_each_provider
CQ_REALIZATION_TRACE=/tmp/mini-linq-pull-trace cargo test -p mini-linq \
  --no-default-features \
  --features written-order,index-demands,physical-selection,physical-realization,access-to-pipeline,pipeline-distinct-iter \
  --test physical_pull captured_boundaries_equal_authored_ix_physical_access_and_pipeline -- --ignored
```
