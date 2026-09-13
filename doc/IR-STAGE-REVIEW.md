# IR and stage review after compact physical lowering

Reviewed on 12 September 2026 against the implemented compiler, macro frontends,
local contracts, edge checkers, and focused debug/release boundary tests.

The refactor preserves the established compilation order. No stage-order bypass
or new relational operator algebra was found. Physical operator inputs now carry
an existing RA definition and exactly its free binding assumptions. Configuration
is separate, key construction uses Project, and reduction consumes that projected
relation. The remaining compactness issue is principally the older, deliberately
explicit IteratorPipeline grammar; it was reviewed but not redesigned here.

## Complete representation inventory

| Representation and owner | Meaning and information retained | Local boundary check | Next compilation decision |
| --- | --- | --- | --- |
| [CQ Module/Program](../crates/cq-ir/src/cq.rs) | Typed input/result declarations, query head and body; Rust expressions and patterns remain native Syn syntax. | Input declarations, arities, head/body binding, expression dependencies, and the admitted CQ forms. Parsing an extension form does not imply its semantic contract admits it. | Written or planned CQ-to-RA scheduling. |
| [RelationalPlan Module](../crates/cq-ir/src/relational_plan.rs) | Preserved CQ source plus a named DAG of Unit, Rename, NaturalJoin, Project, Semijoin, Select and Extend definitions and an ordered output head. | Defined-before-use operands, headings and types, output consistency and reachability. This local check does not prove equivalence to the retained CQ body. | Select equality-index requirements or the deferred empty overlay. |
| [IndexRequirements Module](../crates/cq-ir/src/index_requirements.rs) | The complete RA module plus a set of named equality-key demands. `@` distinguishes the RA-result namespace from input relations; it is not a storage-capability tag. | Declared namespace, nonempty increasing in-range keys and no duplicates. Coverage of a particular algorithm is an edge-specific obligation. | A basic/lazy, materialized, or composed physical realization. |
| [Physical Context](../crates/cq-ir/src/physical_context.rs) | An existing Project, Semijoin or NaturalJoin definition under row/resource binding assumptions. This is the input to a deferred physical lowering, within the physical realization edge. | Exactly the free operand bindings; distinct ordered attributes; result freshness against operands; row/resource roles; canonical projected-key heading. | The selected operator recipe chooses exact or zone traversal, hash witnesses, or nested loops, and emits Rust with applicable Access/provider holes. |
| [Provider Operation](../crates/cq-ir/src/physical_provider.rs) | A type, expression or statement fragment request: type/input/len/row/scan/rows/select/materialize/blocks/bounds/block_rows. It introduces no RA operators. | Derived syntax owns argument categories. The selected provider rejects unsupported operations; Rust checks expanded types. Parsing alone asserts no block-access or membership law. | Native set or block trait operations chosen by that provider. |
| [RustAccessPlan](../crates/cq-ir/src/rust_access_plan.rs) | Logical occurrence labels paired with actual Rust iteration, guard and output expressions. Physical control has already been selected. | Binding availability and appropriate `for`/`if` shape; this does not prove that an arbitrary Rust expression denotes its logical label. | Pull lowers the region to IteratorPipeline; the continuation chooses iterator or owned-set realization. |
| [IteratorPipeline](../crates/cq-ir/src/iterator_pipeline.rs) | A named linear pipeline with explicit source, joins, filters, projection and final distinctness. | Canonical stream sequence, predecessor/binding consistency, one final projection and distinct result, and returned stream. | Standard Rust iterator adapters, or a HashSet sink that folds terminal distinctness into collection. |
| Syn `File` / `Expr` / `Type` / statements | The actual host program and fragments, including deferred macro holes while expansion is incomplete. This is not a separate runtime query description. | Syn checks syntax; selected edge checkers inspect specified correspondences; Rust checks completed expansion. | Rust compilation and ordinary execution. |

The optimizer's [Predicate/Term](../crates/cq-ir/src/cq/predicate.rs) values are
recognizers for a movable expression subset, not an alternative CQ parser.
`JoinPlanReport`, GYO state, clause dependencies, relation headings, Context
analysis, layouts, resource tables and wiring maps are compiler analysis or proof
objects. They are not additional serialized query stages. In particular, deriving
column maps inside `Analysis` does not reintroduce them into Context syntax.

## Compiler flow and edge obligations

```text
CQ
  -- written/planned scheduling --> RA
  -- base/result/deferred index selection --> IndexRequirements
  -- selected physical realization --> Rust with deferred holes

Within composed realization:
  existing RA definitions + free bindings --> selected operator recipe
  --> Rust[Access/provider operations]

Within each Access region:
  Access --> IteratorPipeline --> Rust iterator or exact set

Provider operations expand wherever their Rust fragment is required.
```

