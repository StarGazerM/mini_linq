# An independent RA surface, reusing downstream passes

`ra-theta` defines `theta_join … with … if …; output …;` and its own schema
and heading analysis. It desugars the join to IX's semantic `Join` + `Select`
operations, derives its own IX demands, and implements `cq_lang_ix::Env`.
It does not depend on the reference RA grammar or `cq-index-policy`, whose
source is the reference RA section.

A true `union` operator cannot be sent to the unchanged Physical language:
Physical has no union operation. An independent surface using the existing semantic operations
can reuse every downstream pass without copying or modifying those crates.

`build.rs` calls `cq-physical-lowering::compile`, creates its target Env,
then calls `cq-structure-selection::compile` and creates that target Env.
Finally `cq-physical-realization::compile` prints Rust, which the build script
writes into `OUT_DIR/query.rs`. The library includes this file, so rustc
compiles the generated program and the tests execute it. The only local macro
implements the requested distinct-state protocol; no compiler proc-macro
facade or reference prefix is needed.

Both probe and merge variants are tested against an independent nested-loop
oracle, including duplicates, empty inputs, and every pair of subsets of a
four-row domain (256 pairs per route).

```sh
python3 examples/diverged-ra/check.py
cargo run --manifest-path examples/diverged-ra/Cargo.toml --locked
```

The regression checker also checks `Cargo.lock`, resolved feature sets, and
resolved manifest paths. Every internal dependency is one of:

| Crate | Source |
|---|---|
| `ra-theta` | Local independent surface, only depends on IX and `syn` |
| `cq-lang-ix` | Repository language, defaults disabled |
| `cq-lang-physical` | Repository language, defaults disabled |
| `cq-lang-structures` | Repository language, defaults disabled |
| `cq-physical-lowering` | Repository pass, defaults disabled |
| `cq-structure-selection` | Repository pass, defaults disabled |
| `cq-physical-realization` | Repository pass, defaults disabled |
| `cq-print` | Transitive repository printer support, defaults disabled |
| `cq-syntax` | Transitive repository shared syntax |
| `cq-storage` | Repository execution runtime |

External dependencies are `syn`, `quote`, `proc-macro2`, `unicode-ident` and
the repository-pinned `syn_derive`. The lockfile contains no `cq-lang-ra`,
`cq-lang-cq`, `cq-lang-vo`, upstream pass, or compiler facade.
