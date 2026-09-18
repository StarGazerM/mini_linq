# Documentation map

## Assigned now

- [Homework 1](HW1-START-HERE.md): implement only `CQ -> RelationalPlan`.
- [Language contract](DSL.md): complete typed syntax and whole-program
  artifacts. Homework 1 needs only artifacts 1 and 2.

## Cumulative and future work

- [Basic query-lowering checkpoint](R1-START-HERE.md): Homework 1 through
  Homework 3 and the supplied pull route.
- [Optional Homework 4](HW4-OPTIONAL.md): negation and aggregation after the
  basic checkpoint.
- [Optional project tracks](OPTIONAL-PROJECT-TRACKS.md): later backend and
  multi-query directions.

## Current compiler design

- [Staged borrowed execution](STAGED-BORROWED-EXECUTION.md): borrowed outputs,
  lazy reuse and existence through the existing index, Access and Pipeline stages.
- [Compiler boundaries](IR-STAGE-REVIEW.md): public macro families, native
  producer syntax, composition laws and remaining implementation limits.
- [Compiler structure](COMPILER-STRUCTURE.md): package dependencies,
  module ownership, feature isolation and teaching extraction.
- [Compiler names](NAME-AUDIT.md): crate roles, implementation modules, all macro
  entries and caller-bound names, with their exact boundary differences.
- [Access refinement](ACCESS-REFINEMENT.md): representation choices, complete pair algorithms, current views and exact pruning.
- [Rust expressions](RUST-EXPRESSIONS.md): binding, evaluation, and scheduling rules.
- [Extended compiler research manuscript](formal/README.md):
  interacting host-expression dependencies, replaceable compilation paths,
  language judgments, administrative rules, conditional proofs, source
  correspondence, and executable counterexamples. This is an incomplete
  correctness argument with explicitly recorded obligations.
- [Planner implementation](PLANNING.md): transformation
  ownership, administrative rules, and the indexed materialized route.
- [Planning verification](PLANNING-VERIFICATION.md): 25 authored RA fixtures,
  executable reference tests, and the planner/backend acceptance gates.

## Staff course planning

- [Rust tutorial design](RUST-TUTORIAL-DESIGN.md)
- [Semester plan](semester.md)
- [YA teaching extraction](../teaching/ya/README.md): independently buildable
  Project 2 slice with separate scheduling and semijoin exercises.

The compiler and course-planning references explain why the project is shaped this way. They
are not required reading or submission specifications. The assignment
handout, typed contracts, and tests define student work.
