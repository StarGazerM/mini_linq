# Actual RA → IX → Physical selection trace

This capture comes from the real `physical_selection` integration test, using
only the `index-demands,physical-selection` features. It contains two procedural
macro expansions plus the separately recorded declarative policy/capture macros.

Read [all procedural steps](steps.md) and [declarative steps](declarative.log).
The raw input/output token files are authoritative. The final continuation is a
capture macro: this test does not invoke a provider or realize/execute the query.
`external::` paths are deliberately unresolved. This validates structural
assignment and preservation, not provider admission or physical semantics.

The fixtures are a complete three-input query module, distinct from the paper's
open two-result region. Both actual boundary payloads are compared against
independently authored fixtures by `crates/mini-linq/tests/physical_selection.rs`.

Reproduce in a fresh output directory:

```sh
python3 tools/trace_macros.py --package mini-linq --test physical_selection \
  --features index-demands,physical-selection --declarative --out /tmp/new-selection-trace
```

`command.json`, `source-hashes.json`, and `instrumentation.patch` record provenance.
The isolated copied workspace and target directory remain outside the repository.
