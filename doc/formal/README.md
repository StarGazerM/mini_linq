# MiniLinq specification

The [paper source](compiler.tex) covers CQ/RA set semantics, implemented
CQ → CQ-NF → VO → RA nanopasses, IX demand construction, and the Physical
and Structures local contracts. Build `build/compiler.pdf` with the command below.

| Subject | Source |
| --- | --- |
| Scope and acceptance | [research_contract.tex](research_contract.tex), [evaluation.tex](evaluation.tex) |
| CQ/RA meaning and host assumptions | [foundations.tex](foundations.tex), [languages.tex](languages.tex) |
| Planning and host dependencies | [planning.tex](planning.tex), [constraint_planning.tex](constraint_planning.tex) |
| IX rules | [ix_physical_contract.tex](ix_physical_contract.tex) |
| Local contracts and pass correspondence | [pass_contracts.tex](pass_contracts.tex) |
| Macro composition | [staging.tex](staging.tex) |
| Rule classification and implementing source markers | [rules.tsv](rules.tsv) |

```sh
make -C doc/formal pdf
make -C doc/formal check
make -C doc/formal example
```

`check.py` checks rule inventory and unique marker presence only. It does not
check semantic correspondence between a rule and code, prove conformance, or
run the examples. Implementation rules describe code that realizes their
premises and conclusions; abstract rules state mathematical laws or
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
supply no physical policy. Each writes typed `.tokens` payloads and plan
summaries under `doc/formal/build/logical-*/` (an optional argument chooses
the directory); the default and `computed_key` programs also write
`validation.txt`.

- The default program checks the YA join tree, exact VO/RA expectations and
  index descriptors for the `R--S--T` query, and an independent whole-tuple
  oracle on fixed, duplicate and empty inputs.
- `computed_key` checks an exact VO schedule and IX for a computed join
  argument, plus a fixed oracle answer.
- `argument_placement` checks that `g()` precedes matching and the `f(x)`
  guard follows binding `x`; it executes no generated Rust.
- `constraint_planning` checks automatic and independently authored RA/IX
  contracts on eight input presentations; it does not claim that automatic
  planning discovers the authored plan.

The drivers check local contracts, reduction correspondence when used and
elaboration correspondence, not order or index correspondence, and none
compares generated-query execution with its oracle. Runtime answers, Rust
trait guards and expansion fixtures are covered by the crate tests listed in
[evaluation.tex](evaluation.tex); realization has no source/target
correspondence `check`.
