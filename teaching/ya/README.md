# YA release profile

This profile is an extraction test for compiler decomposition and a starting
point for the YA part of Project 2. It selects one route:

```text
CQ graph planning → RA → named-result indexes → materialized Access
                  → pull Pipeline → Rust set collection
```

The export contains seven package families: `cq-ir`, `cq-query-lowering`, `cq-index-policy`,
`cq-relation-lowering`, `cq-access-lowering`, `cq-pipeline-lowering`, and
`mini-linq`. It selects `hypergraph`, `result_demands`, `materialized_hash`,
`to_pipeline` and `collect_set`. Written-order and clause-prefix query lowering,
base index demands, other relation lowerings, other Pipeline
endpoints, native access/providers, storage, workloads and the optional recursion
assignment are physically absent. Shared CQ syntax stays complete; graph planning
checks its own smaller admitted domain.

Only the selected implementations and their declared family helpers are copied.
Planning certificates remain under `cq-query-lowering`; Rust emission helpers and
independent checker observations remain under `cq-relation-lowering`. There are no analysis/helper
packages. Their existing feature guards and module declarations are reduced to
the selected build. This rewrites build wiring, never algorithm statements.

The two replaceable algorithm files are the logical YA sweep scheduler and
the materialized physical semijoin lowering, in `cq-query-lowering/src/hypergraph/ya.rs` and
`cq-relation-lowering/src/materialized_hash/semijoin.rs`, respectively. Supplied administrative rules
check scheduling premises. Supplied source–target checks examine the emitted
algebra and Access regions independently of the student algorithms.

The profile explicitly inventories selected source files. The exporter checks
that list against the same reviewed module ownership used by the source-removal
gate, and supplies a small frontend with explicit route aliases. Invocation and
Access parsing helpers come from the production frontend. Typed APIs keep their
production module paths; no implementation is replaced by a facade. The export
has its own manifests and lockfile, with no dependency on the parent checkout.
`EXPORT.json` records every delivered file and its hash.

From the staff checkout:

The exporter requires Cargo and Python 3.11 or later (Python 3.10 also works
with `tomli` installed). Its dependency resolution uses the staff machine's
Cargo cache; run a normal workspace build first if that cache is empty.

```sh
python3 tools/export_ya.py /tmp/ya-starter --verify
```

The destination must not exist. Verification first checks the supplied
foundation with both exercises missing. It then checks that each assignment
suite detects its corresponding missing algorithm, installs both reference
files temporarily, and runs the complete exported workspace tests and both
examples. It restores the student stubs before returning. Reference files are
read from this checkout and never included in the exported starter.

This release profile is deliberately narrower than the main research example
`blocked-ya`: it teaches YA over exact hash indexes. Block metadata, zone-map
guards, mixed storage, and alternative access providers remain independent
extensions in the parent compiler. The profile does not yet define the
triejoin or recursive-program releases.
