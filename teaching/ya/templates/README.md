# Project 2: Yannakakis compilation

Implement two algorithms inside a supplied compiler. The first schedules
semijoin reductions of an acyclic query. The second compiles one relational
semijoin into exact hash-indexed Access operations.

Read these files in order:

1. `crates/mini-linq/examples/ya_chain.rs` and
   `crates/mini-linq/examples/semijoin.rs`: the query and the lower
   boundary at which its physical operator can be tested independently.
2. `crates/cq-ir/src/relational_plan.rs`: named relation results and semijoin
   well-formedness. A semijoin preserves its left heading.
3. `crates/cq-plan/src/graph/ya.rs`: implement the two sweeps.
   `rules.rs` supplies checked `Up`, `Down`, and `Freeze` transitions;
   `crates/cq-plan/src/proof.rs` supplies a checked join tree.
4. `crates/cq-realize/src/hash_materialized/semijoin.rs`: implement the
   physical operator. `rust_access_plan.rs` defines its target clauses.

For a join-tree edge from parent `p` to child `c`, the upward sweep replaces
`p` by `p ⋉ c` after all descendants of `c` have been processed. The downward
sweep replaces `c` by `c ⋉ p` after the parent has received its downward
reduction. Each edge is visited once in each direction. `Freeze` permits
assembly only after both complete sweeps.

For sets `L` and `R` with common attributes `K`, physical lowering must preserve

```text
L ⋉ R = { l in L | there exists r in R such that l[K] = r[K] }.
```

It must retain the complete left tuple, use every shared key column, and emit
no additional copies for multiple witnesses. If `K` is empty, every left
tuple survives exactly when `R` is nonempty. The supplied code constructs
indexes and binds each named result before its consumers execute.

Each example selects the route by ordinary Rust macro aliases. The small
frontend in `crates/mini-linq/src/lib.rs` supplies the six selected macros; its
parsing and stage helper modules are copied from the main frontend. Each macro parses
one existing IR and invokes one typed compilation edge. Generated Access
regions call a separate collection macro, which passes through the explicit
Pipeline IR before Rust generation. There is no query interpreter in the
runtime of the exported program.

The export contains seven packages: `cq-ir`, `cq-plan`, `cq-index`,
`cq-realize`, `cq-execution`, `cq-pipeline`, and `mini-linq`. Only the selected
graph, named-result index, materialized, pull and collection implementation
modules are present. Their public APIs have the same module paths as in the
parent compiler. The package roots and Cargo metadata include those modules;
the implementation files and their independent checks are the production code.
Unselected scheduling, indexing, physical and endpoint implementations are
physically absent. There is no compatibility compiler facade or dependency
on the parent checkout.

`crates/mini-linq/tests/hosts.rs` selects the anonymous callable host with a macro alias and calls
the same compiled query with two different borrowed input bindings. This
demonstrates reuse inside a host without adding recursive-program semantics.

The starter intentionally reports an exercise error when either missing
algorithm is used. Check the supplied foundation first:

```sh
cargo check --workspace --locked
cargo test -p cq-ir --locked
```

Then run the two assignment suites independently:

```sh
cargo test -p cq-plan --test ya_schedule --locked
cargo test -p mini-linq --test semijoin --locked
```

The scheduler suite invokes only logical planning. The semijoin suite enters
at RA and bypasses logical planning. Once both pass:

```sh
cargo test --workspace --all-targets --locked
cargo run -p mini-linq --example ya-chain --locked
cargo run -p mini-linq --example semijoin --locked
```

The tests cover schedule legality, actual result preservation, composite and
empty keys, duplicate inputs, owned payloads, empty relations, and disconnected
query components. Output ordering comes from the supplied result interface;
the relational semantics are sets.
