# Compiler structure and decision ownership

The compiler separates language definitions, checked transformations, and their
composition. Each selectable implementation owns its admitted domain and calls
an independent checker. Defaults choose among these entries in one place;
implementations do not call the default composition.

| Owner | Responsibility |
| --- | --- |
| `cq-ir` | CQ, named RA, index requirements, Access, Pipeline and provider-fragment syntax; local well-formedness |
| `mini-linq` | Parse the selected language and call a typed entry; emit the next macro invocation |
| `composition.rs` | Default CQ, index and physical routes; planned graph/prefix dispatch; complete program composition |
| `analysis/{linear,indexes}.rs` | Shared source facts: linear operands, headings and equality-key coverage; no route choice or generated Rust |
| `cq_to_relational/{canonical,prefix,graph}.rs` | Explicit CQ-to-RA implementations; each rejects inputs outside its own domain |
| `cq_to_relational/{rules,clause_rules}.rs` | Checked administrative transitions and their RA emission |
| `cq_to_relational/{hypergraph,strategy,ya}.rs` | Join-tree analysis, join choice and the YA up/down schedule |
| `cq_to_relational/check/` | Independent canonical, ordered-prefix, prefix-derivation and graph-derivation checks |
| `relational_to_indexes/{input,result}.rs` | Explicit input-column or named-result index selection |
| `relational_to_indexes/check/` | Independent checks for each index-selection edge |
| `physical/{input_indexed,lazy}.rs` | Explicit input-indexed and lazy result-indexed storage/access implementations |
| `physical/materialized.rs` | Checked materialized entries and native Rust host adapters |
| `physical/materialized/query.rs` | Shared indexed set computation, local Access regions and optional counter emission |
| `physical/materialized/semijoin.rs` | Exact semijoin scan and witness guard for the selected hash realization |
| `physical/check/` | Separate input-indexed, lazy and materialized correspondence traversals; shared Rust-syntax inspection |
| `physical/direct/` | Configured RA-to-Access lowering, layouts, versioned resources, wiring and its independent checker |
| `physical/access/{scan,keys,witnesses}.rs` | Candidate scans, exact/zone resources and join witnesses |
| `physical/providers/` | Native-set and block operations through public storage traits |
| `pull.rs`, `pull/check.rs` | Access-to-Pipeline implementation and independent correspondence |
| `pipeline.rs`, `pipeline/check.rs` | Pipeline-to-Rust endpoints and independent correspondence |
| `cq-storage` | Data structures, operation traits, masks, summaries and passive counters; no query execution |

Compiler paths are relative to `crates/cq-compiler/src/`. Crate names refer to
`crates/`. Checkers inspect actual targets without calling the emitter to obtain
an expected answer. Their placement beside the transformation does not remove
that independence. Compiler options and derivation witnesses in
`cq_to_relational/model.rs` are ordinary analysis data, not another IR.

## Explicit edges and default composition

`compile_cq_canonical`, `compile_cq_prefix` and `compile_cq_graph` select actual
implementations. Likewise, `compile_input_indexes` and `compile_result_indexes`
choose an index namespace; `compile_input_indexed`, `compile_lazy_indexed` and
`compile_materialized_with` choose physical realizations. The default entries
retain their convenience behavior in `composition.rs`. An explicit entry checks
its domain rather than switching to another implementation.

Macro continuation paths are relative to the invocation's `mini_linq` binding.
A module alias can therefore change a downstream edge or host. The selected
implementation must still satisfy the source, output, binding and observation
contracts of that position. A resolving macro path alone does not establish them.

For example, this composition selects the graph planner, result indexes and a
callable materialized host independently, using the same boundary names:

```rust
mod wiring {
    pub use ::mini_linq::{
        graph_query as mini_linq,
        result_indexes as relational_plan,
        materialized_query as index_requirements,
        collect_access, collect_pipeline,
    };
}
use wiring as mini_linq;
```

The [composition test](../crates/mini-linq/tests/composition.rs) runs the same CQ
through five such routes over all 256 two-relation databases on its small domain,
including duplicate-input variants. The [typed-boundary test](../crates/cq-compiler/tests/composition.rs)
also verifies explicit rejection of incompatible routes and independent replay
of graph and prefix derivations for the same source.

## Query computation and its host

`compile_materialized_query(&indexed, &access, &inputs)` returns a checked native
Rust expression. The input expressions are evaluated once in declaration order;
each supplies owned tuples through `IntoIterator`. Borrowed inputs can be bound
as `full.iter().cloned()` or `delta.iter().cloned()`. The expression normalizes
inputs, builds required indexes, computes named sets and returns an owned
`HashSet` of answer tuples. It emits no observation counters or generated types.

