# YA release profile

This profile is an extraction test for compiler decomposition and a starting
point for the YA part of Project 2. It selects one route:

```text
CQ graph planning → RA → named-result indexes → materialized Access
                  → pull Pipeline → Rust set collection
```

The export has three crates: `cq-ir`, `cq-compiler`, and `mini-linq`. It does
not contain the input-indexed, lazy, direct, block, or native-provider
implementations, the storage crate, workloads, the optional recursion
assignment, or private reference implementations of the two student targets.
The shared CQ syntax is preserved; the selected graph planner checks its own
smaller admitted domain.

Shared analysis and syntax-observation files retain their complete definitions,
including facts used by other profiles. The release module entries mark these
unused projections locally; they do not include another scheduling or physical
lowering implementation.

The two replaceable algorithm files are the logical YA sweep scheduler and
the materialized physical semijoin lowering. Supplied administrative rules
check scheduling premises. Supplied source–target checks examine the emitted
algebra and Access regions independently of the student algorithms.

The exporter uses an explicit file allowlist and small module-entry templates.
It does not copy the whole compiler and conceal unused modules behind feature
flags. `EXPORT.json` in the result records the selected files and their hashes.
The exported workspace has its own lockfile and contains no path dependency
back to this checkout.

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
