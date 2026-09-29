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
| 9 · Oct. 26–30 | Leapfrog Triejoin | Trace trie cursors under supplied valuation orders | Veldhuizen, [*Leapfrog Triejoin*](https://www.openproceedings.org/2014/conf/icdt/Veldhuizen14.pdf), §§1, 3.1–3.5; **practicum 3** |
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
current physical-lowering checkpoint contains the `mini-linq` proc-macro
facade, Syn-backed CQ, CQ-NF, VO, RA, IX, Physical, and Structures programs,
their local language contracts, and tests. Its basic route has three
homework boundaries. The CQ-to-RA chain lives in `languages/cq-lang-cq/src/normal.rs`,
`passes/cq-order/src/lib.rs`, and `passes/cq-elaborate/src/lib.rs`; later stages
live in `passes/cq-index-policy/src/lib.rs` and the `cq-physical-lowering`,
`cq-structure-selection`, and `cq-physical-realization` crates:

1. **Basic HW1:** CQ normalization, written-order coarse valuation planning,
   then elaboration to a complete named RA program;
2. **Basic HW2:** RA to IX, preserving that entire program while adding
   exact first-use index demands; and
3. **Basic HW3:** IX plus a query-independent policy to Physical, then
   Structures, then Rust.

The third edge is three recorded decisions. Physical fixes one physical
operator per result and the mode of every read; the Structures program
records the bindings Γ, the host or built value answering each keyed read;
`structures_to_rust!` prints one Rust block over the `cq-storage` traits (`Build`, `IndexRead`,
`ExactMembership`, `OrderedRead`, `Canonical`) that evaluates to an
iterator of answer tuples in head order. A program over hosts alone
evaluates to the lazy iterator itself; one that builds structures collects
its answers inside the block, since the structures live only for the block.
The caller consumes the iterator however it likes. Rust may inline the
iterator combinators later, but the printed program does not require fusion.
Logical `rN` names and the printed program's bindings are intentionally
different dataflow namespaces. A later
execution-control checkpoint supplies the push target and asks students for
the typed lowering in Stage 2; later releases add the remaining algorithmic
stages:

| Stage | Required implementation | Points |
|---|---|---:|
| 1. Logical and physical lowering | Complete Basic HW1 (CQ → NF → VO → RA), HW2 (RA → IX), and HW3 (IX → Physical → Structures → Rust) | 6 |
| 2. Execution control | Study the supplied pull program, then lower the same Γ to a staff-supplied push target without changing the chosen structures | 4 |
| 3. Semijoin reduction | Implement a semijoin and both Yannakakis passes over a supplied join tree | 8 |
| 4. Triejoin | Implement leapfrog intersection and variable-at-a-time enumeration using a supplied **valuation order** and tries | 12 |
| 5. Semi-naive evaluation | Maintain full and delta relations, remove duplicates, and run to a fixed point | 10 |
| Final presentation | Demonstrate one result and explain the relevant choices, evidence, and limitations | 10 |
| **Total** | | **50** |

The stages are submitted as three releases: **R1** (Stages 1–2) at the end of
Week 5, **R2** (Stages 3–4) at the end of Week 10, and **R3** (Stage 5) at the
end of Week 14. Required recursion will be bounded to linear positive rules
unless the starter code supplies general delta-rule generation.

Every release consists of code and tests—no proposal or long report. In the
current checkout, each pass test pins `compile` on the 30-scenario per-stage
corpus to the checked-in next-stage program, runs the pass's `check`, rejects
mutated targets, and compares answers on generated databases against an
oracle (for example, elaboration evaluates both the CQ and the RA). Separate
full-chain integration tests expand the facade macros over host values,
compile and run the printed Rust, and compare its answers. Any additional
release-specific grading harness is specified with that release. Later
releases time complete query calls; any per-access or algorithm-specific
instrumentation is explicit later Rust-to-Rust work.
DuckDB is a diagnostic baseline; the goal is to explain differences, not beat
it. There is no leaderboard and no novelty requirement. An optional extension
does not replace a required stage unless a later release-specific rubric says
so.

The recommended Optional Homework 4 is the guided
[Ascent-style negation and aggregation extension](HW4-OPTIONAL.md). The CQ
parser already accepts negation and aggregation body items, but the baseline
CQ contract rejects them. Students extend the contract and safety judgment,
add logical `AntiSemijoin` and `AggregateApply` operators to RA, extend demand
derivation, and lower the new operators through Physical and Structures to
the printed Rust program. HW4 is opt-in and does not replace or block a
required release stage.

Further advanced-track designs are recorded in
[Optional project tracks](OPTIONAL-PROJECT-TRACKS.md): push with Tokio messages
and a possible IVM continuation, Rayon parallel physical lowering, and several
queries with an exact globally shared demand set. Concrete Rust column types
are already baseline. Staff supplies any new target IR, checker, emitter, dependencies,
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
semantics of `mini_linq!`, `structures_to_rust!`, or another baseline macro. Tracks whose
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
parsing and token emission. Staff also supplies the `mini_linq!`, `cq_to_ra!`,
`index_demands!`, `lower_physical!`, `select_structures!`, and
`structures_to_rust!` proc-macro entries. `mini_linq!` checks the CQ
contract and forwards to the caller-resolved `cq_to_ra!`; each lowering entry
parses its `TokenStream` into the typed source for its stage, calls the typed
functions, and forwards the result to the next caller-resolved macro
(`structures_to_rust!` emits the final Rust block instead). Grammar
and contract failures alike become span-aware `syn::Error` diagnostics,
in debug and release builds, emitted as `compile_error!`. Students implement
the designated typed stages behind those entries.
`cq_lang_cq::normal::normalize(&cq::Program)` produces CQ-NF;
`cq_order::compile` with `Written` and `Coarse` produces a VO program;
`cq_elaborate::compile` produces the named `ra::Program`.
`cq_index_policy::compile(&ra::Program)` returns
`syn::Result<ix::Program>`, preserving that full program and adding the exact
index demands.
`cq_physical_lowering::compile(&ix::Program, &Policy)` and
`cq_structure_selection::compile(&physical::Program, &Policy)` record the
physical operators and the bindings Γ answering each keyed read, and
`cq_physical_realization::compile(&structures::Program, &Endpoints)` returns
`syn::Result<syn::Expr>`: one Rust block over the `cq-storage` traits. Each
pass borrows its source and constructs its typed target; `quote!` and
`syn::parse2` are used where appropriate, particularly for Rust expressions.

Each language exposes `contract::check(&Program)`, a span-aware check of
that program alone. Each pass's `compile` checks its source contract and
ends by checking its target's. Each pass except realization also exposes a
separate `check(&source, &target, …)` that verifies correspondence when a
test or tool calls it: ordering and reduction include the selected options,
elaboration compares the RA kernel with the source CQ kernel by erasure up to
variable renaming, and the IX and physical edges check their own preservation
and choice obligations. Realization has no correspondence checker; its
evidence is oracle tests, rustc trait guards (`cq_storage::require`), and
expansion fixtures. There is no generic `Pass` or `WellFormedIr` trait system.
Students do not write a parser, printer, procedural-macro entry point, or
parallel Rust builder API. The final pass emits actual Rust collection
construction and access expressions rather than another descriptor language.
The printed block is evaluated in the caller's scope over caller-owned host
values and evaluates to an iterator of answer tuples. The emitted
combinators retain resumable state. Calling `next()` or a consumer such as
`any()` executes query traversal; the caller collects the root into
a collection or drains it without storing the answers.
No async runtime is needed for finite in-memory
sources. The later push
checkpoint targets an explicit staff-supplied Rust mechanism; students do not
have to invent the callback/channel IR or runtime shell.

The current starter supplies CQ-NF and VO syntax, `ra::Program`,
`ix::Program`, `physical::Program`, `structures::Program`, their local `contract`
modules, the supplied structure selection and pull-program printer, and the
pass and integration tests. Later releases add richer workload,
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