The same computation supplies the existing observed `Program`/`Storage` API.
`compile_materialized_callable` wraps the checked expression in an anonymous
Rust closure. The `materialized_query!` macro exposes that host using the existing
indexed-language input. Ordinary Rust calls supply its inputs; there is no
additional binding grammar or query IR. Each closure value has one inferred type
per input parameter. Its captures and repeatability follow the generated body's
ordinary Rust rules. The anonymous host introduces no item name that could shadow
functions referenced directly or through opaque query macros. Its wrapper and
query body have independent source-to-target checks.

For example, an existing planned query becomes a callable expression through this
one host binding:

```rust
mod wiring {
    pub use ::mini_linq::*;
    pub use ::mini_linq::materialized_query as materialized;
}
use wiring as mini_linq;
```

The [macro integration test](../crates/mini-linq/tests/reusable_query.rs) compiles
the same CQ in both hosts and checks repeated borrowed full/delta calls. The
[compiler tests](../crates/cq-compiler/tests/reusable_query.rs) additionally check
exact input evaluation, empty/composite semijoin keys and named non-`Copy` values.
The plain host requires no output ordering; sorted standalone materialization
adds its own `Ord` requirement. Reusing this computation does not yet implement
recursive program semantics, and each call still rebuilds this realization's
local sets and indexes.

## Extraction and remaining boundaries

The [YA teaching profile](../teaching/ya/README.md) selects a concrete dependency
closure and provides two separately replaceable algorithm files: YA scheduling
and semijoin-to-Access lowering. Its standalone build checks that unrelated
physical implementations are not required. Assignment boundaries select existing
implementations rather than introducing homework modules into the compiler.

The configured direct physical edge remains distinct. Provider choice is per
input/result, candidate and key choice per semijoin, and witness choice per join.
Its compiler-local Γ emits existing `Rust[Access]`; its providers retain different
operation laws. The reusable materialized host does not claim to abstract those
representations or to provide trie traversal or recursive rule-program semantics.
A new IR requires a new meaning, grammar and lowering obligation that existing
boundaries cannot express.

[Access refinement](ACCESS-REFINEMENT.md) gives the physical rules;
[the boundary review](IR-STAGE-REVIEW.md) inventories the languages and limits.

## Next module refactor: implementation plan

Status: planned after implementation checkpoint `93e3d61`. The sections above
describe the current implementation; the destinations below are proposed.
Update the ownership map and retire this plan as its steps are completed.

The next round keeps the four coarse crates and the existing IR graph. Its goal
is to make a route's implementation, independent checks, tests and examples
follow the same ownership boundary. A file split must remove a dependency or
separate a concrete responsibility; a uniform directory shape is not a goal.
No new compiler registry, serialized IR, storage capability catalogue, query
algorithm or runtime relation API is part of this refactor.

### Problems verified in the checkpoint

- `composition.rs` still reaches private graph/clause planning and replay
  drivers. Family modules should expose checked entries and keep their drivers
  private.
- `physical/check/` owns checks for three implementations through a broad
  parent import scope. Its input-indexed checker also imports domain/coverage
  predicates from `relational_to_indexes/check.rs`.
- `physical/input_indexed.rs` combines storage selection, ownership transfer,
  Access construction and the generated API. `physical/lazy.rs` similarly
  combines storage, a cursor and its host. Materialized query computation is
  already separate, but its checked entries and host assembly share one file.
- CQ, RA, Access and Pipeline files mix grammar with local semantic judgments.
  RA validation also synthesizes headings/types; that single judgment must stay
  together rather than become an artificial extra analysis stage.
- The YA macro template duplicates the main frontend's `stage` and `AccessInput`
  implementations. Test consumers include an oracle owned by
  `stage1_reference`, and large general test helpers mix distinct edge checks.
- Some documentation still assigns defaults to their former implementation
  files. Both paper roots duplicate their common preamble, but all 19 existing
  LaTeX files are reachable from those roots.

### Module destinations and dependency rules

Paths in the first table are relative to `crates/cq-compiler/src/`.

