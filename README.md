# MiniLinq

MiniLinq is a Rust query compiler with replaceable macro boundaries:

```text
CQ → RA → IX → Physical → Structures → Rust
```

IX names the indexes required by the fixed RA plan and records their uses.
Physical restates every result with one physical operator and the mode its
reads allow; Structures binds every keyed read to a host key fact or a Rust
type built over the result; a printer emits one pull program over the
`cq-storage` operation traits. The policy that fixes those choices is the
caller's and query-independent: host facts, a join strategy, and
`#[index(Type)]` advice. The common compiler contains no data-structure
dispatch or fallback backend.

Read the [compiler structure](doc/COMPILER-STRUCTURE.md), the
[language grammars](doc/DSL.md), the [paper](doc/formal/build/compiler.pdf),
or the [executable composition](examples/composition/README.md).

The [full-chain test](crates/mini-linq/tests/physical_query.rs) covers hosts
read in place through their declared keys, compiler-built hash indexes, a
user-defined index type, merge joins over built ordered maps, Yannakakis
semijoin passes as membership tests, bounded scans of an ordered host, and a
computed head column, each against an independent oracle. The printed program
copies no binding: hosts and built structures yield references, and the head
expression alone decides what the result owns. Logical YA planning and its
independent relational oracle tests remain in `cq-query-lowering`.

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