This is a graph of contextual expansions. Provider calls may survive inside
Access or Pipeline leaves and expand later; a provider type can also appear in
an API parameter. A key projection can emit Rust directly without an Access
region. These facts do not establish a new mandatory provider-before-Access stage.

| Edge | Checked relationship and implementation ownership |
| --- | --- |
| CQ → RA | [passes.rs](../crates/cq-compiler/src/passes.rs) validates the source. The planned path checks the supplied derivation against actual definitions through [planned_join.rs](../crates/cq-compiler/src/pass_contracts/planned_join.rs) and the ordered-clause checker. Basic correspondence remains an additional debug postcondition. |
| RA → IndexRequirements | Canonical passes preserve the complete RA and derive their exact stable first-use demands. `compile_deferred_indexes` preserves the same source and writes exactly `indexes {}`. It does not erase an existing overlay. |
| Indexed → basic/lazy/materialized Rust | Each backend checks its admitted domain and coverage and audits the concrete storage/access correspondence. Extra written requirements are real demands, subject to that backend's contract. |
| Empty-overlay indexed → composed Rust | `compile_composed` rejects every nonempty written overlay before realization. [composed/check.rs](../crates/cq-compiler/src/physical/composed/check.rs) reconstructs current versions, headings, projection identity, sharing, binding dominance and configured macro uses for both execution methods. |
| Context → Rust with lower holes | [semijoin.rs](../crates/cq-compiler/src/physical/semijoin.rs) and [joins.rs](../crates/cq-compiler/src/physical/joins.rs) check source roles and derive physical positions, then construct locally checked Access regions. These checks are not general equivalence proofs for arbitrary recipes. |
| Access → Pipeline | The checked public `compile_pull` preserves source order and native patterns/expressions; the existing independent exact-edge predicate is an additional debug postcondition. |
| Pipeline → Rust | Both public endpoints validate the Pipeline before lowering. Their debug postconditions check the specified iterator or set-sink expansion; Rust and execution tests provide separate behavioral evidence. |

The public `compile_physical`, blocked and zone convenience functions compose
the deferred index and composed realization edges. The public `physical_plan!`
macro forwards configured IndexRequirements through relative
`mini_linq::composed!`, so the intermediate macro remains replaceable. The
`workload_query!` compiled convenience mode invokes the typed stages directly;
it preserves their order but does not expose each intermediate macro alias.

## Syntax necessity and the implemented reduction

```text
Context ::= Definition { Binding* }
Binding ::= Atom [=> Path] { Expr };
```

For `K = heading(l) ∩ heading(r)`, the producer emits the following existing RA
definitions under the necessary bindings:

```rust
q = project r keep { k }; {
    r(k, payload) => rows { right_value };
}
t = semijoin l with q; {
    l(k, other) => rows { left_value };
    q(k) { key_resource };
}
```

The row provider decodes the named relation in its written field order. The
provider-free binding denotes the exact projected relation `q`, under the selected
recipe's resource law. It never denotes all of `r`. The producer checks the
construction and use that establish
`l semijoin r = l semijoin project_K(r)`, rather than trusting an origin label.
Fresh projection names avoid all normalized input/result names and previous
projections. Shared resources require the same current right version, separator,
recipe and key collection, with compatible meaning, representation and lifetime.

| Information | Classification and decision |
| --- | --- |
| Existing RA definition and operand identities | Semantic source of the deferred lowering; retained. |
| Ordered attribute names in a physical binding | Necessary to map logical fields to actual row positions; order cannot be inferred from a heading set. |
| Provider path and Rust binding expression | Free representation assumptions unavailable to an independently expanded macro; retained only where needed. |
| Resource heading | Needed by the reduction to derive left key positions; checked against actual projected construction by the whole-source edge. |
| Storage dependency, Access continuation, key implementation, work expression | Administrative configuration, now outside Context and Operation. Lexical wrappers bind it once per distinct selected implementation/configuration in each generated method. Direct configured macro calls still need those administrative arguments. |
| Build/reduce phase suffix, repeated resource origin, unused opposite operand | Removed. Existing Project/Semijoin determines the operation and its free bindings. |
| Key ordinals, output field maps and cached headings | Derived compiler facts, not serialized Context fields. |
| Retained CQ source in RA and logical labels in Access | Deliberate correspondence witnesses. Their denotation cannot generally be recovered from optimized RA or arbitrary Rust expressions. Removing them requires relocating that proof input, not silently weakening checking. |
| Punctuation retained by Syn-derived ASTs | Concrete-syntax preservation, not additional semantic operations. Braces around value expressions also permit Rust macro path capture. |

## Findings resolved in this round

- Semantic Context and provider operations no longer carry service wiring or
  instrumentation. Compiler functions receive typed source and ordinary options
  separately; frontend configuration syntax lives in
  [recipe_input.rs](../crates/mini-linq/src/recipe_input.rs).
- Key construction no longer receives an entire semijoin or an unused left
  binding. Reduction no longer receives an unused original right value/provider.
