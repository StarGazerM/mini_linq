# Private research and teaching reference

This is private staff material. It must not be pushed to the student repository
or distributed before the release policy permits it. The current checkout is a
compiler redesign in progress, not a completed answer release for every query.

The compiler boundaries are:

```text
CQ → RA → IX → Physical → Rust[Access]
```

`cq-query-lowering` owns logical planners, scheduling, derivations, and CQ-only
composition helpers. `cq-index-policy` records exact-version index demands and
join-use references. `cq-physical-selection` records supplied relation/index
providers and operation lowerings. `cq-physical-lowering` invokes those external
macros without choosing a replacement backend. `cq-example-providers` supplies
the current concrete examples. `cq-access-lowering` and `cq-pipeline-lowering`
provide execution consumers; `cq-storage` contains data structures and operation
traits. `mini-linq` exports procedural entry points. The
[design and implementation domain](doc/IX-PHYSICAL-REDESIGN.md) are authoritative.

The executable [composition example](examples/composition/README.md) combines a
borrowed slice, a hash map, and a sorted map. It reuses existing indexes and
returns borrowed non-`Clone` fields. The caller supplies the Rust function and
selects the Access consumer. Its admitted physical operators are input Rename
and indexed NaturalJoin; this does not establish general physical coverage of
the logical RA language.

## Inspect actual expansions

Capture the current integration query at every real procedural macro invocation,
including external provider and lowering callbacks:

```sh
python3 tools/trace_macros.py --test physical_realization \
  --features index-demands,physical-selection,physical-realization,access-to-set-loops \
  --macro-package cq-example-providers --out /tmp/mini-linq-physical-trace
```

The output directory must be new. `steps.md` contains the actual inputs and
outputs; `calls.json` identifies their packages and invocation sequence. The tool
instruments an isolated copy, returns the original TokenStreams, and verifies
that the source checkout stayed unchanged. A callback is part of Physical
realization; it is not another query-planning stage.

For the workload catalog's typed logical prefix, use:

```sh
cargo xtask explain triangle --through index-requirements
```

The VS Code **MiniLinq: Explain query** task offers the same three stops:
`cq`, `relational-plan`, and `index-requirements`. Their complete payloads are
`01-cq.tokens`, `02-relational-plan.tokens`, and `03-index-requirements.tokens`
under `target/mini-linq-explain/<case>/`. These are typed pass outputs, not
captured macro calls. Physical choices and a host contract must be supplied
before the workload can be realized. `workload_query!` captures catalog metadata
and CQ factories; it does not generate an executable query API.

## Active verification

```sh
cargo test --workspace --locked
cargo test -p cq-query-lowering --all-features --locked
cargo test -p cq-index-policy --locked
cargo test -p cq-physical-selection --locked
cargo test -p cq-physical-lowering --locked
cargo test -p mini-linq --test physical_realization --test physical_variants --locked
cargo run --manifest-path examples/composition/Cargo.toml --locked
python3 tools/check_dependencies.py --isolate --consumers
python3 tools/check_features.py
python3 doc/formal/check.py
```

Dependency/source isolation, individual macro boundary probes, and semantic
integration tests are separate checks. Package-wide `--all-features` is used only
for supported implementation components: the optional `cq-ir/hw4` exercise is
unfinished and is not part of ordinary workspace acceptance. Default workspace
checks, macro tracing, and logical explanation do not build DuckDB.

## Remaining migration work

General input schemas and index construction, stored intermediates, physical YA
semijoins, block pruning, and the former generated `build/query/run` host API
have not been ported. Logical YA planning and its independent oracle tests are
retained. Push/pull placement in the broader physical design remains open.

The workload data, SQL, and golden answers are retained, but generated-query
`cargo xtask differential` execution is unavailable until workload-specific
Physical assignments and a host input/output contract are supplied. The old
standalone YA exporter has been retired; the
[teaching foundation](teaching/ya/README.md) records what remains reusable and
what a new release must implement.
