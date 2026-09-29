# MiniLinq specification

The [paper source](compiler.tex) covers CQ/RA semantics, planning and IX.
Build `build/compiler.pdf` with the `pdf` command below.
It includes [ix_physical_contract.tex](ix_physical_contract.tex), which gives
the IX construction rules and notes that Physical and the bindings Γ are specified
by the language crates under `languages/` module docs and [DSL.md](../DSL.md) until a formal treatment is
written.

| Subject | Source |
| --- | --- |
| Scope and acceptance | [research_contract.tex](research_contract.tex), [evaluation.tex](evaluation.tex) |
| CQ/RA meaning and host assumptions | [foundations.tex](foundations.tex), [languages.tex](languages.tex) |
| Planning and host dependencies | [planning.tex](planning.tex), [constraint_planning.tex](constraint_planning.tex) |
| IX rules | [ix_physical_contract.tex](ix_physical_contract.tex) |
| Macro composition | [staging.tex](staging.tex) |
| Current source correspondence | [rules.tsv](rules.tsv) |

```sh
make -C doc/formal pdf
make -C doc/formal check
make -C doc/formal example
```

The documentation check validates the rule inventory against `rules.tsv` and
each rule's source location. The paper executables check logical derivations
and IX; their [README](example/README.md) explains that scope. Runtime answer
and borrowing evidence comes from the crate tests listed in
[evaluation.tex](evaluation.tex).
