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
| [2172042/001](raw/2172042/001-index_demands.md) | `index_demands` | `struct` | 515 |
| [2172042/002](raw/2172042/002-given_physical_bindings.md) | `given_physical_bindings` | `struct` | 1466 |
| [2172042/003](raw/2172042/003-realize_physical.md) | `realize_physical` | `struct` | 3595 |
| [2172042/004](raw/2172042/004-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | `reply_to(::` | 2078 |
| [2172042/005](raw/2172042/005-resume_physical.md) | `resume_physical` | `resume` | 3857 |
| [2172042/006](raw/2172042/006-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(::` | 2330 |
| [2172042/007](raw/2172042/007-resume_physical.md) | `resume_physical` | `resume` | 4180 |
| [2172042/008](raw/2172042/008-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(::` | 2568 |
| [2172042/009](raw/2172042/009-resume_physical.md) | `resume_physical` | `resume` | 4449 |
| [2172042/010](raw/2172042/010-cq-example-providers::rename.md) | `cq-example-providers::rename` | `reply_to(::` | 4472 |
| [2172042/011](raw/2172042/011-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | `reply_to(::` | 2681 |
| [2172042/012](raw/2172042/012-resume_physical.md) | `resume_physical` | `resume` | 4676 |
| [2172042/013](raw/2172042/013-cq-example-providers::rename.md) | `cq-example-providers::rename` | `reply_to(::` | 4702 |
| [2172042/014](raw/2172042/014-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(::` | 2918 |
| [2172042/015](raw/2172042/015-resume_physical.md) | `resume_physical` | `resume` | 4996 |
| [2172042/016](raw/2172042/016-cq-example-providers::rename.md) | `cq-example-providers::rename` | `reply_to(::` | 5024 |
| [2172042/017](raw/2172042/017-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(::` | 3147 |
| [2172042/018](raw/2172042/018-resume_physical.md) | `resume_physical` | `resume` | 5322 |
| [2172042/019](raw/2172042/019-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(::` | 3378 |
| [2172042/020](raw/2172042/020-resume_physical.md) | `resume_physical` | `resume` | 5652 |
| [2172042/021](raw/2172042/021-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | `reply_to(::` | 8464 |
| [2172042/022](raw/2172042/022-cq-example-providers::slice_rows.md) | `cq-example-providers::slice_rows` | `reply_to(::` | 6253 |
| [2172042/023](raw/2172042/023-cq-example-providers::indexed_join_rows.md) | `cq-example-providers::indexed_join_rows` | `resume` | 9091 |
| [2172042/024](raw/2172042/024-cq-example-providers::hash_relation.md) | `cq-example-providers::hash_relation` | `reply_to(::` | 6898 |
| [2172042/025](raw/2172042/025-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | `resume` | 9266 |
| [2172042/026](raw/2172042/026-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | `reply_to(::` | 6933 |
| [2172042/027](raw/2172042/027-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | `resume` | 3598 |
| [2172042/028](raw/2172042/028-resume_physical.md) | `resume_physical` | `resume` | 6083 |
| [2172042/029](raw/2172042/029-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(::` | 3847 |
| [2172042/030](raw/2172042/030-resume_physical.md) | `resume_physical` | `resume` | 6404 |
| [2172042/031](raw/2172042/031-cq-example-providers::indexed_join.md) | `cq-example-providers::indexed_join` | `reply_to(::` | 10368 |
| [2172042/032](raw/2172042/032-cq-example-providers::sorted_relation.md) | `cq-example-providers::sorted_relation` | `reply_to(::` | 7878 |
| [2172042/033](raw/2172042/033-cq-example-providers::indexed_join_matches.md) | `cq-example-providers::indexed_join_matches` | `resume` | 10537 |
| [2172042/034](raw/2172042/034-cq-example-providers::borrowed_fields.md) | `cq-example-providers::borrowed_fields` | `reply_to(::` | 7905 |
| [2172042/035](raw/2172042/035-cq-example-providers::indexed_join_output.md) | `cq-example-providers::indexed_join_output` | `resume` | 4142 |
| [2172042/036](raw/2172042/036-resume_physical.md) | `resume_physical` | `resume` | 1063 |
| [2172042/037](raw/2172042/037-access_to_set_loops.md) | `access_to_set_loops` | `answer(a,` | 552 |
