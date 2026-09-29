# Heterogeneous composition

A host composes the compiler stages by binding macro slots:

1. `index_demands!` (RA → IX) continues to the caller's `physical_policy!`.
2. `physical_policy!` appends a query-independent `policy { … }` stating
   facts about each host value — here `R` and `S` are maps keyed on their
   join columns — and invokes `lower_physical!` (IX → Physical): one
   physical operator per result, modes from use.
3. `select_structures!` (Physical → Γ) produces the bindings Γ in the
   `structures` section, binding every keyed read to a
   host key fact or a Rust type built over the result; here every read is
   answered by a host key, so nothing is built.
4. `structures_to_rust!` prints one pull program over the `cq_storage`
   traits: a block evaluating to an iterator of borrowed answers, deduplicated
   through the caller's `distinct_state!` provider.

No query-specific names appear in the policy; nothing is built at runtime
because the host's own maps already answer the required probes.

```sh
cargo run --manifest-path examples/composition/Cargo.toml
```
