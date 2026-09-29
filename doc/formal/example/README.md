# Executable paper checks

These drivers exercise **CQ → CQ-NF → VO → RA → IX**, optionally inserting VO
reductions. They stop at IX and supply no physical policy; they neither exercise
nor bypass later stages to obtain executable query output.

```sh
cargo run --manifest-path doc/formal/example/Cargo.toml --locked
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin computed_key
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin argument_placement
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin constraint_planning
```

The four programs write typed CQ/NF/VO/RA/IX `.tokens` payloads and print plan
summaries under `doc/formal/build/logical-*/`; the default and `computed_key`
programs also write `validation.txt`. An optional argument chooses the output
directory. These are typed pass outputs, not captured macro expansions.
The input is borrowed without mutation. NF rewrites a clone, VO embeds NF,
RA is standalone, and IX retains RA unchanged.

- The default program checks the YA join tree, exact VO/RA expectations and
  index descriptors for the `R--S--T` query. Its independent whole-tuple oracle
  checks a fixed answer and duplicate/empty-input cases.
- `computed_key` checks an exact VO schedule and IX for a computed join argument,
  plus a separate fixed oracle answer.
- `argument_placement` checks VO equality and RA positions: `g()` precedes
  matching and the `f(x)` guard follows binding `x`. It executes no generated Rust.
- `constraint_planning` checks automatic and independently authored RA/IX
  contracts. Its oracle runs on eight input presentations: base, duplicate,
  all-empty and five individually empty inputs. It does not claim that automatic
  planning discovers the authored plan.

The boundary helper checks local contracts, reduction correspondence when used,
and elaboration correspondence. It does not call order or index correspondence
checks. None of these drivers compares generated-query execution with its oracle.
The integrated runtime chain is exercised separately by
[physical_query.rs](../../../facade/mini-linq/tests/physical_query.rs).
