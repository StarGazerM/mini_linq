# Private staff answers

This directory contains a focused snapshot of the completed basic
query-lowering answers inside the private `StarGazerM/mini_linq` repository.

- Source branch: `codex/all-answers`
- Source commit: `f083edd384b7ef376b54670d2ee792305a610370`
- Answer implementation commit: `01e2c3b`

The mirrored files are:

- `crates/cq-compiler/src/passes.rs`, which connects the three public
  `compile_*` functions to the staff implementation; and
- `crates/cq-compiler/src/staff_r1.rs`, which implements CQ to
  `RelationalPlan`, `RelationalPlan` to `IndexRequirements`, and
  `IndexRequirements` to staged Rust.

This folder is intended to be versioned in this private repository. Do not copy
it into or push it to the public student repository.

To test the answer without changing the student checkout, overlay the two
mirrored files onto a temporary copy of this repository and run:

```text
cargo test --workspace --locked
cargo check -p cq-workloads --features compiled-queries --locked
```
