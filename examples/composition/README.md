# Heterogeneous composition

A host composes the compiler stages by binding macro slots:

1. `index_demands!` (RA → IX) continues to the caller's `stream_policy!`.
2. `stream_policy!` appends a query-independent `policy { … }` declaring what
   each host relation can do — here `R` and `S` are maps keyed on their join
   columns — and invokes `ix_to_stream!` (IX → Stream).
3. `stream_to_access!` erases Stream to an Access region over `cq_storage`
   traits, then hands it to the caller's `access` binding.
4. `access_to_set_loops!` executes the region eagerly into the selected
   `set_sink!`.

No query-specific names appear in the policy; nothing is built at runtime
because the host's own maps already provide the required seeks.

```sh
cargo run --manifest-path examples/composition/Cargo.toml
```
