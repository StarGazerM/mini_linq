# Modern Query Processing: Databases and Reasoning

> Implementing the current starter? Begin with
> [R1 physical-lowering checkpoint: Start Here](R1-START-HERE.md), then use
> [DSL.md](DSL.md) as the formal specification.

**Fall 2026 — Draft syllabus**

> How does a short declarative query become an efficient executable program?

## Course facts

| | |
|---|---|
| Dates | August 31–December 11, 2026 |
| Level | Graduate; advanced undergraduates with permission |
| Format | Lecture, paper discussion, and a cumulative implementation project |
| Exams | None |
| Office hours | TBD |
| Meeting time, location, and course site | To be announced |

The [Fall 2026 academic calendar](https://www.usu.edu/calendar/academic/?year=2026)
includes Labor Day, Fall Break, Thanksgiving recess, and No-Test Week. These
dates are marked below.

## Course introduction

This course follows conjunctive queries (CQs) from logical rules to executable
code. We study binary joins, execution control, join ordering, output-size
bounds, semijoin reduction, and worst-case-optimal joins. We then treat
positive Datalog as recursive CQ evaluation and study fixed points and
semi-naive execution. Graph, ontology-style, probabilistic, and neuro-symbolic
reasoning provide recurring examples.

The course is one focused path through semantics, algorithms, and systems—not
a survey of SQL, storage, transactions, or every form of logic. All
nonrecursive queries considered are CQs; every positive Datalog rule body is a
CQ.

By the end of the course, students should be able to:

1. explain CQ and positive Datalog semantics;
2. move among rules, hypergraphs, plans, and executable operators;
3. trace binary, semijoin, trie-based, and semi-naive evaluation;
4. use query shape, cardinality, and AGM bounds to choose an algorithm; and
5. test an implementation claim with correctness checks and interpretable work
   counters.

### Prerequisites

Students should know undergraduate algorithms and data structures, be
comfortable with mathematical notation, and program in a systems language.
Prior database coursework is helpful but not required.

## Course format

A typical meeting combines one lecture or board derivation, discussion of the
week's primary reading, and a short shared trace, code inspection, or result
comparison. There is no recurring full-class coding lab. A later meeting may
begin by comparing results from the previous activity.

Each content week has one primary reading and no reading response. Each student
leads one paper discussion. Outside class, students complete three releases of
one cumulative code project. There are no weekly problem sets, quizzes,
midterm, or final exam.

## Weekly plan and primary readings

| Week | Center | In class | Primary reading / due |
|---|---|---|---|
| 1 · Aug. 31–Sep. 4 | CQ meaning | Rules, valuations, and query hypergraphs | Abiteboul, Hull, and Vianu, [*Foundations of Databases*](https://webdam.di.ens.fr/Alice/), §§3.1–3.3, 4.1–4.2 |
| 2 · Sep. 7–11 | Binary joins | Nested-loop and hash join; build/probe state | Garcia-Molina, Ullman, and Widom, [*Database System Implementation*](https://i.stanford.edu/~ullman/dbsi.html), selected §§6.2–6.4; **Labor Day adjustment** |
| 3 · Sep. 14–18 | Pull and push | Run the same fixed plan under both control models | Neumann, [*Efficiently Compiling Efficient Query Plans for Modern Hardware*](https://www.vldb.org/pvldb/vol4/p539-neumann.pdf), §§1, 3.1–3.2; **practicum 1** |
| 4 · Sep. 21–25 | Cardinality and order | Compare left-deep relation orders and intermediate sizes | Selinger et al., [*Access Path Selection in a Relational Database Management System*](https://research.ibm.com/publications/access-path-selection-in-a-relational-database-management-system), §5 |
| 5 · Sep. 28–Oct. 2 | AGM bounds | Compute a fractional cover; compare bounds with measured work | Atserias, Grohe, and Marx, [*Size Bounds and Query Plans for Relational Joins*](https://arxiv.org/abs/1711.03860), §§2–3.1; **practicum 2; R1 due** |
| 6 · Oct. 5–9 | Yannakakis | Join trees, two semijoin passes, and enumeration | Yannakakis, [*Algorithms for Acyclic Database Schemes*](https://dblp.org/rec/conf/vldb/Yannakakis81), §§2, 4; **Fall Break adjustment** |
| 7 · Oct. 12–16 | Practical join filtering | **Hangdong Zhao guest lecture:** predicate transfer and runtime filters | Yang et al., [*Predicate Transfer*](https://www.vldb.org/cidrdb/papers/2024/p22-yang.pdf) |
| 8 · Oct. 19–23 | WCOJ principle | Contrast binary plans with variable-oriented Generic Join | Ngo, Porat, Ré, and Rudra, [*Worst-case Optimal Join Algorithms*](https://arxiv.org/abs/1203.1952), §§1–2 and §5 overview |
| 9 · Oct. 26–30 | Leapfrog Triejoin | Trace trie cursors under supplied variable orders | Veldhuizen, [*Leapfrog Triejoin*](https://www.openproceedings.org/2014/conf/icdt/Veldhuizen14.pdf), §§1, 3.1–3.5; **practicum 3** |
| 10 · Nov. 2–6 | Positive Datalog | Model, fixed-point, and proof views of recursion | *Foundations of Databases*, §§12.1–12.4; **R2 due** |
| 11 · Nov. 9–13 | Semi-naive evaluation | Track full, delta, candidates, duplicates, and termination | *Foundations of Databases*, §13.1 |
| 12 · Nov. 16–20 | Probabilistic and neuro-symbolic Datalog | Compare exact, top-k, and differentiable proof evaluation | Huang et al., [*Scallop*](https://proceedings.neurips.cc/paper/2021/hash/d367eef13f90793bd8121e2f675f0dc2-Abstract.html), §§2–4.2 |
| 13 · Nov. 23–27 | Thanksgiving break | No class | **No reading or work due** |
| 14 · Nov. 30–Dec. 4 | Project clinic | Correctness, work counters, baselines, and failure cases | No reading; **R3 due** |
| 15 · Dec. 7–11 | Synthesis | Project presentations and connections across the course | No reading; **No-Test Week** |

Week 13 is a full course break. Week 15 has no quiz or examination; project
presentations are the only graded activity, as permitted by the university's
[No-Test Days policy](https://catalog.usu.edu/pages/TRt784ev1WOEs7vivhnO).

## Assessment

| Component | Points |
|---|---:|
| Three guided in-class practica | 30 |
| Cumulative engine project | 50 |
| Paper-discussion leadership | 10 |
| Prepared participation | 10 |
| **Total** | **100** |

### In-class practica — 30 points

The three open-note, collaborative practica ask students to trace an
execution, compute or apply a bound, and interpret supplied measurements. Each
is worth 10 points: correctness (4), use of evidence (3), and explanation (3).
They assess reasoning, not mere attendance, and create no separate report or
implementation assignment.

### Cumulative engine project — 50 points

Students work individually or in pairs in a supplied Rust codebase. The
current physical-lowering checkpoint contains one `mini-linq` proc-macro crate,
Syn-backed CQ, RelationalPlan, IndexRequirements, `rust_access_plan::Plan`,
`iterator_pipeline::Pipeline`, local language checks, and tests. Its basic
route assigns three typed compilation edges. Their implementations live in
`crates/cq-plan/src/canonical/lower.rs`, `crates/cq-index/src/input/mod.rs`, and
`crates/cq-realize/src/hash_inputs/mod.rs`:

1. **Basic HW1:** CQ to a complete named RelationalPlan;
2. **Basic HW2:** RelationalPlan to IndexRequirements, preserving that entire
   plan while adding exact first-use keys; and
3. **Basic HW3:** IndexRequirements to the sequential RustAccessPlan residual.

The third lowering returns a complete `syn::File`: its visibility-matching
`Program + Storage` API, concrete
collections, construction statements, names, patterns, access expressions,
output row, eager `build`, zero-read lazy `query`, and explicit materializer
are already Rust.
One expression-position `pull!` region pairs each body occurrence with its
concrete access and the head with its output expression. The supplied pull
pass turns it into an IteratorPipeline with named `iterN` results: explicit
`scan`, left-deep binary `join`, `filter`, `project`, and `distinct` operators
whose consumers name their predecessor. A second supplied semantic pass
lowers that plan to the final lazy Rust iterator expression; token emission
afterward is direct. Intermediate bindings are streamed
iterator items rather than collected relations;
generated internal `for` loops do not implement this protocol. Rust may inline
the iterator combinators later, but pull lowering does not require fusion.
Logical `rN` names and physical lazy `iterN` names are intentionally
different dataflow namespaces. `pull` names the lowering strategy and
IteratorPipeline is its target. A later
execution-control checkpoint supplies the push target and asks students for
the typed lowering in Stage 2; later releases add the remaining algorithmic
stages:

| Stage | Required implementation | Points |
|---|---|---:|
| 1. Logical and physical lowering | Complete Basic HW1 (CQ → RelationalPlan), HW2 (RelationalPlan → IndexRequirements), and HW3 (requirements → sequential RustAccessPlan residual) | 6 |
| 2. Execution control | Study the supplied pull lowering, then lower the same accesses to a staff-supplied push target without changing the concrete access choices | 4 |
| 3. Semijoin reduction | Implement a semijoin and both Yannakakis passes over a supplied join tree | 8 |
| 4. Triejoin | Implement leapfrog intersection and variable-at-a-time enumeration using a supplied **variable order** and tries | 12 |
| 5. Semi-naive evaluation | Maintain full and delta relations, remove duplicates, and run to a fixed point | 10 |
| Final presentation | Demonstrate one result and explain the relevant choices, evidence, and limitations | 10 |
| **Total** | | **50** |

The stages are submitted as three releases: **R1** (Stages 1–2) at the end of
Week 5, **R2** (Stages 3–4) at the end of Week 10, and **R3** (Stage 5) at the
end of Week 14. Required recursion will be bounded to linear positive rules
unless the starter code supplies general delta-rule generation.

Every release consists of code and tests—no proposal or long report. The
current Stage-1 gate structurally checks the complete residual Rust program and
its occurrence-annotated Plan, checks the resulting explicit iterator plan,
applies the supplied IteratorPipeline-to-Rust semantic lowering, compiles each
fixed fixture with rustc, runs it, and checks expected rows. The fixtures
include self-join and multiple-index reuse, a proper multi-column map, a used
input consumed once, an unused input never iterated, lazy partial consumption,
and projected duplicates. These are fixed
per-fixture assertions, not generated-instance comparison with a CQ interpreter
or DuckDB. Later releases time complete query calls; any per-access or
algorithm-specific instrumentation is explicit later Rust-to-Rust work.
DuckDB is a diagnostic baseline; the goal is to explain differences, not beat
it. There is no leaderboard and no novelty requirement. An optional extension
does not replace a required stage unless a later release-specific rubric says
so.

The recommended Optional Homework 4 is the guided
[Ascent-style negation and aggregation extension](HW4-OPTIONAL.md). Students
extend the CQ body IR and safety judgment, add logical `AntiSemijoin` and
`AggregateApply` operators to RelationalPlan, extend index selection, and lower
the new operators to the occurrence-annotated concrete Rust plan. The supplied
pull route continues from those concrete accesses. HW4 is opt-in and does not
replace or block a required release stage.

Four further advanced-track designs are recorded in
[Optional project tracks](OPTIONAL-PROJECT-TRACKS.md): push with Tokio messages
and a possible IVM continuation, Rayon parallel physical lowering, several
queries with an exact globally shared index set, and arbitrary concrete Rust
column types. Staff supplies any new target IR, checker, emitter, dependencies,
and complete residual examples; a student implements the named transformation
rather than designing an IR from scratch. The document does not assign points
or promise substitution for a required stage. In particular, replacing later
semi-naive work with a completed incremental-maintenance track is a grading
decision for a future release, not an automatic consequence of using async
channels.

Optional HW4 cumulatively edits the production grammar and passes and
continues to use `mini_linq!`; its `hw4` feature gates only optional test
targets. A backend alternative gets an explicit macro and typed compiler
function when it is implemented. Cargo features do not silently change the
semantics of `mini_linq!`, `pull!`, or another baseline macro. Tracks whose
design decisions conflict are released on separate checkpoint branches unless
staff provides a combined contract and test suite.

After R1 and R2, staff will release compatible reference checkpoints. Students
may use them for later stages without losing later-stage points. Code is graded
as a team; presentation and questions are graded individually. Pair
submissions include a short contribution statement.

### Discussion and participation — 20 points

Each student leads one reading discussion: accurate central claim (4), useful
connection or counterexample (3), and questions that advance discussion (3).
The remaining 10 points reflect prepared contributions to reading discussions,
derivations, and result clinics. Attendance alone does not earn participation
credit.

## Project tools and AI policy

The project uses Rust with a procedural-macro CQ frontend. Staff supplies the
crate configuration and direct Syn syntax definitions, including derived
parsing and token emission. Staff also supplies the direct `mini_linq`,
`choose_indexes`, `input_indexes`, `hash_inputs`, `pull`, and `iterator_pipeline`
proc-macro entries. Each
entry parses one `TokenStream` into its typed source and calls the corresponding
typed compiler function directly. Grammar failures become span-aware `syn::Error`
diagnostics emitted as `compile_error!`. The typed compiler function checks
source semantics and returns the same diagnostic type; the macro entry does
not duplicate that check. Students implement only three typed functions.
`cq_plan::canonical::compile(&cq::Module)` returns
`syn::Result<relational_plan::Module>` for CQ to the exact named logical plan.
`cq_index::input::compile(&relational_plan::Module)` returns
`syn::Result<index_requirements::Module>` preserving that full plan and adding
the exact requirements.
`cq_realize::hash_inputs::compile_input_indexed(&index_requirements::Module, &syn::Path)` returns
`syn::Result<syn::File>` containing the complete staged Rust program and one
expression-position
`pull! { rust_access_plan::Plan }` region. Inside each body they inspect
the borrowed source and construct the complete target with `quote!` and
`syn::parse2`.

The open-source `contracts` crate, pinned at 0.6.10, attaches debug
postconditions where an edge has a compact query-domain
relation. The exact CQ-to-RelationalPlan and
RelationalPlan-to-IndexRequirements relations and the physical edge's
covered-input/`Ok` definedness predicate compile out in release builds. The
generated physical Rust is inspected structurally by staged fixtures.
The Basic-HW3 harness checks the Plan annotations, applies the supplied pull
operator lowering, checks the explicit IteratorPipeline, emits Rust, compiles
the resulting standalone file with rustc, executes it, and checks fixed fixture-specific
results. The physical compiler also checks the generated storage and Access
wiring against its source; this structural audit does not prove arbitrary
embedded Rust or provider implementations correct. The fixture harness is
not a dynamic CQ/DuckDB oracle and not a generic `Pass` or `WellFormedIr` trait
system.
For each IR, diagnostic `contract::check` and Boolean
`contract::well_formed` are two interfaces to the same local language
judgment. Every public typed compilation entry calls its source's diagnostic
`contract::check` before lowering, so invalid input returns a span-aware error
in debug and release builds. Macro entries do not repeat this check. Tests and
tools may also call it directly. Debug postconditions check the transformation
separately; they are not the source-validation gate.
`cq_plan::canonical::compile`, `cq_index::input::compile`, and `cq_execution::pull::compile` own nominal target
well-formedness in postconditions. The physical endpoints return typed Rust AST nodes, while each
local proc-macro entry parses its embedded Plan before calling the matching
typed function. Malformed grammar and invalid source semantics both produce
`syn::Error` diagnostics; a debug edge-contract failure means a lowering
violated its specified transformation.
Students do not write a parser, printer, procedural-macro entry point, or
parallel Rust builder API. The third pass emits actual Rust collection
construction and access expressions rather than another descriptor language.
A supplied `cq_execution::pull::compile` pass consumes the occurrence-annotated
`rust_access_plan::Plan` and produces `iterator_pipeline::Pipeline`; supplied
`cq_pipeline::iterator::compile` returns one ordinary iterator expression. The plan
retains binary operator and intermediate-record boundaries, while the emitted
combinators retain resumable state. Calling `next()` or a consumer such as
`any()` executes query traversal; `Storage::materialize` drains the root into
a collection. Other consumers can drain it without storing the answers.
No async runtime is needed for finite in-memory
sources. The later push
checkpoint targets an explicit staff-supplied Rust mechanism; students do not
have to invent the callback/channel IR or runtime shell.

The current starter supplies the logical syntax, `relational_plan::Module`,
`rust_access_plan::Plan`, `iterator_pipeline::Pipeline`, their local `contract`
modules, both supplied pull
lowerings, and the compiler test
harness. Later releases add richer workload,
reference, and benchmark infrastructure.
Students implement only the named compiler and execution passes; they do not
build SQL parsing, transactions, a storage manager, a cost-based optimizer, or
a parallel runtime.

`cargo test --workspace` checks the implemented compiler and supplied
infrastructure. Targeted edge tests are listed in the
[query-lowering handout](R1-START-HERE.md). The current checkout includes an
executable triangle baseline and the [workload and DuckDB comparison tools](../workloads/README.md).
Each released starter specifies which implementation edges students complete.

AI-assisted coding, testing, debugging, and writing are allowed. Each release
must include the root [AI-use disclosure](../AI-USE.md), naming material
assistance and how it was checked; prompt transcripts are not required.
Students remain responsible for all submitted code, measurements, claims, and
citations and must be able to explain them in the final presentation.

## Optional references

- Ullman, [*Principles of Database and Knowledge-Base Systems*](https://dblp.org/db/books/dbtext/ullman89.html), and Ceri, Gottlob, and Tanca, [*What You Always Wanted to Know About Datalog (And Never Dared to Ask)*](https://doi.org/10.1109/69.43410).
- Graefe, [*Query Evaluation Techniques for Large Databases*](https://doi.org/10.1145/152610.152611).
- Ngo, [*Worst-Case Optimal Join Algorithms: Techniques, Results, and Open Problems*](https://arxiv.org/abs/1803.09930).
- Koutris et al., [*Database Theory in Action: Yannakakis' Algorithm*](https://doi.org/10.4230/LIPIcs.ICDT.2026.25).
- Abo Khamis, Ngo, and Suciu, [*PANDA: Query Evaluation in Submodular Width*](https://arxiv.org/abs/2402.02001).

## Policies and draft status

Official university statements on accessibility, accommodations, academic
integrity, safety, and related policies will be inserted before publication.
Meeting details, deadlines, grade thresholds, and late-work rules remain to be
finalized.
