# Private research and teaching reference

This is private staff material. It must not be pushed to the student repository
or distributed before the release policy permits it. The current checkout is a
compiler redesign in progress, not a completed answer release for every query.

The compiler boundaries are:

```text
CQ → NF → VO → RA → IX → Physical → Γ → Rust
```

`cq_lang_cq::normal::normalize` puts CQ in kernel normal form. `cq-order` orders
valuations in VO; `cq-reduce` can add guard pushdown and, for acyclic queries,
Yannakakis semijoin reductions; `cq-elaborate` turns VO into named RA. Normal
form has a contract check; order, reduction, and elaboration have correspondence
checks. Reductions erase to the input VO plan, and VO and RA plans erase to the
CQ-NF kernel up to variable renaming. `cq-index-policy` records exact-version
demands, the key sets a plan applies results at. `cq-physical-lowering` restates
every IX result with one physical operator and read mode from the caller's policy.
`cq-structure-selection` produces the bindings Γ in the `structures` section:
the value answering each keyed read, supplied by a host or built over the result.
`cq-physical-realization` prints one pull program over the `cq-storage` traits;
`cq-state-providers` supplies the selectable distinct providers. `mini-linq`
exports the procedural entry points and caller-resolved slots. The
[crate map](doc/COMPILER-STRUCTURE.md) and [grammars](doc/DSL.md) are
authoritative.

The executable [composition example](examples/composition/README.md) binds
every slot: hosts keyed on their join columns are probed in place and nothing
is built at runtime. The [full-chain test](facade/mini-linq/tests/physical_query.rs)
additionally covers compiler-built indexes, a user-defined `#[index(Type)]`,
merge joins, Yannakakis semijoin passes, bounded ordered scans and computed
columns against an independent oracle.

## Inspect actual expansions

`tools/check_expansions.py` captures the ten physical query expansions and
compares them byte for byte with a saved baseline.

For the workload catalog's typed logical prefix, use:

```sh
cargo xtask explain triangle --through index-requirements
```

The VS Code **MiniLinq: Explain query** task offers the same three stops:
`cq`, `relational-plan`, and `index-requirements`. Their complete payloads are
`01-cq.tokens`, `02-relational-plan.tokens`, and `03-index-requirements.tokens`
under `target/mini-linq-explain/<case>/`. These are typed pass outputs, not
captured macro calls. A physical policy and a host contract must be supplied
before the workload can be compiled below IX. `workload_query!` captures catalog metadata
and CQ factories; it does not generate an executable query API.

## Active verification

```sh
cargo test --workspace --locked
cargo test -p cq-order -p cq-reduce -p cq-elaborate --all-features --locked
cargo test --manifest-path xtask/Cargo.toml --locked
cargo run --manifest-path examples/composition/Cargo.toml --locked
bash tools/test_regressions.sh
python3 tools/check_dependencies.py --frontends --consumers
python3 tools/check_features.py
python3 doc/formal/check.py
```

Package-wide `--all-features` is used only for supported implementation
components: the optional `cq-lang-cq/hw4` exercise is unfinished and is not part of
ordinary workspace acceptance. Default workspace checks, macro tracing, and
logical explanation do not build DuckDB.

## Outside the language

Recursion, aggregation, negation and incremental evaluation are outside the
language; deltas are ordinary hosts under a rewrite tool, not IR constructs.
The workload data, SQL, and golden answers are retained, but generated-query
`cargo xtask differential` execution is unavailable until the catalog queries
are given a host policy. The old standalone YA exporter has been retired; the
[teaching foundation](teaching/ya/README.md) records what remains reusable and
what a new release must implement.
