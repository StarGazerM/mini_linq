# MiniLinq specification

Read the [main paper](build/compiler.pdf) for CQ/RA semantics, planning, IX and
physical compilation. The [shorter paper](build/physical_design.pdf) focuses on
`IX → Physical → Rust[Access]`, provider/rule bindings, administrative callbacks,
and representation obligations. Both include
[ix_physical_contract.tex](ix_physical_contract.tex).

The generic driver and a borrowed exact-index domain are implemented. The old
whole-query hash/configured/Native routes have been retired. The
[implementation domain](../IX-PHYSICAL-REDESIGN.md#implementation-domain) explicitly
lists unported behavior. Block/YA realization, stored intermediates, broader
index construction, and the general push/pull placement are not claimed complete.

The [full-CQ eager report](../ix-physical/query-trace/review.md),
[full-CQ pull report](../ix-physical/pull-trace/review.md) and
[direct-RA report](../ix-physical/realization-trace/review.md) separate
query stages from provider callbacks. Their authored fixtures are checked against
real compiler input/output pairs, with source hashes and the instrumentation
patch retained. The older local two-output and auxiliary-index examples in the
paper remain authored expectations, not captures of the complete query entry.
The pull capture additionally checks the named Pipeline boundary and lazy source
demand; `bind PAT <- (EXPR)` itself chooses neither loops nor adapters.

| Subject | Source |
| --- | --- |
| Scope and acceptance | [research_contract.tex](research_contract.tex), [evaluation.tex](evaluation.tex) |
| CQ/RA meaning and host assumptions | [foundations.tex](foundations.tex), [languages.tex](languages.tex) |
| Planning and host dependencies | [planning.tex](planning.tex), [constraint_planning.tex](constraint_planning.tex) |
| IX/Physical and realization rules | [ix_physical_contract.tex](ix_physical_contract.tex) |
| Representation obligations | [physical.tex](physical.tex), [access_refinement.tex](access_refinement.tex) |
| Current source correspondence | [rules.tsv](rules.tsv) |

```sh
make -C doc/formal pdf
make -C doc/formal check
python3 doc/formal/check.py --implementation
make -C doc/formal example
```

The documentation check validates rule inventory, source locations and authored
fixture assembly. `--implementation` additionally requires all three current-source
captures and runs their actual-boundary comparisons. A stale capture fails and must
be regenerated with `tools/trace_macros.py`; expectations are not regenerated to
make a changed implementation pass.

The five paper executables now check logical derivations and IX, plus one
independent local Access example. They do not emit or execute the removed
whole-query backend. Their [README](example/README.md) explains that scope.
Neither token agreement nor Rust type checking proves arbitrary external
provider laws. Runtime answer and borrowing tests supply separate evidence.
