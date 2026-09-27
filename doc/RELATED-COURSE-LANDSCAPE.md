# Related Course Landscape: Rust, Compilers, and Database Systems

> Status: course-design survey.
>
> Last checked: August 8, 2026.
>
> Purpose: compare existing courses with the proposed *Modern Query
> Processing* course, especially its Rust learning barrier, staged query
> compiler, and target budgets of fewer than 5,000 lines of course-owned
> machinery and fewer than 1,000 cumulative lines of student-written code.

This is a curated list of the publicly documented courses examined for this
design, not a claim to enumerate every course taught worldwide. Offerings and
assignments change, so every entry records the offering or material date used
for the comparison.

## 1. What combination are we looking for?

The target course combines five ideas that usually live in different courses:

1. Rust algebraic data types, exhaustive matching, ownership, and borrowing;
2. source programs represented as typed Rust values;
3. local well-formedness and contracts between successive representations;
4. staged metaprogramming through typed passes and `quote!`; and
5. join processing, output-size bounds, physical algorithms, work counters,
   and recursive query evaluation.

No surveyed course makes this entire chain its central organizing idea:

```text
grammar
  <-> algebraic datatype
  -> validated program value
  -> contracted typed transformation
  -> next program value
  -> emitted next-stage syntax
  -> executable query kernel and measured work
```

The closest courses fall into two groups: Rust/programming-languages courses
that do not center query processing, and database-system courses that use much
larger implementation projects.

## 2. Rust, programming-languages, and compiler courses

