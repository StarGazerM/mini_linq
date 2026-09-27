# Executable paper checks

This driver now checks **CQ → RA → IX**. The old whole-query physical backends
and their generated `run` APIs have been removed. The driver supplies no
physical policy and does not skip the Physical and Structures stages.

```sh
cargo run --manifest-path doc/formal/example/Cargo.toml --locked
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin computed_key
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin argument_placement
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin constraint_planning
```

The first four programs write typed `.tokens` payloads and derivation reports
under `doc/formal/build/logical-*/`. An optional argument chooses the output
directory. These files are typed pass outputs, not captured macro expansions.
Each retained logical route emits CQ, RA and IX; IX includes named index demands
and explicit use references. The source remains unchanged across the boundaries.

- The default program checks the YA join tree, exact derivation, complete RA,
  and index descriptors for the `R--S--T` query. Its independent whole-tuple
  oracle retains the fixed answer and duplicate/empty-input cases.
- `computed_key` checks the dependency-constrained plan and IX for the computed
  join argument, plus its independent fixed oracle answer.
- `argument_placement` checks that `g()` is placed before matching and `f(x)`
  after `x` becomes available in RA. It makes no runtime evaluation-count claim.
- `constraint_planning` checks both automatic logical routes and an independently
  authored RA containing dependent expressions, projection and semijoins.
  The independent oracle retains duplicate/empty-input cases. It does not claim
  that the automatic planner discovers the authored plan.

None of these checks compares compiled query execution with the oracle. Runtime
intermediate counts, old lazy/materialized comparison results, and six-stage
backend snapshots are retired. Those queries require a physical policy
and a host contract before executable compiler comparison can resume.

The integrated chain below IX is exercised by
`crates/mini-linq/tests/physical_query.rs`; it covers the Physical, Structures
and printed-program boundaries, not these retired whole-query backends.
