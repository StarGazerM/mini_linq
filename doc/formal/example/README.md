# Executable paper checks

This driver now checks **CQ → RA → IX**. The old whole-query physical backends
and their generated `run` APIs have been removed. The driver does not synthesize
replacement physical assignments or skip the new Physical boundary.

```sh
cargo run --manifest-path doc/formal/example/Cargo.toml --locked
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin computed_key
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin argument_placement
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin constraint_planning
cargo run --manifest-path doc/formal/example/Cargo.toml --locked --bin access_representations
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
backend snapshots are retired. Those queries require selected physical rules
and a host contract before executable compiler comparison can resume.

`access_representations` is a separate local Access-boundary example. Its authored
Access quotations compare hash membership, exact bounded bitmaps and a coarse
prefilter followed by exact membership against independent input membership on
264 cases. Its runtime representation choice is ordinary Rust. It validates no
CQ-to-Physical route, YA schedule, zone-map provider, or optimizer cost model.
Its `bind PAT <- (EXPR)` clauses describe dependent enumeration; the selected
pull endpoint lowers them through named Pipeline definitions before emitting
Rust adapters.

The integrated new Physical realization tests live in
`crates/mini-linq/tests/physical_realization.rs` and `physical_variants.rs`; they
cover their explicitly admitted host-expression domain, not these retired
whole-query backends.
