# Physical composition regression

This example currently fails to compile at `mini_linq::choose_physical`.
The query-specific `explicit_policy!` table has been removed: naming this
query's `l`, `r`, `s`, `j0`, `j1`, `i0`, and `i1` did not demonstrate general
physical selection.

```sh
cargo run --manifest-path examples/composition/Cargo.toml --locked
```

The source retains its RA query, borrowed slice/HashMap/BTreeMap inputs, and
result assertion. Completing the compiler must make the shared selection path
handle this input without requiring a handwritten table for its internal plan.
The input/provider configuration API is still unresolved; this regression does
not prescribe a replacement API.

The selected Access endpoint and explicit set-state provider remain in the
caller wiring. The regression runner includes this example and reports its
compilation failure.
