# Compose physical providers per relation

This runnable example starts with an explicit RA plan, computes IX, records
Physical assignments, and realizes an Access region through external provider
macros. It demonstrates one query over a borrowed slice, `HashMap`, and
`BTreeMap`. The caller writes the Rust function signature and chooses the
Access consumer; no generated query object or runtime relation wrapper is used.

From the repository root:

```sh
cargo run --manifest-path examples/composition/Cargo.toml --locked --offline
```

Expected output:

```text
Borrowed heterogeneous join: [(1, 10, 100), (1, 10, 101), (2, 20, 200)]
```

The compilation boundaries in `src/main.rs` are:

1. `index_demands!`: RA → IX. It records `i0 = @r[0]` and `i1 = @s[0]`,
   with `j0.right = i0` and `j1.right = i1`.
2. `choose_physical!`, an import alias for the caller's `explicit_policy!`:
   IX → Physical through `given_physical_bindings!`. The policy records every
   relation/index provider and operation lowering. `r/i0` use the hash provider;
   `s/i1` use the sorted provider. These are independent assignments.
3. `realize_physical!`: Physical → Rust[Access], including the selected external
   provider/rule callbacks. It follows the recorded assignments and reuses the
   two existing indexes. Field values remain borrowed.
4. `access!`, explicitly aliased to `access_to_set_loops!`, consumes the completed
   Access region and returns a set of borrowed tuples.

The example does not invoke a CQ planning macro: its RA plan is written directly
so the supported physical region is explicit. Its admitted operators are direct
input Rename and exact indexed NaturalJoin. Zone-map semijoins, intermediate
materialization, and generated host signatures are not implemented by these
providers yet. Cargo features select only the macros used here.
