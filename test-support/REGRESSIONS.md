# Compiler regressions recovered from before physical selection redesign

The baseline is `a054555` (`620c5aa^`). The redesign deleted acceptance tests
alongside the former backends. Their absence made the smaller replacement suite
look complete. The recovered tests retain their query bodies, expected results,
and assertions. Obsolete feature guards have been removed so Cargo attempts the
checks; removed compiler implementations have not been reinstated.
The [source inventory](regression-sources.tsv) records every recovered file or
inline test module against that baseline.

Run all recovered targets, continuing after compilation failures:

```sh
bash tools/test_regressions.sh
```

The runner returns nonzero if any target fails. Its inventory is
[`tools/regression-targets.tsv`](../tools/regression-targets.tsv); each target
has a separate log and an exit status under `target/regression-logs/`.
This inventory includes the query tests and composition example whose fixed
physical policies were removed. CI runs the same command.

The ordinary workspace gate also sees the recovered tests:

```sh
cargo test --workspace --locked --no-fail-fast
cargo test --manifest-path xtask/Cargo.toml --locked
```

## Test placement

- MiniLinq acceptance tests are back under `crates/mini-linq/tests/`, including
  block/zone access, heterogeneous and borrowed storage, join algorithms,
  expressions, hygiene, reusable queries, and planned execution.
- The former `cq-compose` test sources and their support files remain at their
  original paths. Active `mini-linq` test targets named `compose_*` run them;
  there is no resurrected `cq-compose` implementation or placeholder library.
- Deleted index and IR tests are back in their original crates. Tests from the
  removed relation and native lowerers are registered with the current physical
  lowering and provider packages. Former unit tests retain their assertions;
  imports expose the missing former APIs.
- YA teaching tests are registered as `mini-linq` targets. Four executable
  examples are again Cargo test targets, without obsolete feature prerequisites.
- Deleted macro-input and xtask tests are compiled as unit tests. The replacement
  assertion requiring rejection of later explain stages was removed.

## What the failures mean

The five replacement execution-test files and the composition example no longer
append a fixed table of generated relation/index/operator names. They currently
fail at the required `choose_physical` continuation. They retain their original
queries and result assertions, without a substitute selection implementation.
A future fix must supply general selection and an appropriate input/provider
configuration API; these tests do not invent that API.

`physical_selection.rs` now calls `given_physical_bindings!` directly with an
authored Physical IR. Its separate IX capture and structural checks test those
boundaries only; passing them does not establish automatic selection.

Recovered source tests also expose removed API names and old Access syntax.
Those compilation failures mean the assertions have not executed. They are not
proof that a ported implementation returns an incorrect result. Porting the
assertions to replacement interfaces is still required, while preserving their
behavioral obligations. The historical HW4 extension tests are included too;
these are extension obligations, not claims that every feature previously passed.

The latest recovery run attempted 76 targets: one passed (two YA scheduling
tests), 73 failed during compilation, and two ran six failing tests involving
retired Access syntax and missing documentation artifacts. The separate
Physical-IR boundary test passed two tests. See the per-target logs for
diagnostics; no failure is inverted into success.
