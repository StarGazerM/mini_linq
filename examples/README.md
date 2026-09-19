# MiniLinq examples

| Example | Boundary exercised |
| --- | --- |
| [composition](composition/README.md) | Full RA → IX → Physical → Rust[Access] realization with heterogeneous providers |
| [cq-inspection](cq-inspection/main.rs) | Parse and inspect CQ |
| [pull-execution](pull-execution/main.rs) | Execute an explicit Access quotation through Pipeline |
| [free-variables](free-variables/main.rs) | Analyze Rust expression dependencies |

```sh
cargo run --manifest-path examples/composition/Cargo.toml --offline
cargo run --example cq-inspection
cargo run --example pull-execution
cargo run --example free-variables
```

The old generated-query, block-YA, and Rust-expression backend examples used
retired post-IX routes. Their behavior is listed as unported in the
[implementation domain](../doc/IX-PHYSICAL-REDESIGN.md#implementation-domain).
[Negation and aggregation](negation-aggregation/README.md) remain a teaching
specification, not a supported executable route.
