# Actual proc-macro boundaries

Historical capture from before operator-rule extraction. Its `cq_example_providers`
operator paths describe that revision. Current operator entry points are in
`mini_linq`; current storage adapters remain in `cq_example_providers`.


These calls were captured during rustc expansion of the unchanged target.
Raw token files are authoritative. Formatted Rust is a reading aid only.
Sequence numbers are invocation order within each compiler process, not runtime order.
Provider calls selected by --macro-package share that process sequence.
See command.json, compile.log, source-hashes.json, instrumentation.patch
and instrumented-packages.json.

[Print all procedural steps](steps.md)

[Declarative macro input/output trace](declarative.log) was captured in a
second compilation of the same source with target-local `-Ztrace-macros`.
The two logs are separate observations, not one merged event timeline.

| Process / step | Macro | Input begins | Output bytes |
| --- | --- | --- | ---: |
| [2172002/001](raw/2172002/001-cq_to_ra_written_order.md) | `cq_to_ra_written_order` | `struct` | 476 |
| [2172002/002](raw/2172002/002-index_demands.md) | `index_demands` | `struct` | 557 |
| [2172002/003](raw/2172002/003-given_physical_bindings.md) | `given_physical_bindings` | `struct` | 1628 |
| [2172002/004](raw/2172002/004-realize_physical.md) | `realize_physical` | `struct` | 3955 |
| [2172002/005](raw/2172002/005-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | `reply_to(::` | 2264 |
| [2172002/006](raw/2172002/006-resume_physical.md) | `resume_physical` | `resume` | 4217 |
| [2172002/007](raw/2172002/007-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(::` | 2516 |
| [2172002/008](raw/2172002/008-resume_physical.md) | `resume_physical` | `resume` | 4540 |
| [2172002/009](raw/2172002/009-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(::` | 2754 |
| [2172002/010](raw/2172002/010-resume_physical.md) | `resume_physical` | `resume` | 4811 |
| [2172002/011](raw/2172002/011-cq-example-providers::rename.md) | `cq-example-providers::rename` | `reply_to(::` | 4834 |
| [2172002/012](raw/2172002/012-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | `reply_to(::` | 2871 |
| [2172002/013](raw/2172002/013-resume_physical.md) | `resume_physical` | `resume` | 5043 |
| [2172002/014](raw/2172002/014-cq-example-providers::rename.md) | `cq-example-providers::rename` | `reply_to(::` | 5069 |
| [2172002/015](raw/2172002/015-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(::` | 3111 |
| [2172002/016](raw/2172002/016-resume_physical.md) | `resume_physical` | `resume` | 5387 |
| [2172002/017](raw/2172002/017-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(::` | 3354 |
| [2172002/018](raw/2172002/018-resume_physical.md) | `resume_physical` | `resume` | 5694 |
| [2172002/019](raw/2172002/019-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | `reply_to(::` | 8585 |
| [2172002/020](raw/2172002/020-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | `reply_to(::` | 6284 |
| [2172002/021](raw/2172002/021-cq-example-providers::indexed_join_rows.md) | `cq-example-providers::indexed_join_rows` | `resume` | 9209 |
| [2172002/022](raw/2172002/022-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(::` | 6926 |
| [2172002/023](raw/2172002/023-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | `resume` | 9387 |
| [2172002/024](raw/2172002/024-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | `reply_to(::` | 6962 |
| [2172002/025](raw/2172002/025-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | `resume` | 3553 |
| [2172002/026](raw/2172002/026-resume_physical.md) | `resume_physical` | `resume` | 6122 |
| [2172002/027](raw/2172002/027-cq-example-providers::rename.md) | `cq-example-providers::rename` | `reply_to(::` | 6150 |
| [2172002/028](raw/2172002/028-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(::` | 3801 |
| [2172002/029](raw/2172002/029-resume_physical.md) | `resume_physical` | `resume` | 6453 |
| [2172002/030](raw/2172002/030-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(::` | 4035 |
| [2172002/031](raw/2172002/031-resume_physical.md) | `resume_physical` | `resume` | 6775 |
| [2172002/032](raw/2172002/032-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | `reply_to(::` | 10977 |
| [2172002/033](raw/2172002/033-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(::` | 8304 |
| [2172002/034](raw/2172002/034-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | `resume` | 11148 |
| [2172002/035](raw/2172002/035-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | `reply_to(::` | 8332 |
| [2172002/036](raw/2172002/036-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | `resume` | 4334 |
| [2172002/037](raw/2172002/037-resume_physical.md) | `resume_physical` | `resume` | 7384 |
| [2172002/038](raw/2172002/038-cq-example-providers::project.md) | `cq-example-providers::project` | `reply_to(::` | 12118 |
| [2172002/039](raw/2172002/039-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | `reply_to(::` | 9037 |
| [2172002/040](raw/2172002/040-cq-example-providers::project_output.md) | `cq-example-providers::project_output` | `resume` | 4647 |
| [2172002/041](raw/2172002/041-resume_physical.md) | `resume_physical` | `resume` | 1066 |
| [2172002/042](raw/2172002/042-access_to_set_loops.md) | `access_to_set_loops` | `answer(a,` | 485 |
