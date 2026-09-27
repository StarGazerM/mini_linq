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
The inventory lists the current test, example and manifest targets of the
`CQ → RA → IX → Physical → Structures → Rust` chain. CI runs the same command.

The ordinary workspace gate also sees the recovered tests:

```sh
cargo test --workspace --locked --no-fail-fast
cargo test --manifest-path xtask/Cargo.toml --locked
```