| Current owner | Planned owner | Responsibility retained at that boundary |
| --- | --- | --- |
| `cq_to_relational/canonical.rs`, `check/canonical.rs` | `cq_to_relational/canonical/{mod,check}.rs` | Checked canonical entry and independent canonical correspondence |
| `cq_to_relational/graph.rs`, `rules.rs`, `strategy.rs`, `ya.rs`, `check/graph.rs` | `cq_to_relational/graph/{mod,rules,strategy,ya,check}.rs` | Graph entry/replay, transition kernel, binary choice, YA sweeps and correspondence; keep the state-machine invariants together |
| `cq_to_relational/prefix.rs`, `clause_rules.rs`, `names.rs`, `check/{ordered,prefix}.rs` | `cq_to_relational/prefix/{mod,rules,strategy,names}.rs` and `prefix/check/{mod,clauses}.rs` | Checked prefix entries, C-* transitions, legal-clause choice, freshness and independent source/derivation checks |
| `relational_to_indexes/{input,result}.rs`, `check.rs`, `check/result.rs` | `relational_to_indexes/{input,result}/{mod,check}.rs` | An index implementation and its correspondence share an owner; parent is a facade |
| `physical/input_indexed.rs`, `physical/check/input_indexed.rs` | `physical/input_indexed/{mod,storage,access,host,check}.rs` | Entry, concrete storage/ownership decisions, Access construction, native API and correspondence |
| `physical/lazy.rs`, `physical/check/lazy.rs` | `physical/lazy/{mod,storage,access,host,check}.rs` | Entry, stored renamed inputs/indexes, expression-preserving cursor, native API and correspondence |
| `physical/materialized.rs`, `materialized/{query,semijoin}.rs`, `physical/check/materialized.rs` | `physical/materialized/{mod,host,query,semijoin}.rs` and `materialized/check/{mod,query,host}.rs` | Keep the reusable set computation and exercise intact; separate host assembly and its checks from body correspondence |
| `physical/check/{context,syntax}.rs` | `physical/check_support/{mod,access,rust,indexed}.rs` | Checker-only syntax observations; no implementation dispatch or expected output obtained from an emitter |
| `physical/direct/mod.rs` | `physical/direct/{mod,options,compile}.rs` | Move configuration validation and compilation driver out of the facade; retain existing `api`, `layout`, `resources`, `lower`, `wiring` and `check` owners |

Keep `cq_to_relational/{model,hypergraph,dependencies}.rs` as shared proof/source
facts. Keep `pull.rs`, its checker, and the small Pipeline endpoint implementation
together at their existing edge; do not create a file for every iterator operator.
Keep `physical/access`, `physical/providers` and the storage crate's existing
operation boundaries. They already represent coherent responsibilities.

The dependency direction is:

```text
macro entry / composition -> checked family entry -> private implementation
                                                 -> independent family check
implementation and check -> IR judgments / pure source facts
```

Family facades re-export checked entries. Their child implementations and checks
become private; helpers use the narrowest workable visibility. Composition may
select from source facts and call checked entries, including checked report/replay
entries, but must not manipulate a transition state or enter an unchecked driver.
Existing public function paths remain valid through real owner re-exports.

Move the shared input-domain/coverage predicates out of the index-selection
checker into the pure linear-analysis owner. Preserve their independent
implementation: the check must not validate a generated plan by invoking its
emitter or replacing itself with the emitter's access analysis. Family checks
use explicit helper imports instead of inheriting a mixed parent namespace.
Shared source-domain facts needed by implementations and checks belong in
`analysis/{linear,indexes}`. Implementations must not depend on
`physical/check_support`; that module serves only independent checks.

For physical storage/access/host separation, keep each implementation's lifetime
and ownership decisions local. In particular, input-indexed final-consumer
transfer, lazy first-poll behavior and materialized set execution have different
requirements. Extract functions over existing typed IR and native Rust syntax;
do not introduce a common relation context to force those APIs together.
The proposed small files may be combined when they have no independent invariant.

| IR/frontend source | Planned change |
| --- | --- |
| `crates/cq-ir/src/cq.rs` | Keep derived grammar/types; move local judgments to `cq/contract.rs` |
| `crates/cq-ir/src/relational_plan.rs` | Keep grammar; move `RelationInfo`, `check_in`, output typing and local validity together into `relational_plan/contract.rs`, re-exporting their existing public names |
| `crates/cq-ir/src/rust_access_plan.rs` | Keep grammar; extract `rust_access_plan/contract.rs` for binding and shape judgments |
| `crates/cq-ir/src/iterator_pipeline.rs` | Keep grammar; extract `iterator_pipeline/contract.rs` for stream order, binding and terminal judgments |
| `crates/mini-linq/src/lib.rs` | Extract shared invocation code to `entry.rs`, Access options parsing to `access_input.rs`, and workload handling to `workload.rs`; retain visible proc-macro declarations and continuation paths at the root |