| Course | Offering examined | What it covers | Difference from the target course |
|---|---|---|---|
| [CMU 17-350/17-650: Safe Systems Programming in Rust](https://www.cs.cmu.edu/~aldrich/courses/17-350-fa26/) | Fall 2026 announced syllabus | Ownership, borrowing, interpreters, JIT compilation, error handling, unsafe encapsulation, traits, WebAssembly/FFI, concurrency, async, typestate, verification, and macros | Closest by breadth, but Rust is a vehicle for safe systems construction. Interpreters, verification, and macros are separate portions of a much broader systems course rather than one staged typed-pass spine. |
| [Brown CSCI 1951Q: Topics in Programming Languages](https://cel.cs.brown.edu/csci-1951q-f25/) | Fall 2025 | Compiler architecture and Rust, semantics, type inference, dataflow, pointer and information-flow analysis, JIT compilation, and a compiler implemented in Rust | Closest to rule-directed compiler reasoning. Rust is mainly the implementation language; procedural macros, quasiquotation, and contracts between successive nominal IRs are not the center. |
| [Brown CSCI 1951Q Assignment 4](https://cel.cs.brown.edu/csci-1951q-f25/assets/A4.pdf) | Fall 2025 assignment | Gives bytecode syntax and operational rules, asks for soundness reasoning, and then asks students to implement an analysis | Strong precedent for “read a rule, then implement the corresponding operation,” but the operation is a program analysis rather than a staged query lowering. |
| [Stanford CS 110L: Safety in Systems Programming](https://web.stanford.edu/class/cs110l/handouts/course-material/) | Published Winter 2022 materials | Ownership, lifetimes, references, enums, `Option`, `Result`, traits, generics, multiprocessing, concurrency, and networking | Strong Rust-mechanism introduction, but the projects are a debugger and network server rather than AST/IR transformation. |
| [Cornell CS 6120: Advanced Compilers](https://www.cs.cornell.edu/courses/cs6120/2025fa/self-guided/) | 2025 self-guided course | Representing programs, educational IRs, local/global analysis, dataflow, SSA, LLVM, loop optimization, alias analysis, JITs, and research papers | Strongest compiler-pass and IR precedent, but it assumes general implementation fluency and does not teach Rust ownership or Rust procedural metaprogramming as part of the compiler model. |
| [LMU CMSI 3802: Programming Language Concepts and Implementation](https://cs.lmu.edu/~ray/classes/pl/syllabus/) | Spring 2027 planned syllabus | Sum and product types, distinction from subtyping, ownership/borrowing/lifetimes, abstract syntax, interpreters, compilers, IRs, and virtual machines | Broad conceptual overlap, but the topics span several languages and units; the compiler toolchain uses Node/Ohm rather than a typed Rust pass pipeline. |

### Interpretation

These courses explain why the proposed Rust strand is intellectually real.
The target is not merely “teach enough syntax to finish an assignment.” It
combines a Rust value model with the representation and transformation of
programs. At the same time, the target course does not need the unsafe, JIT,
FFI, async, or general compiler-optimization breadth of these courses.

### Rust onboarding audit

The courses do **not** share one policy on prior Rust knowledge. Their actual
policies fall into four different models:

| Course | Is prior Rust assumed? | Published onboarding | First substantial use | Judgment |
|---|---|---|---|---|
| CMU 17-350/17-650 | No prior Rust prerequisite is stated; prior systems-programming knowledge is required | Rust is taught throughout the course. The announced schedule begins with Rust, interpreters, and a warm-up interpreter before later meetings on error handling, unsafe code, traits, crates, concurrency, verification, and macros. | An interpreter in the first two weeks | **Rust taught through systems construction.** This is not a beginner programming course, and the Fall 2026 offering had not yet begun when this survey was checked. |
| [Brown CSCI 1951Q](https://cel.cs.brown.edu/csci-1951q-f25/) | No Rust prerequisite is stated; the published ramp relies on students' compiler/PL background | One meeting titled “Compiler Architecture & Rust,” *The Rust Programming Language* Chapters 1–4, then [A1: Compiler Warmup](https://cel.cs.brown.edu/csci-1951q-f25/assets/A1.md) | Add `break` and arrays across a parser, type checker, bytecode IR, code generator, and runtime boundary | **A compressed bridge, not a full Rust primer.** The warm-up explicitly says compiler/PL background should be sufficient. One lecture and four book chapters are the bridge into an already substantial compiler. |
| Stanford CS 110L | Prior Rust is not assumed; recent or concurrent systems coursework is expected | Roughly four weeks: analyzer limitations; memory safety and ownership; Cargo/basic syntax; errors, `enum`, `Option`, and `Result`; custom types; then traits and generics | [Week 3/4 file-descriptor inspection tool](https://web.stanford.edu/class/cs110l/assignments/week-3-exercises/), followed by a debugger project | **Rust is explicitly taught.** This is the strongest published model for students who know C/C++ systems programming but not Rust. |
| UChicago CrustyDB | Prior Rust is not assumed; students are told they must learn it quickly | A Week 1 Rust primer, a [course-specific Rust Guide](https://uchi-db.github.io/rust-guide/), selected Rustlings exercises, the Rust Book, and development-tool setup | The quarter-long CrustyDB project starts after the primer | **A compressed, project-specific bridge.** The guide teaches the Rust features used by CrustyDB rather than trying to be a complete Rust course. |
| Cornell CS 6120 | Rust is neither required nor taught | No Rust onboarding; the self-guided course starts open-ended compiler hacking with Bril and LLVM | “Representing Programs” is Lesson 2, followed immediately by analysis and optimization | **General implementation fluency is assumed.** Its IR/pass pedagogy is useful after a host-language bridge, not as that bridge. |
| LMU CMSI 3802 | Proficiency in one, preferably two, languages is required; Rust is only one possible prior language | Comparative coverage across many language families. Sum/product types and subtyping appear in the types unit; ownership/borrowing/lifetimes appear later in a systems-languages unit. | A compiler built with Node and Ohm, not Rust | **Rust concepts are surveyed, not operationally onboarded.** It supplies distinctions worth teaching but not a Rust project ramp. |

The practical conclusion is that the target course should say explicitly:

> Prior Rust is not required. Prior programming proficiency is required. The
> course teaches the small Rust model needed to read and transform typed query
> programs; it does not teach general systems programming in Rust.

#### What each course actually teaches that we can reuse

**Stanford CS 110L: the best exercise progression.** Its [published
schedule](https://web.stanford.edu/class/cs110l/handouts/course-material/)
makes each new mechanism do visible work:

1. inspect failures that C/C++ analyzers miss, establishing a reason for the
   type system;
2. set up Cargo and learn basic syntax through a one-to-three-hour
   [Hello World exercise](https://web.stanford.edu/class/cs110l/assignments/week-2-exercises/);
3. predict whether short ownership examples compile and explain why;
4. introduce data-carrying enums and exhaustive `match` before defining
   `Result<T, E>`;
5. use ownership, `Option`, `Result`, I/O, and structs together in a small
   useful program; and
6. reactivate the same ideas through linked lists, traits, and generics.

The compile-prediction questions are especially reusable. A student must
distinguish a moved `String`, a shared borrow, a returned reference to a local
value, and an attempted move out of a `Vec`; merely producing compiling code is
not enough. For this course, the systems examples should become query-program
values and small transformations.

**UChicago CrustyDB: the best course-specific reference model.** The Rust
Guide is organized around setup and Cargo, variables and control flow,
`Vec`/`HashMap`, ownership and borrowing, structs and methods, enums and
pattern matching, generics, `Option`/`Result`, traits, crates, linting, testing,
serialization, and concurrency. It links narrowly selected Rustlings problems
after each topic. Two choices are particularly relevant:

- lifetimes are introduced so students recognize them, but the project avoids
  making explicit lifetimes a central programming burden; and
- workspaces, tests, and project-specific types are taught early enough that
  students can navigate the supplied machine.

We should reuse the **targeted guide plus selected exercises** model. We should
not copy its whole scope: serialization and concurrency are unnecessary for the
query-compiler spine. Its pointer/stack/heap motivation also needs rewriting
for this course, where a borrow should first mean “temporary permission to
inspect this program value,” not “a safer raw pointer.”

**Brown CSCI 1951Q: the best post-onboarding compiler warm-up.** A1 asks for
two language extensions. `break` is about a 30-line cross-stage change; arrays
are about a 200-line change touching most of the compiler. Students must find
the analogous tuple implementation, extend the right representations, and add
snapshot tests. This is an excellent *read an existing staged machine and make
a coherent change* assignment. It is too large and assumes too much compiler
context to serve as a first Rust exercise.

**CMU 17-350/17-650: the best “Rust through a running artifact” model.** The
announced course moves immediately from safe systems programming to an
interpreter, then revisits Rust mechanisms as new systems domains require them.
We should reuse the early executable artifact, but not its breadth. JIT code
generation, unsafe encapsulation, FFI, distributed protocols, embedded code,
async, and verification are legitimate systems topics rather than prerequisites
for implementing typed query passes. Macros occur near the end of the announced
schedule, which also warns against treating macro internals as a beginner Rust
topic.

**Cornell CS 6120: the best ordering after Rust readiness.** “Representing
Programs” is Lesson 2, before local analysis, dataflow, SSA, and LLVM. That
ordering supports the target course's central move: once students can read
Rust values, make the program representation explicit before teaching
transformations or optimizations.

**LMU CMSI 3802: the best conceptual separation.** Its topic list explicitly
places sum/product types beside unions and subtypes, then handles pointers and
ownership later in a different systems-language unit. We should make that
separation even more concrete in Rust: an enum variant is a closed alternative
of one value type, not a subclass, and matching eliminates that sum by covering
its alternatives.

#### Coverage of the five project-specific barriers

| Course | Enum as ADT, not subtype | Borrow as access, not raw pointer | Contracts from rules | Program as Rust AST/IR value | `quote!` or raw ADT construction |
|---|---|---|---|---|---|
| CMU 17-350/17-650 | Not explicit in the announced schedule | Central | Typestate and verification appear later, but not per-pass IR contracts | Early interpreters imply program representation, but published details were not yet available | Macros are scheduled late; procedural quotation is not specified |
| Brown CSCI 1951Q | Not in the assigned Rust Book Chapters 1–4 | Compressed into the one-meeting/book bridge | Formal semantics and analysis rules are central later | Direct: A1 crosses parser, type checker, bytecode, and generator | Not a stated course focus |
| Stanford CS 110L | Direct: data-carrying enums plus exhaustive matching | Direct and repeatedly exercised | Function signatures, `Result`, and compiler guarantees, but not IR pass contracts | No | No |
| UChicago CrustyDB | Direct in the guide, followed by Rustlings enum exercises | Direct, with Rustlings move-semantics exercises | Testing and `Result`, but not rule-derived semantic contracts | The DBMS uses Rust plan structures, but AST transformation is not part of the Rust primer | Brief macro use and `derive`; not `quote!` or procedural-macro construction |
| Cornell CS 6120 | Host-language dependent | No Rust coverage | Analysis equations and pass reasoning, but not Rust contracts | Direct and central, using Bril/LLVM rather than Rust nominal IRs | No Rust metaprogramming |
| LMU CMSI 3802 | Direct conceptual distinction, mainly through ML-family languages | Comparative systems-language coverage | Semantics and compiler phases, not Rust pass contracts | Abstract syntax is central, but the implementation stack is Node/Ohm | No |

No surveyed course directly covers all five barriers. In particular, none of
the published materials makes this full equivalence the beginner spine:

```text
grammar case
  <-> Rust enum variant
  -> owned program value
  -> borrowed pass input
  -> rule-derived precondition/postcondition
  -> transformed program value
  -> quoted Rust syntax
```

That is the distinctive material this course must author rather than link to.
None of the surveyed courses presents stage contracts as an interface for AI
assistance either. A nominal input type, nominal output type, local
precondition, local postcondition, and tiny examples give both a student and an
AI tool enough bounded context to implement or review one transformation
without reconstructing the whole compiler.

#### Recommended eight-session Rust bridge

Use the second meeting of each of the first eight weeks as a cumulative bridge.
Each session should end in a small checked artifact, not a detached syntax
worksheet.

| Session | Core idea | Small assessed artifact | Best source to adapt |
|---:|---|---|---|
| 1 | Values, `struct`, `Vec`, functions, Cargo, and tests | Construct and inspect a tiny query value | UChicago setup/project guide |
| 2 | Data-carrying `enum`, exhaustive `match`, sums vs. products vs. subtypes | Define and evaluate a three-case expression/query ADT | Stanford enum lesson plus LMU's type distinction |
| 3 | Move, shared borrow, mutable borrow, scope, and `Clone` | Predict compile/fail cases, then write `analyze(&Query)` without consuming the query | Stanford compile-prediction exercises |
| 4 | `Option`, `Result`, error enums, and `?` | Validate names/arity and return a typed diagnostic | Stanford error progression plus UChicago Rustlings selections |
| 5 | A program is a value: grammar, AST, typed IR, recursive data | Translate tiny concrete syntax to an independently printable program value | Cornell “Representing Programs” |
| 6 | Contracts as executable pass boundaries | Turn one inference/sequent rule into a precondition, postcondition, and failing test | Brown's rule-to-implementation discipline, narrowed to one pass |
| 7 | Typed transformations between nominal stages | Implement one 30–80-line pass and compare exact input/output values | Brown's cross-stage warm-up pattern |
| 8 | Staging and syntax generation: raw ADT assembly versus `quote!` | Emit the same tiny Rust expression both ways and compare parsed syntax | Course-authored material; no surveyed course supplies this bridge |

Traits, iterators, and generics should be introduced only where the scaffold
uses them. Explicit lifetime design, unsafe Rust, FFI, async, JIT compilation,
concurrency, and procedural-macro plumbing remain staff machinery or optional
enrichment. Students need to understand `quote!` as a constructor for syntax;
they do not need to implement the procedural-macro transport that invokes it.

This sequence borrows Stanford's repeated conceptual exercises, Chicago's
project-specific guide, Brown's small cross-stage changes, and Cornell's early
focus on representation. It is intentionally more gradual than Brown or
Chicago because Rust is not merely the implementation language here: its ADTs,
borrows, and nominal types are the notation in which the course explains the
compiler.

## 3. Database-system and query-processing courses

| Course | Offering examined | Project model | How it handles students new to DBMS implementation |
|---|---|---|---|
| [CMU 15-445/645: Database Systems](https://15445.courses.cs.cmu.edu/spring2026/) | Spring 2026 | Cumulative C++ BusTub projects: C++ primer, buffer pool, B+ tree, and [query execution/optimization](https://15445.courses.cs.cmu.edu/spring2026/project3/) | It is an introductory database course, but C++ programming/debugging is a prerequisite. The mandatory primer must be passed before later projects, and the course explicitly does not teach C++. |
| [MIT 6.5830/6.5831: Database Systems](https://dsg.csail.mit.edu/6.5830/) | Spring 2026 | GoDB labs plus problem sets, quizzes, and either a final project or an additional lab | It explicitly assumes no prior database course and adds a Go tutorial, but expects students to be comfortable debugging, implementing, and designing software at scale. |
| [MIT GoDB Lab 2: Query Execution](https://github.com/MIT-DB-Class/GoDB-lab-s26/blob/main/lab2.md) | Spring 2026 | Students implement basic operators, scans and modifications, BNLJ, sort, aggregation, and two advanced operators in a supplied parser/binder/optimizer/runtime | A particularly relevant use of supplied machinery: the parser, binder, optimizer, indexes, expression system, and shell are provided. The editable query-execution surface still spans sixteen listed executor files. |
| [Berkeley CS 186: Introduction to Database Systems](https://cs186.gitbook.io/project) | Current project documentation checked in 2026 | Java RookieDB projects: SQL, B+ trees, joins and optimization, multigranularity locking, and recovery | Students receive a substantial cumulative teaching DBMS and modify assignment-specific files. [Project 3 Part 1](https://cs186.gitbook.io/project/assignments/proj3/part-1-join-algorithms) alone estimates 20–25 hours for BNLJ, Grace hash join, external sort, and sort-merge join, before the optimizer portion. |
| [University of Chicago CMSC 23500/33550: Introduction to Database Systems](https://classes.cs.uchicago.edu/archive/2026/winter/23500-1/assignments/) | Winter 2026 assignments; Spring 2025 detailed syllabus | Individual Rust CrustyDB project with a Rust primer and three main milestones | Closest database-course precedent for introducing Rust and DBMS implementation together. The course warns that students must learn Rust quickly, provides a course-specific Rust guide, and still uses a quarter-long engine project. |
| [CrustyDB Rust resources](https://classes.cs.uchicago.edu/archive/2026/winter/23500-1/rust/) and [DataEd 2025 paper](https://people.cs.uchicago.edu/~suhail/publication/rehman-crustydb-2025/rehman-crustydb-2025.pdf) | Winter 2026 resources and 2025 paper | Rust primer, parser-generated AST, optimizer, iterator executor, pluggable storage and transaction managers, and index interfaces | The best precedent for “slipstreaming” Rust into a DB course, but its goal remains implementation of database components rather than small contracted transformations over supplied components. |
| [Harvard CS 165: Data Systems](https://daslab.seas.harvard.edu/classes/cs165) | Fall 2025 | Large, open-ended, individual system project in C, weighted at 60%, plus midterms | Assumes basic data structures, algorithms, and C/C++ systems programming; explicitly describes the course and project as demanding and high-workload. |
| [University at Buffalo CSE 462/562: Database Systems / TacoDB](https://cse.buffalo.edu/adblab/tacodb/fa25/) | Fall 2025 | POSIX I/O, data layout, single-table processing, B-tree/index scan, join algorithms, and optional manual optimization | Uses cumulative engine components and teams. It is another example of teaching database internals through a substantial DBMS codebase rather than a narrow query-kernel scaffold. |
| [University of Washington CSE 444: Database Systems](https://courses.cs.washington.edu/courses/cse444/23sp/content/Project.html) | Spring 2023 public materials | Java SimpleDB with buffer manager, operators, lock manager, log manager, and a final architecture report | Historical but still useful evidence for the classic SimpleDB pattern: students learn internals by completing several engine subsystems in a supplied codebase. |
| [Stanford CS 346: Database System Implementation](https://web.stanford.edu/class/cs346) | Spring 2015 public offering | Individual C/C++ RedBase implementation: record manager, index, system manager, query language, and personal extension | An advanced implementation course, not a novice-facing comparison. It requires prior database-system knowledge and recommends prior C/C++ and Unix experience. |

## 4. Three recurring database-course models

### 4.1 Application/database-use course

Students learn modeling, SQL, normalization, and application APIs. They do not
implement a database engine. This model avoids the implementation barrier but
cannot teach how joins or recursive queries are physically executed.

### 4.2 Teaching-DBMS construction course

BusTub, GoDB, RookieDB, CrustyDB, TacoDB, and SimpleDB use this model:

```text
little or no prior DBMS implementation
+ strong general programming prerequisites or a short language primer
+ a large supplied teaching DBMS
+ cumulative subsystem assignments
```

The scaffold supplies integration, tests, and many components, but students
still implement storage, indexing, execution, concurrency, or recovery. This
is effective for a database-internals course and deliberately expensive in
student time.

### 4.3 Advanced seminar or research-project course

Students read papers and complete an open-ended project. The shared
implementation surface is small or absent, but the model assumes that students
can independently acquire the necessary engineering skills. It does not give
every student a common, inspectable path from semantics to measured execution.

## 5. Measured public scaffold sizes

The following measurements provide scale, not exact student-solution sizes.
They count current public starter code rather than the private code students
eventually submit.

| Scaffold | Snapshot measured | Command and boundary | Code lines |
|---|---|---|---:|
| [CMU BusTub](https://github.com/cmu-db/bustub) | `f0d9e3753482d45f2b5919da1873684600b48508` | `tokei src`; C/C++ headers, C++, and build files under `src` | 19,241 |
| [MIT GoDB](https://github.com/MIT-DB-Class/GoDB-lab-s26) | `22b3f7febd0b2257f3bc8483c6761a079c11ce15` | `tokei . --exclude '*.md' --exclude '*_test.go'`; Go only | 7,781 |
| MiniLinq measured snapshot | branch `codex/r1-tutorial`, measured August 8, 2026 | `tokei crates`; Rust only, including public tests | 3,018 |
| MiniLinq staff source subset | same working tree | `cq-ir/src`, `cq-compiler/src`, and `mini-linq/src` | 1,614 |
| MiniLinq public tests subset | same working tree | test directories of the three crates | 1,355 |

The MiniLinq working tree contained uncommitted development when measured, so
its numbers are a design snapshot rather than a released checkpoint. External
dependencies such as Rust's standard library, `syn`, `quote`, and `contracts`
are not included in any course-owned machinery budget.

## 6. Positioning the proposed course

The proposed course should not promise that students will “build a database.”
Its cumulative artifact is a small query-processing compiler plus executable
query kernels:

```text
supplied machinery
  grammar and typed IRs
  parsing and token transport
  local validators and cross-stage contracts
  in-memory relations and deterministic workloads
  correctness oracle, work counters, and reporting

student work
  query-dependent analysis
  physical-access and control decisions
  selected join or recursive-query algorithms
  correctness cases and interpretation of measured work
```

Storage engines, SQL completeness, buffer pools, B+ trees, transactions,
recovery, executable-memory management, and general compiler infrastructure
remain outside the project boundary.

## 7. Recommended enforceable budgets

| Surface | Proposed maximum | Reason |
|---|---:|---|
| Course-owned, non-generated implementation | 5,000 LOC | Keeps the entire teaching machine small enough for staff to maintain and motivated students to inspect. |
| Required student reading surface | 1,500 LOC | Prevents a small edit from hiding inside a cognitively huge framework. |
| Cumulative student-authored diff, including student tests | 1,000 LOC | Preserves time for paper discussion, formal reasoning, traces, bounds, and evaluation. |
| Editable functions across required releases | 10–12 | Makes ownership and oral explanation of every contribution realistic. |
| Typical function size | fewer than 80 LOC | Encourages one semantic decision per function and keeps contracts local. |

Line count is a guardrail, not a learning outcome. A release must also satisfy
the following acceptance conditions:

1. one exact source and target language are visible at every pass boundary;
2. each student function has an explicit precondition and postcondition;
3. each stage can be invoked and tested independently;
4. the final artifact runs supplied workloads and checks exact results;
5. work counters expose the algorithmic effect of a decision;
6. a fresh clone can reproduce the documented commands; and
7. the reference-solution diff is measured before the assignment is released.

## 8. MiniLinq checkpoint scope

The implemented canonical compiler follows

```text
CQ -> RelationalPlan -> IndexRequirements -> Physical -> Structures -> Rust
```

Each typed edge lives in its semantic module; the [compiler structure guide](COMPILER-STRUCTURE.md)
records the current ownership. A course checkpoint can assign selected edges
while supplying the other stages. The exact starter-to-reference diff must be
measured for that release before claiming a student-code budget. Historical
repository line counts above do not establish the size of the current compiler
or an assignment's solution.

## 9. Bottom line

The surveyed database courses show that students can learn DBMS internals
without previously building a database, but the common solution is a large
scaffold plus a substantial systems-programming workload. CrustyDB is the
closest precedent for teaching Rust alongside database implementation.

The proposed course occupies a different niche:

> Teach the semantics, representation, transformation, and measured execution
> of joins and recursive queries without requiring students to implement the
> surrounding database machinery.

That focus makes the 5,000/1,000-line limits plausible and aligns the software
work with the course's actual intellectual center.
