# YA teaching extraction

The old executable export and its materialized-backend profile have been removed.
A standalone teaching release must now follow the same boundaries as the compiler:

```text
CQ → RA → IX → Physical → Structures → Rust
```

Logical YA scheduling remains implemented in
[`hypergraph/ya.rs`](../../crates/cq-query-lowering/src/hypergraph/ya.rs), with
checked administrative rules and independent relational oracle tests. The retained
[`ya_schedule.rs`](ya_schedule.rs) is an acceptance-test source for that logical
exercise: it checks upward/downward dependencies, complete reduction before
assembly, and singleton, chain, branching, composite, disconnected, and repeated
occurrence cases. It is not a standalone Cargo release.

Run the current logical foundation with:

```sh
cargo test -p cq-query-lowering --all-features --locked
```

The shared [relational fixtures](../../test-support/relational) and
[workload queries, datasets, and golden results](../../workloads) remain available.
The executable [heterogeneous composition](../../examples/composition/README.md)
probes host maps in place; the [full-chain test](../../crates/mini-linq/tests/physical_query.rs)
realizes the acyclic planner's semijoin passes as `semi` operators answered by
membership tests over built structures.

A new YA release therefore requires checked expansion fixtures for that
semijoin realization before an exporter or physical exercise can be supplied. The logical exercise and
physical implementation must have separate acceptance tests; removing a backend
must not silently replace its semantic obligations.
