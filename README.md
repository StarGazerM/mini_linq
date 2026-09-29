# MiniLinq

MiniLinq is a Rust query compiler with replaceable macro boundaries:

```text
CQ → NF → VO → RA → IX → Physical → Structures → Rust
```

An IX entry is a demand: the key set a plan applies a result at.
IX records these demands for the fixed RA plan. Physical derives their uses
from its operands; the lowering check requires each probe/semi to have a
matching demand.
Physical restates every result with one physical operator and the mode its
reads allow. Structures extends Physical with the bindings Γ: the value
answering each keyed read, supplied by a host or built over the result.
A printer emits one pull program over the `cq-storage` operation traits.
The policy that fixes those choices is the
caller's and query-independent: host facts, a join strategy, and
`#[index(Type)]` advice. The common compiler contains no data-structure
dispatch or fallback backend.

Read the [compiler structure](doc/COMPILER-STRUCTURE.md), the
[language grammars](doc/DSL.md), the [semantics of the chain](doc/SEMANTICS.md), [what is established](doc/ESTABLISHED.md),
the [formal specification](doc/formal/README.md), or the
[executable composition](examples/composition/README.md).

The [full-chain test](facade/mini-linq/tests/physical_query.rs) covers hosts
read in place through their declared keys, compiler-built hash indexes, a
user-defined index type, merge joins over built ordered maps, Yannakakis
semijoin passes as membership tests, bounded scans of an ordered host, and a
computed head column, each against an independent oracle. The printed program
copies no binding: hosts and built structures yield references, and the head
expression alone decides what the result owns. The logical prefix normalizes
CQ to NF, orders valuations in VO, optionally adds guard and semijoin
reductions, and elaborates VO to RA. Every language checks its own complete
program; every pass except the printer has an independent correspondence
check between its source and target programs; the printer is held to oracle
answers and rustc's trait guards. See
[contracts and evidence](doc/COMPILER-STRUCTURE.md#contracts-and-evidence).

```sh
cargo run --manifest-path examples/composition/Cargo.toml --offline
cargo run --example cq-inspection
cargo run --example free-variables
cargo test --workspace --offline
python3 tools/check_dependencies.py
make -C doc/formal pdf
```

The [workload catalog](workloads/README.md) keeps authored queries, deterministic
datasets, and SQL oracles independently of executable composition.
`cargo xtask explain` inspects the logical/IX prefix; it does not supply a
policy for the Physical and Structures stages. Ordinary workspace checks do not build DuckDB.

The [documentation map](doc/README.md) separates the current compiler design
from teaching tasks and research proposals.
