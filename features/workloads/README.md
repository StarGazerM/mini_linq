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

```sh
cargo test -p cq-load --locked
cargo test -p cq-load --features parquet --locked
cargo test -p mini-linq --test relation_loaders --locked
cargo test -p cq-workload-e2e --test typed --locked
cargo test -p cq-workload-e2e --test typed --features parquet --locked
cargo xtask golden typed --write                 # DuckDB COPY creates typed Parquet fixtures and SQL goldens
cargo xtask golden typed                         # both formats execute against the same golden gate
mkdir -p target/load-bench
duckdb :memory: -c "CREATE TABLE lineitem AS SELECT (5000000000+i)::BIGINT AS orderkey, (i%7+1)::INTEGER AS linenumber, ((i%1000000)::DECIMAL(18,2)/100)::DECIMAL(18,2) AS price, DATE '1992-01-01'+(i%2500)::INTEGER AS shipdate, 'lineitem-'||(i%1000)::VARCHAR AS comment FROM range(1000000) AS rows(i); COPY lineitem TO 'target/load-bench/lineitem.csv' (FORMAT CSV, HEADER false); COPY lineitem TO 'target/load-bench/lineitem.parquet' (FORMAT PARQUET, COMPRESSION SNAPPY);"
cargo bench -p cq-workload-e2e --bench load --features parquet --locked -- target/load-bench
```

```sh
cargo xtask bench --tpch --sf 0.1                 # TPC-H-derived; automatic release build and DuckDB oracle; exact SF 0.1 gate
cargo xtask bench --tpch --sf 1                   # TPC-H-derived; repeats the SF 0.1 correctness gate before SF 1 measurement
cargo xtask bench --tpch --sf 0.1 --runs 9        # one explicit warm execution, median of an odd number >= 3; never a time gate
cat target/bench/tpch-0.1.tsv target/bench/tpch-1.tsv
cat workloads/tpch-methodology.txt workloads/tpch/coverage.tsv
```
