# YA teaching extraction

The old executable export and its materialized-backend profile have been removed.
A standalone teaching release must now follow the same boundaries as the compiler:

```text
CQ → RA → IX → Physical → Rust[Access]
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
currently realizes Rename and indexed NaturalJoin. It does not provide physical
semijoin reduction, block pruning, or stored intermediate results.

A new YA release therefore requires an explicit physical semijoin provider/rule,
representation choices for reduced versions, and checked expansion fixtures before
an exporter or physical exercise can be supplied. The logical exercise and
physical implementation must have separate acceptance tests; removing a backend
must not silently replace its semantic obligations.
