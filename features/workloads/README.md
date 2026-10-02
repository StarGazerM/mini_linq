# Workloads external consumer

Run commands from `features/workloads/`. This is a standalone Cargo workspace;
core dependencies have versions and local paths, and planners, rewrites, printers
and surface extensions are sibling external consumers. The feature-local xtask
alias works here; the root alias is requested in `/tmp/codex/w2/workloads.core_requests.md`.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test -p cq-workload-corpus --test census -- --nocapture
cargo test -p cq-workload-corpus --test compile_fail
cargo test -p cq-workload-e2e --features parquet
cargo test -p cq-workloads --features counters --test growth -- --nocapture
cargo xtask list
cargo xtask generate all tiny --scenario all
cargo xtask golden
cargo xtask fuzz --cases 200 --seed 0
cargo xtask fuzz --cases 2000 --seed 0
cargo xtask explain triangle --through structures
cargo xtask explain triangle --through final-rust
cargo xtask bench
cargo xtask bench --scale 512 --seed 7
cargo xtask bench --tpch --sf 0.1
cargo xtask bench --tpch --sf 1
```

The matrix contains all nine planner passes; merge, canonical-prefix, smallest
intersection and lazy-semijoin rewrites; recorded hash/ordered dedup types and a
caller-owned sorted payload index; pull/push printers; and slice/HashMap/BTreeMap
hosts. Four kernels use full products of 3,456 pipelines each. All 27 previous
pairwise rows are retained and extended by a deterministic, exhaustively checked
cover of the added axes. Unsupported ordered hosts and printer shapes are
individually attributed compile failures. Matrix generation is a mandatory
workspace crate so its proof work does not delay unrelated typed/extension tests.

The authored inventory remains 20 primary and 29 extended MiniLinq/SQL pairs,
including 17 extension queries, plus five typed pairs. Shared fuzz generation,
SQL evaluation and strict golden comparison use the public cq-testkit package.
There are 151 tracked goldens. Typed CSV and Parquet use the same goldens.

Synthetic timing uses an uninstrumented release build and counting uses a
separate release execution. TPC workers are independently compiled without/with
instrumentation; timed runs and both untimed counting runs each require exact
DuckDB answers. TPC query selection (`--query NAME`) creates a separately named
report; it does not imply full-catalog coverage.

Current source exposes core blockers in the generic-Fine smallest adjacency and
Hypergraph correspondence proof. Strict growth assertions and all matrix rows
remain mandatory. The live acceptance ledger and exact reproductions are in
`/tmp/codex/w2/workloads.ledger.md` and `workloads.core_requests.md`; completed
commands must be distinguished from implemented or blocked checks.