Keep the compact index-requirement and provider-operation IR files intact.
Keep the native and configured physical input parsers with their option
translation. IR judgments may depend on embedded earlier IRs and lexical Rust
analysis; they must never import a compiler implementation. The teaching profile
will copy shared frontend helpers instead of maintaining duplicate implementations.

### Tests, fixtures and examples move with their owners

Test paths below are relative to the named crate. This is a coverage-preserving
migration: retain the independent authored RA/reference verifier separately from
the production planner and generated-Rust checks.

| Existing test owner | Planned owner/action |
| --- | --- |
| `cq-compiler/tests/stage1_reference/{cases.rs,plans/}` | Repository `test-support/relational/{cases.rs,plans/}`; retain the same 25 authored fixtures and expected answers |
| `cq-compiler/tests/stage1_reference/oracle.rs` | Repository `test-support/relational/{semantics,join_trees}.rs`; separate valuation/RA evaluation from exhaustive join-tree enumeration |
| `cq-compiler/tests/stage1_reference.rs` and its verifier/contracts | `tests/relational_reference.rs` and `relational_reference/` |
| `cq-compiler/tests/stage1_production.rs` and administrative checks | `tests/planned_execution.rs` and `planned_execution/derivations.rs`; move physical shape assertions to their physical test owner |
| `cq-compiler/tests/stage1_production/indexes.rs` | Independently runnable `tests/result_indexes.rs` |
| `cq-compiler/tests/pass_contracts.rs` | `tests/edge_contracts.rs`, with edge-owned test modules |
| `cq-compiler/tests/support/harness.rs` | Split into CQ/index edge checks, `support/input_indexed/{api,construction,access}.rs`, and test-only syntax/process helpers |
| `cq-compiler/tests/support/fixtures.rs` | `support/fixtures/{logical,input_indexed,invalid}.rs`; group by contract, not one file per query |
| `cq-compiler/tests/support/compiled.rs` | Retain materialized expansion assumptions separately; share only Rust subprocess plumbing with the input-indexed harness |
| `mini-linq/tests/stage1_shapes.rs`, `stage1.rs` | `tests/planned_queries.rs`, `tests/stage_boundaries.rs` |

The shared `test-support` directory is source support, not a new crate or a
runtime dependency. Hypergraph unit tests and the YA release consume only the
join-tree oracle they need. Update all test imports and the exporter allowlist
in the same commit as these moves. Keep coherent physical/storage, expression,
composition and reusable-host suites intact. Keep intentionally unimplemented
negation/aggregation specifications and their opt-in gate; module cleanup does
not implement or silently discard that extension.

Keep root examples grouped by the behavior they demonstrate. Add a compact
`examples/composition/` example showing one CQ under explicit stage aliases,
using the already-tested routes. Retain `blocked-ya` as the storage/access
example with its `test = true, harness = false` execution gate. Retain the Rust
expression gallery and its separate default/planned entries.

Extract the artifact factory from
`crates/cq-compiler/examples/planned_boundaries.rs` into an example-support module;
both the CLI and production test use that factory instead of including a CLI
file as a test module. Keep all six `doc/planned-example` files generated and
checked. Keep `doc/formal/example` separate: those drivers are executable paper
evidence, not another copy of the boundary snapshots. Update example manifests,
documentation commands and `examples/README.md` together.

### Markdown and LaTeX disposition

