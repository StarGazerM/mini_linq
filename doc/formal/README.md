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

The [paper drivers](example/README.md) check logical boundaries through IX and
separate oracle fixtures. Runtime answers, Rust trait guards and expansion
fixtures are covered by the crate tests listed in [evaluation.tex](evaluation.tex).
Realization has no source/target correspondence `check`.
