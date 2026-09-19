# MiniLinq

MiniLinq is a Rust query compiler with replaceable macro boundaries:

```text
CQ → RA → IX → Physical → Rust[Access]
```

IX names the indexes required by the fixed RA plan and records their uses.
Physical records a provider for each relation version and index, and a lowering
rule for each RA definition. Realization invokes those recorded external macros.
The common compiler contains no hash/sorted/block dispatch or fallback backend.

Read the [design and current domain](doc/IX-PHYSICAL-REDESIGN.md), the
[short paper](doc/formal/build/physical_design.pdf), or the
[executable composition](examples/composition/README.md).

The implemented example combines a borrowed slice, `HashMap`, and `BTreeMap` in
one query. Its output borrows non-`Clone` values. Tests cover independent provider
rebinding, renamed index columns, external result providers, exact answers,
incompatible assignments, and actual macro expansion. A separate full-CQ test includes terminal projection. The caller writes the Rust
host function and chooses the Access consumer. Existing Access/Pipeline consumers
remain independently usable; their placement in the general physical design is
still open.

The old whole-query hash/configured/Native routes have been removed. Stored
intermediates, block/YA realization, general schemas/index construction, and the
old generated `build/query/run` API have **not** been ported. Logical YA planning
and its independent relational oracle tests remain. See the explicit
[migration coverage](doc/IX-PHYSICAL-REDESIGN.md#implementation-domain).

```sh
cargo run --manifest-path examples/composition/Cargo.toml --offline
cargo run --example cq-inspection
cargo run --example pull-execution
cargo run --example free-variables
cargo test --workspace --offline
python3 tools/check_dependencies.py
make -C doc/formal pdf
```

The [workload catalog](workloads/README.md) keeps authored queries, deterministic
datasets, and SQL oracles independently of executable provider composition.
`cargo xtask explain` inspects the logical/IX prefix; it does not invent missing
Physical assignments. Ordinary workspace checks do not build DuckDB.

The [documentation map](doc/README.md) separates the current compiler design
from teaching tasks and research proposals.
