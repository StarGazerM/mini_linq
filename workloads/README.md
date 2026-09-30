```sh
cargo test -p cq-workload-e2e --locked              # compiler vs tiny DuckDB goldens; generated matrix + compile-fail cases
cargo test -p cq-workload-e2e --test census --locked -- --nocapture
cargo test --manifest-path examples/own-dialect/Cargo.toml --test corpus --locked  # pull/push vs the same goldens
cargo xtask list
cargo xtask generate all tiny --scenario all
cargo xtask golden all tiny --scenario all
cargo xtask golden                              # byte comparison of every checked-in golden, all scenarios
cargo xtask fuzz --cases 200 --seed 0            # gated deterministic compiled-query comparison with DuckDB
cargo xtask fuzz --cases 5000 --seed 0           # larger local run; generated Rust stays under target/
cargo xtask differential --cases 200 --seed 0    # alias for fuzz
cargo xtask golden all tiny --scenario all --write
cargo xtask explain triangle --through index-requirements
cargo test -p cq-storage --features counters --test counters --locked
cargo test -p cq-workloads --features counters --test growth --locked -- --nocapture
cargo xtask bench
cargo xtask bench --scale 512 --seed 7
cargo xtask generate extended tiny --scenario all
cargo xtask golden extended tiny --scenario all --write
cargo xtask golden extended tiny --scenario all
cargo test -p cq-workload-e2e --test extended --locked
cargo test -p cq-workload-e2e --test extension_laws --locked
cargo test -p cq-workload-e2e --test extension_compile_fail --locked
cargo test -p cq-workloads --test extended_catalog --locked
```