| Document | Planned treatment |
| --- | --- |
| `README.md`, `ALL-ANSWERS.md` | Correct default/explicit entry claims and `compile_program` ownership; replace repeated implementation maps with links. Preserve the private-answer release restriction |
| `doc/COMPILER-STRUCTURE.md` | Authoritative module/dependency map; replace this plan with verified ownership as work lands |
| `doc/IR-STAGE-REVIEW.md` | Retain meanings, domains and edge obligations; link detailed host/module explanations to this structure guide |
| `doc/PLANNED-PASS-IMPLEMENTATION.md` | Rename to `doc/PLANNING.md`; retain administrative rules and implementation correspondence, fixing graph versus prefix strategy ownership |
| `doc/STAGE-I-VERIFICATION.md` | Rename to `doc/PLANNING-VERIFICATION.md`; retain reference/production separation and coverage evidence |
| `doc/BLOCK-ACCESS-IMPLEMENTATION.md` | Merge its trait table, concrete storage explanation and reproduction commands into `doc/ACCESS-REFINEMENT.md`; delete the old file after migrating every inbound link |
| `doc/RUST-EXPRESSIONS.md` | Correct clause-scheduling ownership and source links; retain evaluation/binding semantics |
| `doc/DSL.md`, `doc/RUST-EXPRESSION-HELPERS.md` | Retain their distinct grammar and lexical-helper teaching roles; update moved sources |
| Student handouts, course plans, `BRANCHES.md`, `AI-USE.md` | Retain; homework names alone are not evidence of obsolete content. Replace homework wording in reusable compiler comments with semantic descriptions |
| `teaching/ya` guides/templates/profile | Move exercise paths and source closure atomically; preserve separate staff and exported-student instructions |
| `doc/formal/PLANNING-COVERAGE.md`, `doc/formal/example/README.md` | Update source links, renamed test targets and reproduction commands with their owners |
| Root/documentation/example reading maps | Update after final path decisions; examples must describe current implemented versus intentionally unsupported routes |
| `doc/notes/**` | Leave untouched; it is a separate repository |

Retain both manuscripts and their semantic chapters. Extract the 41 identical
leading preamble lines from `compiler.tex` and `physical_design.tex` into
`common.tex`; preserve each title, abstract, bibliography and reading order.
There is no obsolete LaTeX chapter identified for deletion. Update source-owner
claims and remove repeated change-history prose while preserving definitions,
rules, proofs, algorithms and stated limitations. Code file boundaries do not
require one paper fragment per implementation module.

Update `rules.tsv` and `physical_rules.tsv` to actual owners in each affected
commit. Keep the 154 named rules and algorithm/model links unless a separate
semantic change explicitly justifies changing them. Check actual TeX inclusion
closure; if files move into subdirectories, replace the current top-level glob
inventory with traversal of included sources so rules cannot disappear silently.
Refresh source fingerprints only after reviewing and validating the moved code.

### Delivery order and acceptance

Deliver small commits in this order; each move includes its affected imports,
focused tests, teaching profile and documentation links so intermediate commits
remain usable:

1. Establish neutral fixture/oracle and Rust-runner support; remove cross-suite
   ownership, isolate the artifact factory, and rename the corresponding test
   targets with all command references.
2. Extract IR judgments and shared frontend helpers. Confirm unchanged public
   IR paths, token round trips and invocation-scope macro resolution; remove
   duplicated frontend implementations from the YA templates.
3. Group compiler families with their checks; narrow visibility and route
   composition through checked family entries. Move shared domain checks to
   their pure source-fact owner without merging checker and emitter algorithms.
4. Split physical storage, Access and host responsibilities one implementation
   at a time, beginning with input-indexed, then lazy, then the remaining
   materialized/direct facade work. Preserve ownership transfer, callback demand,
   exact set results, native captures and optional observation behavior.
5. Finish example navigation, Markdown consolidation and shared LaTeX setup;
   run the consolidated gates and record the resulting snapshot.

Required gates:

- Inventory unique assertions/fixtures before and after target moves. Preserve
  all 25 authored RA cases, their expected answers, reference checking, compiled
  acceptance and literal macro coverage; test counts alone do not prove this.
- Regenerate all six staged artifacts and require unchanged generated syntax
  for a structural move, then compile and execute the result. Any necessary
  generated-code change gets a separate behavioral review and regression.
- Run source/edge rejection and relevant mutation checks in debug and release;
  retain malformed-proof diagnostics and the direct/opaque macro hygiene tests.
- Run the workspace with `cq-workloads/compiled-queries`, the changed focused
  release gates, all-target Clippy, formatting and whitespace checks. Preserve
  execution of the harness-free examples.
- Re-export YA outside the checkout: foundation builds, each missing algorithm
  fails for its intended reason, reference overlay passes, stubs are restored,
  and all manifest paths/inventory hashes are internal and current.
- Check Markdown paths/anchors and documented target names; run formal
  rule/source checks, the bounded model, paper example drivers and both PDF
  builds without unresolved references, citations or layout warnings.
- Update CI to run the completed-workload and YA extraction gates explicitly.
  Do not use `--all-features` to activate intentionally unfinished extensions.
- Record new validation and fingerprints after the checks pass. Remove stale
  paths, duplicate implementations and compatibility-only forwarding files.

The checkpoint has 407 passing workspace tests, 87 focused release tests and a
verified YA extraction. These are the starting evidence, not a claim that the
planned changes have already passed their gates.
