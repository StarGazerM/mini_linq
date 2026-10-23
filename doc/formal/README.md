# MiniLinq specification

The [paper source](compiler.tex) states the denotation, one section per
language (grammar, contract invariants, meaning, recorded decisions, rewrite
check) and one subsection per pass (compilation rules, independent check,
refinement statement). Build `build/compiler.pdf` with the command below.

| Subject | Source |
| --- | --- |
| Scope and pass obligation | [intro.tex](intro.tex) |
| Denotation, premises, blocks | [denotation.tex](denotation.tex) |
| Languages | [ir_cq.tex](ir_cq.tex), [ir_vo.tex](ir_vo.tex), [ir_ra.tex](ir_ra.tex), [ir_ix.tex](ir_ix.tex), [ir_physical.tex](ir_physical.tex), [ir_structures.tex](ir_structures.tex) |
| Sequence backend and machine grading | [backend.tex](backend.tex), [backend_ir.tex](backend_ir.tex) |
| Passes and their checks | [passes.tex](passes.tex) |
| Pass protocol and features | [composition.tex](composition.tex) |
| Context and representation changes | [context.tex](context.tex) |
| Guards and relational negation | [guards.tex](guards.tex) |
| Incremental execution | [incremental.tex](incremental.tex) |
| Datalog and parallel rounds | [datalog.tex](datalog.tex) |
| Validation | [validation.tex](validation.tex) |
| Rule classification and implementing source markers | [rules.tsv](rules.tsv) |

```sh
make -C doc/formal pdf
make -C doc/formal check
make -C doc/formal example
```

`check.py` checks rule inventory and unique marker presence only. It does not
check semantic correspondence between a rule and code, prove conformance, or
run the examples. Implementation rules summarize structural code checks; trusted semantic
premises such as Eq/Ord admissibility are not proved by them; abstract rules state mathematical laws or
metalinguistic judgments. Marker presence alone is not evidence of that match.

[TERMINOLOGY.md](TERMINOLOGY.md) maps the specification's terms to the
textbook and course notes; [PLANNING-COVERAGE.md](PLANNING-COVERAGE.md) maps
the notes' planning concepts to passes and code.

## Executable paper checks

`make -C doc/formal example` runs the four drivers in [example/](example):
the default program and the `computed_key`, `argument_placement` and
`constraint_planning` binaries (each `cargo run --manifest-path
doc/formal/example/Cargo.toml --locked [--bin NAME]`). They exercise
CQ → CQ-NF → VO → RA → IX, optionally inserting VO reductions, stop at IX and
select no physical passes. Each writes typed `.tokens` payloads and plan
summaries under `doc/formal/build/logical-*/` (an optional argument chooses
the directory); the default and `computed_key` programs also write
`validation.txt`.

- The default program checks the YA join tree, exact VO/RA expectations and
  index descriptors for the `R--S--T` query, and an independent whole-tuple
  oracle on fixed, duplicate and empty inputs.
- `computed_key` checks an exact VO schedule and IX for a computed join
  argument, plus a fixed oracle answer.
- `argument_placement` checks that `g()` precedes matching and the `f(x)`
  bind and relational identity follow binding `x`; it executes no generated Rust.
- `constraint_planning` checks automatic and independently authored RA/IX
  contracts and elaboration correspondence, plus an independent oracle on
  eight input presentations; it does not claim that automatic
  planning discovers the authored plan.

The shared driver checks local contracts and order, reduction (when used),
elaboration, and index correspondence. The standalone argument-placement and
constraint-planning drivers check their stated boundaries. None
compares generated-query execution with its oracle. Runtime answers, partial-expression panic-domain cases, Rust
trait guards and expansion fixtures are covered by the crate tests listed in
[validation.tex](validation.tex); realization has no source/target
correspondence `check`.

The manuscript describes source revision `3b21e5d`. Its exact written reference
is desugared CQ-NF with Written/Coarse and `cq_elaborate::compile_written`;
the default `plan` uses optimizing elaboration. Core blocks and
multi-rule union sugar require no surface union extension.
