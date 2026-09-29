# Workloads and the DuckDB oracle

A correctness corpus of 20 complete positive conjunctive queries: ten classic
join shapes (`src/queries/classic/`) and ten integer kernels abstracted from
TPC-H/TPC-DS join topologies (`src/queries/retail/`). The retail kernels run
on synthetic data and are not TPC queries or benchmark results. Each query
leaf is authored Rust and the only source of its syntax and meaning: it
invokes `::mini_linq::workload_query!` once, which emits the typed CQ factory
and the metadata the catalogs in `src/catalog/` register.

Generated data, visible oracle SQL and golden answers are derived artifacts:

```text
workloads/data/<suite>/<case>/<scenario>/<scale>/{manifest.txt,inputN.csv}
workloads/sql/<suite>/<case>.sql
workloads/golden/<suite>/<case>/<scenario>/<scale>.csv
```

Scenarios are `coverage` (nonempty result), `empty-input` (one input empty)
and `no-match` (nonempty inputs, empty result; not applicable to
`cartesian-product`). Scales are `tiny` (checked in), `medium` and `large`
(generated locally, ignored by Git). The SQL oracle deduplicates every input
and the result, so golden CSVs are sorted distinct answer sets.

## Commands

```sh
cargo xtask list                                        # the 20 cases and their purpose
cargo xtask generate all tiny --scenario all            # data + SQL; selection: all|classic|retail|<case>
cargo xtask golden all tiny --scenario all              # compare golden rows with a fresh DuckDB run
cargo xtask golden all tiny --scenario all --write      # regenerate golden answers (explicit)
cargo xtask explain triangle --through index-requirements
```

`generate` defaults to `all tiny --scenario coverage` and never rewrites a
query module. `golden` re-runs Cargo with the `oracle` feature, so only it
builds DuckDB. `explain` runs the typed CQ → RA → IX passes (`--through` is
`cq`, `relational-plan` or `index-requirements`) and writes the complete
programs to `target/mini-linq-explain/<case>/`. `cargo xtask differential`
currently reports that it is unavailable.
