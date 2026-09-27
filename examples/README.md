# MiniLinq examples

| Example | Boundary exercised |
| --- | --- |
| [composition](composition/README.md) | Full RA → IX → Physical → Structures → Rust chain with every macro slot bound |
| [cq-inspection](cq-inspection/main.rs) | Parse and inspect CQ |
| [free-variables](free-variables/main.rs) | Analyze Rust expression dependencies |

```sh
cargo run --manifest-path examples/composition/Cargo.toml --offline
cargo run --example cq-inspection
cargo run --example free-variables
```

[Negation and aggregation](negation-aggregation/README.md) remain a teaching
specification, not a supported executable route.
