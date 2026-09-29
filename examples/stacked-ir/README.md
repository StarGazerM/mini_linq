# Stack an independent IR

The reference pipeline's Physical → Γ continuation (selection of the bindings Γ
in the `structures` section) is replaced with
Physical → Trace → Physical → Γ. `cq-lang-trace` owns `trace {}` and
depends only on shared syntax. Two ordinary library passes add and erase it;
`trace-bridge` uses `cq-stage` and `Staged` to compose the token payloads.
Erasure preserves the Physical section exactly. No reference crate is edited
to register or dispatch the new language.

All nine `physical_query` execution tests run through both chains.
`python3 examples/stacked-ir/check_expansions.py` additionally expands all ten
`run` functions and compares the reference and stacked bytes. This command is
part of `tools/test_regressions.sh`.
