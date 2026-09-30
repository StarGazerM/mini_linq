```sh
cargo test -p cq-workload-e2e --locked              # compiler vs tiny DuckDB goldens; generated matrix + compile-fail cases
cargo test -p cq-workload-e2e --test census --locked -- --nocapture
cargo test --manifest-path examples/own-dialect/Cargo.toml --test corpus --locked  # pull/push vs the same goldens
cargo xtask list
cargo xtask generate all tiny --scenario all
cargo xtask golden all tiny --scenario all
cargo xtask golden all tiny --scenario all --write
cargo xtask explain triangle --through index-requirements
```
