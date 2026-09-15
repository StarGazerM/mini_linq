# IR and compiler boundary review

This review covers the Access-refinement design and the implementation boundaries.
The canonical and configured routes are distinct valid edges:

```text
CQ → RA → IndexRequirements → Rust[Access] → Rust[Pipeline] → Rust
RA ── configured direct physical lowering ──→ Rust[Access]
```

The direct edge ends at `Rust[Access]`. Γ is compiler-local physical information,
not an intervening language. It is required for physical lowering with or without
Yannakakis; logical RA denotation itself needs only a database valuation.

## Complete inventory

| Representation and owner | Meaning | Boundary obligation | Next decision |
| --- | --- | --- | --- |
| [CQ](../crates/cq-ir/src/cq.rs) | Declarations, query, native expressions/patterns | Arity, declarations, binding and admitted source forms | Written or planned scheduling |
| [RA](../crates/cq-ir/src/relational_plan.rs) | Named algebra DAG, preserved CQ source and ordered output | Definition order, headings, output/reachability; equivalence to CQ is a separate edge obligation | Canonical index demands or direct physical lowering |
| [IndexRequirements](../crates/cq-ir/src/index_requirements.rs) | RA plus named equality-key demands | Namespace, ordered in-range nonempty keys, uniqueness; chosen realization must cover required access | Canonical physical implementation |
| Syn `File` with Access holes | Host allocation, lifetime, ordering and local access code | Selected physical edge's correspondence and representation relation | Expand local Access and native operations |
| [Access](../crates/cq-ir/src/rust_access_plan.rs) | Logical occurrence labels plus actual Rust iteration, guard and output expressions | Logical binding/shape checks; native-leaf denotation remains a premise | Pull lowers to Pipeline |
| [Provider Operation](../crates/cq-ir/src/physical_provider.rs) | Typed native type/expression/statement fragments, including Build, Probe and Candidates | Argument syntax, operation-specific checks and generated Rust typing | Selected storage or local Access expansion |
| [Pipeline](../crates/cq-ir/src/iterator_pipeline.rs) | Explicit Scan/Join/Filter/Project/Distinct graph | Stream order, predecessors, bindings, terminal shape | Distinct iterator, fused set collector or support-preserving stream endpoint |
| Syn `Expr`/`Type`/statements and completed Rust | Host program and its fragments | Syntax first; Rust resolution/typing after macro expansion | Ordinary execution under host/provider laws |

Provider operations are native leaves, not a whole-program IR between RA and
Access. Their Build argument is an existing Access plan. Candidates may produce
a nested Access plan expressing blocks, a metadata guard and selected block rows.
That local refinement does not run a later stage backwards or reinterpret RA.

GYO state, administrative histories, expression recognizers, layouts, resource
maps and macro wiring are analysis/proof objects. None requires a serialized
language merely because a Rust struct holds it.

## Syntax necessity

The retained CQ in RA supports source correspondence. Retained RA in
IndexRequirements gives index demands a named source. Access labels connect
binding semantics to native Rust leaves. These are semantic/proof sources, not
redundant execution annotations. They should be evaluated against the checks that
consume them rather than erased by textual duplication alone.

Physical positions are derived from logical headings and current layouts.
Resources use generated Rust locals, without new logical projection names.
Native operations carry their operands only. Storage paths, selected continuations,
collection families and instrumentation live in frontend options and lexical
wrappers.

The Pipeline grammar deliberately repeats names/binding descriptions to expose
and check each step. The stream endpoint has the same set support and preserves
the pre-Distinct sequence. Unique provider traversal therefore stays unique; support equality
alone would permit an arbitrary replacement endpoint to introduce duplicates.

## Checking boundaries

Public CQ, RA, index, Access and Pipeline pass entries check malformed sources
unconditionally and return diagnostics in debug and release builds. Debug-only
assertions may check postconditions, but they are not the source validation API.
The malformed-source regression suite catches unwinding and requires `Err`.

Canonical edge contracts check their emitted targets and declared source
correspondence. The direct checker reconstructs the selected implementation's
expected bindings, layouts, resource versions and ordering from RA and options,
then checks generated code. This is evidence about the supplied template and
wiring; it is not arbitrary Rust verification. Native leaf contracts remain
separate, including exact membership, full key observation, current-mask access,
handle uniqueness until selection and conservative metadata. Selection may copy
native rows into new storage; block selections preserve their payload identity. Macro expansion and Rust typing
reject unavailable operations; they do not prove those laws.

A pruning candidate must cover every exact match in the current left view and
must never enumerate a handle outside that view or twice. The residual exact
membership test completes the proof. Metadata does not receive a false claim of
standalone semijoin denotation. Current RHS versions govern resources; original
immutable block bounds remain conservative for selected subsets.

Checked entry functions live with their transformations: CQ lowering, index
selection, physical realization, Access lowering and Pipeline realization. The
crate root exposes those entries through re-exports. Independent edge relations
are divided under `pass_contracts` by source/target semantics. Shared linear-RA
facts belong to index analysis and contain no storage or emitted-Rust choices.
These are module boundaries within the existing edges, not additional IRs or passes.

## Remaining limits

- The direct array implementation supports a declared subset of RA and `i64`
  rows. Canonical edges retain their own domains.
- Physical choices are explicit paths and overrides, without cost search,
  automatic capability discovery or approximate transfer scheduling.
- The direct query API still shares a block-size parameter and statistics surface;
  these are administrative API debt, not relational semantics.
- Arbitrary Rust effects, resolution/hygiene, lifetimes, custom collectors and
  provider laws are not mechanically proved. Alternate aliases must establish
  the boundary contract they occupy.
- Pipeline syntax can be made smaller only while preserving its useful authored
  program checks; it is outside this change.

[Access refinement](ACCESS-REFINEMENT.md) is the single physical design record;
[compiler structure](COMPILER-STRUCTURE.md) assigns implementation ownership;
[the formal snapshot](formal/snapshot.txt) records consolidated validation.
