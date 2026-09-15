# MiniLinq examples

These examples are grouped by what they demonstrate rather than by the crate
that happens to provide an API.

| Folder | Demonstrates | Needs completed student passes? |
|---|---|---|
| [`composition`](composition/README.md) | One query with explicit stage aliases and only the selected compiler crates enabled | Supplied independent implementation |
| [`blocked-ya`](blocked-ya/main.rs) | Heterogeneous relation storage and exact semijoin after block metadata pruning | Supplied implementation |
| [`cq-inspection`](cq-inspection/main.rs) | Parsing one CQ into the typed source IR and inspecting its head and body | No |
| [`pull-execution`](pull-execution/main.rs) | The supplied `pull!` stage over an already-built physical store, including lazy and materialized execution | No |
| [`triangle-duckdb`](triangle-duckdb/main.rs) | The whole `mini_linq!` compiler path, visible input rows, execution, equivalent DuckDB SQL, and an exact result comparison | Yes |
| [`rust-expressions`](rust-expressions/README.md) | Literal/computed arguments, lets, Rust guards and heads, join dependencies, evaluation counts, and lazy cursors; every result is asserted | Yes |
| [`negation-aggregation`](negation-aggregation/README.md) | Optional specifications for safe negation, correlated sum, and global sum with expected results | Yes, plus the unimplemented HW4 extension |

Run the first two examples from the repository root:

```text
cargo run --example cq-inspection
cargo run --example pull-execution
```

After implementing the three student passes, run the end-to-end comparison:

```text
cargo run --example triangle-duckdb --features duckdb-example
```

The DuckDB feature is explicit so ordinary workspace builds do not compile the
bundled database library. The larger deterministic corpus remains under
[`workloads`](../workloads/README.md); use `cargo xtask differential ...` for
coverage rather than as the first readable example.
