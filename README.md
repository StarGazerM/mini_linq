# MiniLinq

MiniLinq compiles conjunctive queries written inside Rust into Rust iterator
programs at macro-expansion time. Each stage is a small language whose
complete program is checked by its own contract, and every pass except the
printer has an independent check relating its source and target programs.
The caller binds every stage boundary, supplies a query-independent policy and
declares facts about its host data; the compiler contains no data-structure
dispatch or fallback backend. The printed program is one Rust block over the
`cq-storage` traits that evaluates to an iterator of answer tuples and borrows
host data instead of copying it.

```text
CQ → VO → RA → IX → Physical → Structures → Rust
```

[Design principles and invariants](doc/DESIGN.md) ·
[examples](examples/README.md) · [workload corpus](workloads/README.md) ·
[formal specification](doc/formal/README.md). Grammars and APIs are documented
in each crate's `//!` docs.

## Layout

| Folder | Contents |
|---|---|
| `languages/` | One crate per language (CQ, VO, RA, IX, Physical, Structures) plus shared `cq-syntax`: program, parser, printer, local contract |
| `passes/` | One crate per pass: `compile` and an independent `check`; `cq-print` and `cq-physical-realization` print Rust |
| `runtime/` | `cq-storage` (access traits, structures, answer sets), `cq-protocol`, `cq-state-providers`; no compiler dependency |
| `facade/` | `cq-stage` (typed macro continuations) and `mini-linq` (procedural entry points) |
| `examples/` | Composition, inspection and swapped/stacked/diverged stages |
| `workloads/` | Authored query corpus, deterministic datasets, SQL oracle and golden answers |
| `test-support/` | Relational fixtures: per-stage scenarios, databases and set evaluators |
| `tools/` | Gate scripts and the regression target inventory |
| `doc/formal/` | Formal specification, rule inventory and executable paper checks |

`xtask/` is the workload tool (`cargo xtask`); `teaching/ya/` holds the
YA-schedule acceptance test run by `cq-reduce`.

## Gates

CI (`.github/workflows/ci.yml`) runs, in order:

```sh
cargo fmt --all -- --check
python3 tools/check_dependencies.py --isolate
bash tools/test_regressions.sh
python3 tools/check_dependencies.py --consumers
python3 tools/check_features.py
cargo test --workspace --locked --no-fail-fast
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path xtask/Cargo.toml --locked
cargo run --manifest-path examples/composition/Cargo.toml --locked
python3 doc/formal/check.py
```

`tools/test_regressions.sh` runs every target in
`tools/regression-targets.tsv` (workspace tests, examples, external manifests
and scenario scripts), continues past failures, and writes one log per target
plus `results.tsv` under `target/regression-logs/`.

Other tools:

```sh
python3 tools/check_dependencies.py --frontends          # invoke each leaf-feature macro
python3 tools/check_expansions.py DIR [--compare PREV]   # capture/compare physical_query expansions
make -C doc/formal pdf                                   # build doc/formal/build/compiler.pdf
```

Ordinary workspace checks do not build DuckDB; only the workload oracle does.
