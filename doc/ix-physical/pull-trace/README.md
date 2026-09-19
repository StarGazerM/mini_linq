# Actual proc-macro boundaries

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
| [2172007/001](raw/2172007/001-cq_to_ra_written_order.md) | `cq_to_ra_written_order` | `struct` | 471 |
| [2172007/002](raw/2172007/002-index_demands.md) | `index_demands` | `struct` | 552 |
| [2172007/003](raw/2172007/003-given_physical_bindings.md) | `given_physical_bindings` | `struct` | 1339 |
| [2172007/004](raw/2172007/004-realize_physical.md) | `realize_physical` | `struct` | 3270 |
| [2172007/005](raw/2172007/005-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | `reply_to(observe_reply)` | 2102 |
| [2172007/006](raw/2172007/006-resume_physical.md) | `resume_physical` | `resume` | 3430 |
| [2172007/007](raw/2172007/007-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(observe_reply)` | 2299 |
| [2172007/008](raw/2172007/008-resume_physical.md) | `resume_physical` | `resume` | 3731 |
| [2172007/009](raw/2172007/009-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(observe_reply)` | 2558 |
| [2172007/010](raw/2172007/010-resume_physical.md) | `resume_physical` | `resume` | 3975 |
| [2172007/011](raw/2172007/011-cq-example-providers::rename.md) | `cq-example-providers::rename` | `reply_to(::` | 3975 |
| [2172007/012](raw/2172007/012-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | `reply_to(observe_reply)` | 2664 |
| [2172007/013](raw/2172007/013-resume_physical.md) | `resume_physical` | `resume` | 4161 |
| [2172007/014](raw/2172007/014-cq-example-providers::rename.md) | `cq-example-providers::rename` | `reply_to(::` | 4160 |
| [2172007/015](raw/2172007/015-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(observe_reply)` | 2881 |
| [2172007/016](raw/2172007/016-resume_physical.md) | `resume_physical` | `resume` | 4396 |
| [2172007/017](raw/2172007/017-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(observe_reply)` | 3097 |
| [2172007/018](raw/2172007/018-resume_physical.md) | `resume_physical` | `resume` | 4676 |
| [2172007/019](raw/2172007/019-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | `reply_to(::` | 6989 |
| [2172007/020](raw/2172007/020-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | `reply_to(observe_reply)` | 5668 |
| [2172007/021](raw/2172007/021-cq-example-providers::indexed_join_rows.md) | `cq-example-providers::indexed_join_rows` | `resume` | 7537 |
| [2172007/022](raw/2172007/022-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(observe_reply)` | 6274 |
| [2172007/023](raw/2172007/023-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | `resume` | 7766 |
| [2172007/024](raw/2172007/024-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | `reply_to(::` | 5852 |
| [2172007/025](raw/2172007/025-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | `resume` | 3070 |
| [2172007/026](raw/2172007/026-resume_physical.md) | `resume_physical` | `resume` | 5152 |
| [2172007/027](raw/2172007/027-cq-example-providers::rename.md) | `cq-example-providers::rename` | `reply_to(::` | 5153 |
| [2172007/028](raw/2172007/028-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(observe_reply)` | 3585 |
| [2172007/029](raw/2172007/029-resume_physical.md) | `resume_physical` | `resume` | 5382 |
| [2172007/030](raw/2172007/030-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(observe_reply)` | 3788 |
| [2172007/031](raw/2172007/031-resume_physical.md) | `resume_physical` | `resume` | 5677 |
| [2172007/032](raw/2172007/032-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | `reply_to(::` | 9159 |
| [2172007/033](raw/2172007/033-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(observe_reply)` | 7612 |
| [2172007/034](raw/2172007/034-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | `resume` | 9381 |
| [2172007/035](raw/2172007/035-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | `reply_to(::` | 7114 |
| [2172007/036](raw/2172007/036-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | `resume` | 3833 |
| [2172007/037](raw/2172007/037-resume_physical.md) | `resume_physical` | `resume` | 6386 |
| [2172007/038](raw/2172007/038-cq-example-providers::project.md) | `cq-example-providers::project` | `reply_to(::` | 10551 |
| [2172007/039](raw/2172007/039-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | `reply_to(::` | 7971 |
| [2172007/040](raw/2172007/040-cq-example-providers::project_output.md) | `cq-example-providers::project_output` | `resume` | 4198 |
| [2172007/041](raw/2172007/041-resume_physical.md) | `resume_physical` | `resume` | 1110 |
| [2172007/042](raw/2172007/042-access_to_distinct_iter.md) | `access_to_distinct_iter` | `answer(a,` | 457 |
| [2172007/043](raw/2172007/043-pipeline_to_distinct_iter.md) | `pipeline_to_distinct_iter` | `iter0` | 1377 |
