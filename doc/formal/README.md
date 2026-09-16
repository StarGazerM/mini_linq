# Administrative query compilation across embedded languages

[compiler.tex](compiler.tex) builds the main manuscript. The separate
[physical supplement](physical_design.tex) develops the implemented direct
RA-to-Access edge and native Access refinements, with a
[design guide](../ACCESS-REFINEMENT.md). Both use an 11-point, one-inch-margin
layout.

The main paper follows valuations and native Rust leaves, occurrence graphs, GYO,
Yannakakis reduction, dependency scheduling, named RA, index requirements, local
cursors and quotation across independently usable languages. The supplement
adds compiler-local representation bindings, exact/zone key resources, candidate
scans, complete pair algorithms, row construction and conditional preservation.

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
| Replacing components | [extensibility.tex](extensibility.tex), [component map](../COMPILER-STRUCTURE.md) |
| Complete current boundary audit | [IR-STAGE-REVIEW.md](../IR-STAGE-REVIEW.md) |
| Bounded evidence and prior work | [evaluation.tex](evaluation.tex), [related_work.tex](related_work.tex) |

[TERMINOLOGY.md](TERMINOLOGY.md) records AHV terminology and course adaptations.
[rules.tsv](rules.tsv) maps 166 rules, including 22 native-refinement rules, to
implementation symbols or explicitly abstract cases. [check.py](check.py) checks
those links, algorithm/model correspondence, the 14 executable planner commands
and recorded source fingerprints. It does not check mathematical proofs.

[physical_rules.tsv](physical_rules.tsv) connects each physical rule to its source
function, one of the two algorithms and the finite model where applicable.
[physical_model.py](physical_model.py) checks 98,415 current-view candidate cases
and 1,280 witness cases against a pairwise oracle. Operand masks, block sizes and
paired key layouts vary, including empty and permuted separators. Targeted cases
cover correlated keys, signed `i64` boundaries, empty resources, stream support
and stale-version exact resources. This is bounded semantic evidence, not a proof
of generated Rust or arbitrary provider laws.

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
The configured relation realization supports Unit, Rename, Select, Extend,
Project, Semijoin and NaturalJoin with native Rust output expressions. It derives
logical fields and delegates Type/Field/Record to each selected provider. Tuple
sets support heterogeneous Rust columns; stock arrays and block storage require
`i64` columns. Rust checks each selected operation's trait and borrowing
requirements, including the result host's output ordering requirement.

The canonical physical route consumes real index requirements. The alternative
configured edge consumes RA and emits Rust with Access holes; Γ is ordinary
compiler state. Lexical aliases select provider and local access macros. Public
source checks diagnose invalid IR in debug and release. The direct audit checks
source-to-emitted-code correspondence; selected provider laws and expanded Rust
remain independent checking boundaries. Storage contains data structures and
traits, without runtime relational operators.

Compiler families group implementations by the boundary they own: `cq-plan`,
`cq-index`, `cq-realize`, `cq-native`, `cq-execution` and `cq-pipeline` use shared
languages from `cq-ir`. `cq-storage` holds runtime data structures; `cq-compose`
and the macro frontend supply composition. Source-removal builds check selected
modules independently. External macro probes also exercise renamed
dependencies and replacement continuations. These dependency checks do not
establish semantic substitution laws; the [ownership map](../COMPILER-STRUCTURE.md)
records the distinct obligations.

Explicit implementation entries check their own domains. The macro frontend
chooses CQ, index and realization implementations through caller-relative macro
invocations; `cq-compose` provides separate typed recipes. IX carries equality
demands without a route tag. `optimized_query!` selects a planner; a facade can
bind result indexes and eager realization explicitly. Compiled workloads follow
the same macro bindings. The materialized computation can produce either an
observed program or a plain callable query through a host macro alias. Both use
the same checked query body. Repeated borrowed full/delta inputs are supported,
while recursive program semantics and persistent index maintenance remain future
work. Direct physical lowering separately supports plain/observed item and
callable hosts, configured provider paths and provider-owned packing sizes.
Complete pair fragments select hash, nested or sort/merge control before result
rows enter the chosen provider's emission sink. Semijoin retains conservative
candidates followed by exact membership on current relation versions.

Pull Access execution lowers to Pipeline, while `push_set!` emits eager Rust
loops directly. Pipeline permits arbitrary fresh stream names, tuple binding
renaming, selective retention and early projections, while retaining a linear
single-consumer chain and terminal Distinct. Eager-set, distinct-iterator and
pre-Distinct-stream holes have different observation contracts.

The [YA release profile](../../teaching/ya/README.md) extracts the selected packages with
unselected implementations absent and separate scheduling/semijoin exercises.

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
python3 tools/check_features.py
cargo test -p cq-storage -p cq-compose -p mini-linq --lib --tests --locked
cargo test -p mini-linq --test heterogeneous --release --locked
cargo test -p cq-compose --test planned_execution --locked
cargo test -p cq-compose --test expression_planning --locked
cargo test -p mini-linq --test formal_counterexamples --locked
cargo test -p mini-linq --test formal_counterexamples --release --locked
cargo test -p cq-compose -p mini-linq --test reusable_query --locked
```

| Experiment | Source and detailed instructions |
| --- | --- |
| GYO versus independent tree enumeration | [hypergraph.rs](../../crates/cq-plan/src/graph/hypergraph.rs) |
| 25 queries and independently authored RA plans | [Stage I verification](../PLANNING-VERIFICATION.md), [reference tests](../../crates/cq-compose/tests/relational_reference.rs) |
| Four-policy compiled plans | [production tests](../../crates/cq-compose/tests/planned_execution.rs) |
| Literal public-macro expansion and execution | [shape tests](../../crates/mini-linq/tests/planned_queries.rs) |
| Rust-expression orders and 120 schedules | [expression tests](../../crates/cq-compose/tests/expression_planning.rs) |
| YA and authored-expression walkthroughs | [example README](example/README.md) |
| Native hash/bitmap access and conservative prefiltering | [access_representations.rs](example/src/bin/access_representations.rs) |
| Effect counterexamples | [formal_counterexamples.rs](../../crates/mini-linq/tests/formal_counterexamples.rs) |
| Empty bindings and local contract agreement | [formal_domains.rs](../../crates/cq-compose/tests/formal_domains.rs), [empty_bindings.rs](../../crates/mini-linq/tests/empty_bindings.rs) |
| Caller-relative macro choices | [macro_routing.rs](../../crates/mini-linq/tests/macro_routing.rs) |
| Complete pair algorithms and multiplicity | [pair_algorithms.rs](../../crates/mini-linq/tests/pair_algorithms.rs) |
| Pull/push observations and authored Pipeline programs | [execution_models.rs](../../crates/mini-linq/tests/execution_models.rs) |
| Typed and mixed relation representations | [physical_typed.rs](../../crates/mini-linq/tests/physical_typed.rs) |

The overview asserts exact RA, indexes, both sweeps, all thirteen intermediate
arities/counts, and compiled answers on five input presentations across four
routes. The expression example compares three routes on 14 presentations;
`argument_placement` checks candidate-local versus prefix evaluation. Full data,
expected answers and generated programs remain in the executable artifacts.

The current source identity and consolidated validation are recorded in
[snapshot.txt](snapshot.txt). A focused component test gate does not imply validation
of optional workloads or a performance benchmark.
