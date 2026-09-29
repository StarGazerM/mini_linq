# YA teaching extraction

The old executable export and its materialized-backend profile have been removed.
A standalone teaching release must now follow the same boundaries as the compiler:

```text
CQ → RA → IX → Physical → Γ → Rust
```

IX records demands, the key sets a plan applies results at. The `structures`
section records the bindings Γ, the value answering each keyed read.

Logical YA scheduling is the `Reductions::Tree` option of the
[`cq-reduce`](../../passes/cq-reduce/src/lib.rs) pass: it writes the upward and
downward semijoin sweeps over the GYO join tree of
[`cq::hypergraph::Analysis`](../../languages/cq-lang-cq/src/hypergraph.rs) into a
valuation plan as `reduce parent with child(shared)` / `reduce child with
parent(shared)` steps, all before the first atom step, and `cq-elaborate` turns
each into an RA semijoin. The pass's own tests
([`passes/cq-reduce/tests/reduce.rs`](../../passes/cq-reduce/tests/reduce.rs))
pin the chain schedule and evaluate every reduced plan against the relational
oracle. The retained [`ya_schedule.rs`](ya_schedule.rs) is an acceptance-test
source for that logical exercise, written against the language crates, `cq_reduce` and
`cq_elaborate`: it checks upward/downward dependencies, complete reduction before
the first atom, the exact schedules of the chain, branching, composite,
disconnected and repeated occurrence cases, and the rejection of a cyclic
query. It is not a standalone Cargo release.

Run the current logical foundation with:

```sh
cargo test -p cq-reduce --locked
```

The shared [relational fixtures](../../test-support/relational) and
[workload queries, datasets, and golden results](../../workloads) remain available.
The executable [heterogeneous composition](../../examples/composition/README.md)
probes host maps in place; the [full-chain test](../../facade/mini-linq/tests/physical_query.rs)
realizes the reducer's semijoin passes (`cq_to_ra_hypergraph!`) as `semi`
operators answered by membership tests over built structures.

A new YA release therefore requires checked expansion fixtures for that
semijoin realization before an exporter or physical exercise can be supplied. The logical exercise and
physical implementation must have separate acceptance tests; removing a backend
must not silently replace its semantic obligations.
