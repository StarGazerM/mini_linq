# Administrative query compilation across embedded languages

[compiler.tex](compiler.tex) builds the main manuscript. The separate
[physical supplement](physical_design.tex) develops the implemented direct
RA-to-Access edge and native Access refinements, with a
[design guide](PHYSICAL-DESIGN.md). Both use the existing 11-point, one-inch-margin
layout. The supplement is independent of the main manuscript's page budget.

The main paper follows valuations and native Rust leaves, occurrence graphs, GYO,
Yannakakis reduction, dependency scheduling, named RA, index requirements, local
cursors and quotation across independently usable languages. The supplement
adds compiler-local physical bindings, exact/zone key resources, candidate scans,
join witnesses and conditional preservation. No whole-program physical Context
or speculative administrative overlay remains.

## Reading map

| Topic | Source |
| --- | --- |
| Compiler problem and contribution | [introduction.tex](introduction.tex) |
| YA through every canonical IR | [overview.tex](overview.tex), [executable artifacts](example/README.md) |
| Relations and host assumptions | [foundations.tex](foundations.tex), [host_embedding.tex](host_embedding.tex) |
| Computed arguments and authored RA | [constraint_planning.tex](constraint_planning.tex) |
| Language grammars and local judgments | [languages.tex](languages.tex) |
| GYO, YA, scheduling and arity | [planning.tex](planning.tex), [coverage](PLANNING-COVERAGE.md) |
| Canonical indexes and physical realization | [physical.tex](physical.tex) |
| Macro granularity and research comparison | [architecture_placement.tex](architecture_placement.tex) |
| Implemented physical choices, laws and algorithms | [access_refinement.tex](access_refinement.tex), [rule map](physical_rules.tsv) |
| Cursor semantics, quotation and pass composition | [staging.tex](staging.tex) |
| Replacing components | [extensibility.tex](extensibility.tex), [module map](../COMPILER-STRUCTURE.md) |
| Complete current boundary audit | [IR-STAGE-REVIEW.md](../IR-STAGE-REVIEW.md) |
| Bounded evidence and prior work | [evaluation.tex](evaluation.tex), [related_work.tex](related_work.tex) |

[TERMINOLOGY.md](TERMINOLOGY.md) records AHV terminology and course adaptations.
[rules.tsv](rules.tsv) maps 140 baseline rules and 14 Access-refinement rules to
implementation symbols or explicitly abstract cases. [check.py](check.py) checks
those links, algorithm/model correspondence, the 14 executable planner commands
and recorded source fingerprints. It does not check mathematical proofs.

## Semantic and implementation limits

Relational proofs assume total deterministic host expressions, lawful silent
value operations and adequate Rust elaboration. Arbitrary Syn syntax does not
establish these assumptions. The effect counterexample demonstrates why complete
Rust typing, resolution and effectful cursor adequacy remain separate obligations.

The CQ graph route implements YA, eager selection/pruning and a logical arity cap.
Computed or nonvariable atom arguments, lets, computed heads and general guards
select prefix planning. General subtree placement is demonstrated by authored RA,
not discovered by the planner. Negation and aggregation parse but do not lower.
Cost-based physical search, WCOJ, bags and factorized execution are not implemented.
The direct array route has its own narrower domain documented in the supplement.

The canonical physical route consumes real index requirements. The alternative
configured edge consumes RA and emits Rust with Access holes; Γ is ordinary
compiler state. Lexical aliases select provider and local access macros. Public
source checks diagnose invalid IR in debug and release. The direct audit checks
source-to-emitted-code correspondence; selected provider laws and expanded Rust
remain independent checking boundaries. Storage contains data structures and
traits, without runtime relational operators.

## Reproduction

Requirements: Python 3, Rust/Cargo, `latexmk`, pdfLaTeX, `pdfinfo`, and the packages
imported by `compiler.tex`, including `mathpartir`, `stmaryrd`, `algorithm` and
`algpseudocode`.

```sh
make -C doc/formal
pdfinfo doc/formal/build/compiler.pdf
pdfinfo doc/formal/build/physical_design.pdf
make -C doc/formal model
make -C doc/formal example
cargo test -p cq-storage -p cq-compiler -p mini-linq --lib --tests --locked
cargo test -p mini-linq --test heterogeneous --release --locked
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
| Native hash/bitmap access and conservative prefiltering | [access_representations.rs](example/src/bin/access_representations.rs) |
| Effect counterexamples | [formal_counterexamples.rs](../../crates/mini-linq/tests/formal_counterexamples.rs) |
| Empty bindings and local contract agreement | [formal_domains.rs](../../crates/cq-compiler/tests/formal_domains.rs), [empty_bindings.rs](../../crates/mini-linq/tests/empty_bindings.rs) |

The overview asserts exact RA, indexes, both sweeps, all thirteen intermediate
arities/counts, and compiled answers on five input presentations across four
routes. The expression example compares three routes on 14 presentations;
`argument_placement` checks candidate-local versus prefix evaluation. Full data,
expected answers and generated programs remain in the executable artifacts.

The current source identity and consolidated validation are recorded in
[snapshot.txt](snapshot.txt). The three-crate test gate does not imply validation
of optional workloads or a performance benchmark.