- Resource provenance is checked against actual Project construction and its
  later Semijoin use. Projection names and configured macro aliases have separate
  freshness rules, including normalized raw identifiers and caller-selected bare
  macro names.
- The final audit removed a shared-emitter dependency from wrapper verification:
  the checker now reconstructs administrative bindings independently. It also
  rejects local macro names that shadow selected implementations or a bare Access
  continuation, rather than relying only on the emitter's fresh-name allocator.
- Public CQ, RA, IndexRequirements, Access and Pipeline entries diagnose malformed
  parsed IR in every build profile. Access/Pipeline functions now return
  `syn::Result`; callers were migrated, including the separate `xtask` crate.
- `run` has an uninstrumented expansion. `run_with_stats` enables instrumentation
  through the same lowering and captures a borrowed counter value; omission no
  longer means allocating counters and discarding their results.
- No compiler-owned manual input parser was introduced. Remaining manual parsing
  and lookahead in `mini-linq` are administrative frontend handling; declared
  semantic syntax remains in `cq-ir`.

## Remaining findings and limits

**P2 — IteratorPipeline contains derivable syntax.** Its local contract fixes
`iterN` names, the predecessor, each complete input binding, scan/join yielded
bindings and the final returned stream
([iterator_pipeline.rs:179](../crates/cq-ir/src/iterator_pipeline.rs#L179),
[226](../crates/cq-ir/src/iterator_pipeline.rs#L226),
[290](../crates/cq-ir/src/iterator_pipeline.rs#L290)). An intentionally linear
grammar could derive these. That requires a coordinated declared-language,
lowering, checker, example and paper migration. Terminal distinctness is also
mandatory in this language: removing its syntax must preserve the set semantics
and the two endpoint implementations. No Pipeline grammar change is hidden in
the physical cleanup.

**P3 — Repeated source analyses remain.** The composed entry, layout analysis and
both method audits independently validate or infer the same source headings.
This is compile-time duplication rather than serialized IR redundancy. A future
checked-analysis object could avoid repeated traversals while keeping the target
audit independent of the emitter's choices. No performance regression is claimed
without measuring it.

**P3 — Provider result categories are checked outside the Rust return type.**
The provider compiler functions accept typed `Operation` but return
`syn::Result<TokenStream>` because operations produce different Rust fragments.
Their tests parse the declared Type/Expr/statement category, and Rust checks the
actual expansion. This is weaker than returning the corresponding Syn AST
category explicitly. It is a target-interface limitation, not an undeclared
query parser or another semantic IR.

**P3 — Some physical API parameters remain broader than a selected route needs.**
`materialize` still accepts a block-size expression and owner/value names because
the block provider needs construction and borrowing control; native row providers
do not need every argument. The query API still accepts block size on native-only
routes and declares the statistics API, including storage `Work`/`Counters` types,
even when a caller uses only `run`. Plain execution omits counter operations, but
API shape and compile-time dependencies are not yet specialized away. These are
remaining configuration/interface questions, distinct from the removed semantic
Context fields. Any further split must preserve independently selected providers
and avoid introducing a universal capability catalogue.

**Admitted-domain limits are explicit.** Composed array providers require `i64`
columns, variable-only output with at most twelve fields, and the five supported
RA operators. These restrictions belong to this physical edge, not all RA or
every future provider. Canonical backends and the optional language extensions
retain their own contracts. Exact membership and conservative rejection remain
different operations; a Bloom filter cannot satisfy the exact membership law.

The reviewed checks are not a proof of the complete generated Rust program.
The composed audit does not validate arbitrary provider expansion, every API
statement or arbitrary host effects. Rust establishes types, available trait
operations and borrowing; it does not prove exact selections, conservative bounds,
stable provenance, metadata-only access or guard-before-open semantics. Those
remain explicit provider/recipe laws backed by the applicable execution tests.

## Validation specific to this review

The five new `public_boundaries` tests, seven existing `pass_contracts` tests and
four `pull_lowering` tests pass in both debug and release: sixteen cases per
profile. The malformed-source tests use `catch_unwind` and require diagnostic
`Err` results rather than panics. The standalone `xtask` checks with the updated
API. The revised formal supplement builds to 21 pages; the unchanged main
manuscript remains 32 pages. These focused results do not replace the root task's
workspace, provider, storage, macro-expansion and formal fingerprint gates.

Consolidated final gates also pass: 398 workspace tests, 37 release integration
tests, workspace Clippy with warnings denied, formatting, all five formal
example drivers, and both manuscript builds (32/21 pages). The regenerated
fact-star composed instantiation, explicitly widened from i32 to i64 for this
backend, matches all 26 golden rows in both execution methods of both selected
routes. Its expanded plain methods contain no counter operations or deferred
operator/provider invocations. These remain bounded checks, not a general
proof of host/provider semantics. Logs: `target/compact-*.log`.
