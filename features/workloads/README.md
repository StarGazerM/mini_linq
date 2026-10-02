Run acceptance from this standalone workspace. Corpus and benchmark contracts
are documented in the crates; fresh evidence lives in the task ledger.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test -p cq-workloads --features counters --test growth -- --nocapture
cargo test -p cq-workload-e2e --features parquet
cargo xtask golden
cargo xtask fuzz --cases 200 --seed 0
cargo xtask fuzz --cases 2000 --seed 0
cargo xtask bench
cargo xtask bench --tpch --sf 0.1
cargo xtask bench --tpch --sf 1
```

From the repository root: `python3 tools/check_external.py`.
