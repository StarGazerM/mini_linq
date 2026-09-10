# Administrative query compilation across embedded languages

[compiler.tex](compiler.tex) is the manuscript entry point; `make` produces
[build/compiler.pdf](build/compiler.pdf). The page budget is **32 pages including
references**, using the existing 11-point, one-inch-margin layout.

The paper follows the implemented compiler: valuations and native Rust leaves,
occurrence graphs, GYO, Yannakakis reduction, dependency scheduling, named RA,
indexes, local cursors, and quotation across independently usable languages.
Four numbered algorithms specify scheduling; the inference rules specify legal
transitions. The earlier speculative provenance chapter and standalone
conformance chapter have been removed. Host assumptions and outstanding
adequacy obligations are stated together in [foundations.tex](foundations.tex).

## Reading map

| Topic | Source |
| --- | --- |
| Compiler problem and contribution | [introduction.tex](introduction.tex) |
| YA example through every IR | [overview.tex](overview.tex), [executable artifacts](example/README.md) |
| Valuations, relations and host assumptions | [foundations.tex](foundations.tex), [host_embedding.tex](host_embedding.tex) |
| Computed arguments, directional requirements and authored RA | [constraint_planning.tex](constraint_planning.tex) |
| Each IR's grammar and local judgment | [languages.tex](languages.tex) |
| Graph extraction, GYO, YA, policies, pruning and arity | [planning.tex](planning.tex), [notes coverage](PLANNING-COVERAGE.md) |
| Index selection and physical realization | [physical.tex](physical.tex) |
| Cursor semantics, quotation and pass composition | [staging.tex](staging.tex) |
| Replacing compiler components | [extensibility.tex](extensibility.tex) |
| Bounded evidence and prior work | [evaluation.tex](evaluation.tex), [related_work.tex](related_work.tex) |

[TERMINOLOGY.md](TERMINOLOGY.md) records AHV terminology and course adaptations.
[rules.tsv](rules.tsv) maps all 139 named rules to implementation symbols or
explicitly abstract cases. [check.py](check.py) checks that inventory, the 14
executable administrative commands, and 32 source fingerprints. It detects
coverage gaps and source drift; it does not check proofs or semantic preservation.

## Implemented scope

Relational proofs assume total, deterministic, representation-independent host
expressions, lawful silent value operations, and adequate Rust elaboration.
Parsing arbitrary Syn expressions does not establish these assumptions.
The compiled effect counterexample shows that lazy and materialized execution
can return different sets despite retaining clause order. Complete Rust typing,
resolution, transition simulation and effectful cursor adequacy remain obligations.

The graph path implements YA, eager selection/pruning and a logical arity cap.
A let, literal/computed/repeated/wildcard atom argument, computed head or general
host guard selects prefix planning for the whole query. General subtree placement
is illustrated by an independently authored plan, not discovered by the current
planner. Negation and aggregation parse but do not lower. Cardinality estimation,
WCOJ, sorted providers and configurable public macro continuations are not
implemented. Lower-language quotation and typed pass composition are available.

## Reproduction

Requirements: Python 3, Rust/Cargo, `latexmk`, pdfLaTeX, `pdfinfo`, and the packages
imported by `compiler.tex`, including `mathpartir`, `stmaryrd`, `algorithm` and
`algpseudocode`.

```sh
make -C doc/formal
pdfinfo doc/formal/build/compiler.pdf
make -C doc/formal example
cargo test -p cq-compiler --test stage1_production --locked
cargo test -p cq-compiler --test expression_planning --locked
cargo test -p mini-linq --test formal_counterexamples --locked
cargo test -p mini-linq --test formal_counterexamples --release --locked
```

| Experiment | Source and detailed instructions |
| --- | --- |
| GYO versus independent tree enumeration | [hypergraph.rs](../../crates/cq-compiler/src/cq_to_relational/hypergraph.rs) |
| 25 queries and independently authored RA plans | [Stage I verification](../STAGE-I-VERIFICATION.md), [reference tests](../../crates/cq-compiler/tests/stage1_reference.rs) |
| Four-policy compiled plans | [production tests](../../crates/cq-compiler/tests/stage1_production.rs) |
| Literal public-macro expansion and execution | [shape tests](../../crates/mini-linq/tests/stage1_shapes.rs) |
| Rust-expression orders and 120 schedules | [expression tests](../../crates/cq-compiler/tests/expression_planning.rs) |
| YA and authored-expression walkthroughs | [example README](example/README.md) |
| Effect counterexamples | [formal_counterexamples.rs](../../crates/mini-linq/tests/formal_counterexamples.rs) |
| Empty bindings and local contract agreement | [formal_domains.rs](../../crates/cq-compiler/tests/formal_domains.rs), [empty_bindings.rs](../../crates/mini-linq/tests/empty_bindings.rs) |

The overview asserts exact RA, indexes, both sweeps, all thirteen intermediate
arities/counts, and compiled answers on five input presentations across four
routes. The expression example compares three routes on 14 presentations;
`argument_placement` checks candidate-local versus prefix evaluation. Full data,
expected answers and generated programs remain in the executable artifacts.

The source snapshot and compiler-cleanup validation history are recorded in
[snapshot.txt](snapshot.txt) and [DIVERGENCE-RECONCILIATION.md](DIVERGENCE-RECONCILIATION.md).
The manuscript condensation changes documentation only; it does not represent
another compiler implementation or a newly repeated full-workspace test gate.
