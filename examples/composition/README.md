# Build a selected compiler

This standalone example selects graph planning, named-result indexes, a
materialized callable, pull lowering and a set collector. Its Cargo dependency
disables the macro frontend's defaults. The canonical/prefix planners, input
indexes, other physical realizations, native Access providers and default
composition are absent from its resolved compiler dependencies.

Run from the repository root:

```sh
cargo run --manifest-path examples/composition/Cargo.toml --locked
```

The import aliases in `src/main.rs` connect the stages. Cargo features control
which implementation modules are included; each implementation receives/emits
its existing IR. Changing a stage still requires matching its semantic domain.
