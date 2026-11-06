# Rule conformance and layering audit

Origin: read-only audit of 2026-10-28; living status synchronized against
`7e187597` (2026-11-02). The per-area current tables supersede historical
finding narratives and their old line numbers. `exact` describes conditional
structural conformance, not a fresh test run or a proof of runtime laws.

Current host realization uses recorded direct calls: `build!`, autoref
dispatch tiers and `AutoDense` are retired (see `CHANGELOG.md`, “No fallback
tiers”). Host IF-Capture values use owned clones per consuming move closure,
including nested closures (`passes/cq-print/src/ownership.rs`), not the
historical type-dependent capture handles. GPU-Capture is a distinct
host-expression/codec/domain record, not that retired ownership mechanism.

`doc/formal/check.py` proves only that every `\Rule` has one source marker. This
record pairs each rule's written premises with the code that implements it and
marks the invariant the rule maintains. Per-area tables (rule | tex | code
anchor | premises | code behaviour | status | invariant):

- [front.md](conformance/front.md): CQ → VO → RA → IX → Physical → Structures.
- [sequence.md](conformance/sequence.md): Sequence language, planner, grading.
- [printers.md](conformance/printers.md): pull, Rayon, CUDA translations.
- [datalog.md](conformance/datalog.md): Datalog languages, passes, printers, GPU.
- [drivers.md](conformance/drivers.md): cq-stage, facade registry, GPU TPC-H drivers.

`check.py` verifies that every rule named in these tables exists in
`rules.tsv` and that every cited source file exists, so a renamed rule or
moved file fails the check. `unverified` marks a revised contract/implementation
obligation without claiming that integration checks were exercised.

## Rule coverage gate

`python3 doc/formal/check.py` (or `--coverage` alone; `--coverage --list`
prints the current rows) enumerates decision sites and fails unless each is
covered or listed in [coverage-allowlist.tsv](coverage-allowlist.tsv). The
classification is syntactic and deterministic:

- **Functions.** Every `fn` item with a body (free, method, trait default, nested)
  in `passes/*/src/**/*.rs` and `languages/*/src/**/*.rs`, excluding items under
  `#[test]`/`#[cfg(test)]` and anything inside macro invocations. A function's
  *own body* excludes nested `fn` items. Its key is
  `path::Impl-or-mod::…::name` (`#N` suffix for the N-th repeat in one file).
- **Output IR types.** For `languages/X`, the `pub struct`/`pub enum` names
  declared in X. For a pass, those of its output language (`OUTPUT_IR` in
  `check.py`: order/reduce → VO; relational-lowering/ra-rewrites → RA;
  index-policy → IX; physical-lowering → Physical; structure-selection,
  represent, staging → Structures; sequence-lowering/plan → Sequence) minus
  names the pass declares itself; printers (print, physical-realization,
  sequence-pull) output Rust tokens.
- **(a) Producer decision**: the own body contains, outside `matches!`, a
  struct literal `T {`, tuple-struct call `T(` or variant `E::V` of an output
  type not in pattern position (not followed, after one optional group, by
  `=>`, `|`, `if` or `=`, nor next to `==`/`!=`, nor after `struct enum impl
  for trait -> type :`); or `parse_quote!`/`parse_quote_spanned!`; or
  `.definitions|entries|steps.push(`; or, in a printer, `quote!`/`quote_spanned!`.
- **(b) Checker condition**: the own body contains, outside
  `quote!`/`parse_quote!`/`matches!`, `Err(` not in pattern position,
  `bail!`, `Error::new`/`Error::new_spanned`, or a call of a crate error helper.
  An *error helper* is a function whose whole body is one `Err(..)` expression;
  it is itself no site (it decides no condition).
- **Covered** iff the attached comments (doc/comment lines and attributes
  directly before the item) plus own-body comments contain exactly one
  `Formal rule NAME` marker, NAME resolves in `rules.tsv` (marker text
  `Formal rule NAME`, else rule name NAME) with a row whose path is this file,
  and that rule's `\Rule` premise is mathematical.
- **Mathematical premise.** Split the premise on `\\`, `\quad`, `\qquad`,
  `\and`. It is rejected if empty, or if some conjunct is a bare *prose
  predicate*: only `\mathsf{Name}` with optional `_{..}`, arguments `(..)` and
  `\Downarrow X`, where `\mathsf{Name}` occurs in no `\Rule` conclusion and is
  defined by no displayed definition (`\[..\]`, `$$..$$`, `align`/`equation`/
  `gather`/`displaymath` containing `\mathsf{Name}(..)` followed by
  `\triangleq`, `:=`, `\coloneqq`, `\iff`, `\Leftrightarrow`, `\equiv`,
  `\defeq` or `=`). `\text{..}` phrases are not detected.
- **Markers** naming an unregistered rule fail. A marker outside every
  registered file of its rule is *misplaced*; it never covers a site and must
  be listed as a `misplaced` row `path#NAME` (one row per copy).

The allowlist only shrinks: an unlisted uncovered site or misplaced marker
fails, and so does a listed row that is now covered or gone. A cover is
per function, so a long function with one marker counts as covered.

## Rules whose implementation is not exact

Verified by reading both the rule and the anchor (✓) or reported by the audit
and spot-checked only where marked.

| Rule | Status | Code anchor | Invariant at stake | ✓ |
|---|---|---|---|---|
| PL-Select; PL-Catalog (`backend_ir.tex`) | exact: explicitly stable-partition catalog entries by the recorded token, then select the first entry whose declared facts satisfy the obligations; catalogs are keyed by token id and a program declaration of a catalog token must equal it | `passes/cq-sequence-plan/src/rewrites/catalog.rs`; `passes/cq-sequence-plan/src/catalog.rs` | explicit catalog policy determines representation; no Rust spelling is read | ✓ |
| ST-Entry (`composition.tex`): π = resolve(configuration, defaults) | exact: default `plan_sequential` is a decomposition selected during configuration resolution; facade starts the resulting queue unchanged | `facade/mini-linq-macros/src/lib.rs` (`defaults`, `expand_compile`); `facade/cq-stage/src/lib.rs` (`Configuration::resolve`) | executed queue equals the resolved queue | ✓ |
| BE-Length (`backend_ir.tex:298-300`) | exact (was weaker/stronger): grading consumes checked store Len; Map/Choose propagation and completed publication lengths are derived only by the contract | `languages/cq-lang-sequence/src/contract.rs:26`; `languages/cq-lang-sequence/src/grading.rs:412-418` | every separated write has a pre-write bound | ✓ |
| STR-BuildStats; BE-StatsRead; BE-Choose (`backend.tex`, `backend_ir.tex`) | typed provider statistic declarations and immutable publication references; completed flat Store dominance, matching units/layout/domain/precision, host-only scalar guards and explicit fallback | `languages/cq-lang-structures/src/stats.rs`; `languages/cq-lang-sequence/src/stats.rs`; `languages/cq-lang-sequence/src/contract.rs`; `languages/cq-lang-sequence/src/grading.rs` | no automatic statistic selection, lazy forcing, device readback or estimate-dependent answer semantics | |
| STR-Construction; RT-BuildStats; RT-RunFilter (`backend.tex`) | Structures provider instances record ordinary versus statistic-demand construction, with explicit provider accumulation support and independently checked publication witnesses; printers consume that record. Ordinary Build does not accumulate statistics; demanded lazy initialization is outside the ordinary OnceCell closure. Optional CPU metadata and exact borrowed run views remain | `languages/cq-lang-structures/src/{stats,provider}.rs`; `runtime/cq-storage/src/{sorted_run,radix_run,hash_index,sharded,run_filter}.rs`; `passes/cq-print/src/{lib,access}.rs` | no forced lazy construction, printer-side protocol discovery, unselected accumulation or device/committed-round publication claim; gates reported separately | |
| STR-JoinFilter; SEL-JoinFilter; IF-JoinFilter (`ir_structures.tex`, `passes.tex`) | main-identical projected `Build` and exact key-count activation by default; optional-statistic activation is authored-only; explicit `borrowed_filter` requires `run_filter` and scalar complete layout; explicit `attached_filter` uses `BuildSidecar` / `BuildWithSidecar`; realization flags are mutually exclusive; abandonment comparison is overflow-safe | `languages/cq-lang-structures/src/provider.rs`; `passes/cq-structure-selection/src/positional.rs`; `passes/cq-print/src/lib.rs`; `runtime/cq-storage/src/sidecar.rs` | every covering operator remains; activation-enabled P7.2 rejected by paired measurements; no new default activation; no per-worker rebuild | |
| WF-SequenceEvidence; PL-Evidence (`backend_ir.tex`) | per-fact authored/original-VO provenance; planning keys cannot enter semantic-key consumers; legacy ambiguous key syntax rejected | `languages/cq-lang-sequence/src/evidence.rs`; `passes/cq-sequence-plan/src/rewrites/evidence.rs` | only distinctly authored trusted keys remain semantic premises | |
| CQ-Atom (`ir_cq.tex:68-84`): simultaneous tuple binders, written-order clauses | exact (was weaker than stale argument-order wording): one derived atom binder scope; intentional existing A-Local semantics retained, later-clause variables rejected | `languages/cq-lang-cq/src/lib.rs:596-674` (`CQ-Atom`) | column permutation does not change scope | ✓ |
| WF-VO; WF-VO-Rust (`ir_vo.tex`) | unverified: contract 7 replaces recognition with unchanged Rust and free-variable placement | `languages/cq-lang-vo/src/lib.rs` (`WF-VO-Rust`) | Every Rust expression is pure-total by premise | |
| CQ-Where; CQ-Closed; A-Closed (`ir_cq.tex`, `passes.tex`) | closed atom operands remain relational occurrence restrictions; explicit predicates and atom restrictions preserve query-closed host operands | `languages/cq-lang-cq/src/lib.rs` (`CQ-Where`, `CQ-Closed`); `languages/cq-lang-cq/src/normal.rs` (`A-Closed`); `languages/cq-lang-cq/tests/normal.rs` | Atom restrictions and where supply predicate-derived facts; closed operands never become row Bind dependencies | ✓ |
| BE-RustRepresentation (`backend_ir.tex`) | unverified: machine host-Rust support or identity device read columns and closed captures | `languages/cq-lang-sequence/src/grading.rs` (`BE-RustRepresentation`) | Device Rust is emitted unchanged; codecs use relational comparisons | |
| PL-Cost (`backend_ir.tex:473-478`) | exact (was weaker): costing consumes constructor-preorder `Analysis.partial_groups` | `passes/cq-sequence-plan/src/cost.rs:166,800-829` | cost describes the checked candidate | ✓ |
| BE-KeyPartProvider (`backend_ir.tex:187-190`) | stronger in the rewrite: routing tuple must equal the complete outer key in order | `passes/cq-sequence-plan/src/rewrites/key_partition.rs:821-826` | build and lookup use identical routing | |
| EXT-SeqOutput (`backend_ir.tex:797`) | original weaker finding was rule drift: rule now includes `reserve_previous`; runtime behavior unchanged | `passes/cq-sequence-plan/src/rewrites/mod.rs:281-294` | output strategy is explicit | ✓ |
| STR-Lineage (`ir_structures.tex:48-71`) | exact (was stronger): eager semi preserves left lineage without requiring support lineage | `languages/cq-lang-structures/src/positional.rs:84-130` (`STR-Lineage`) | one base occurrence per position | ✓ |
| R-Placement (`passes.tex:265-267`) | exact: reducer graphs exclude explicitly correlated VO blocks before tree analysis | `passes/cq-reduce/src/lib.rs:75-91` (`R-Placement`) | only independently available relations are reduction operands | ✓ |
| CK-DL-Fork, P-DL-Gpu | stronger: reject execution certificates / observers / phase timing not named in the premises | `features/datalog/passes/fork/src/lib.rs:89-100`; `experiments/gpu/datalog/src/print.rs:151-162` | checked Round subset | |
| O-Leaf, O-Join, L-Retention, BE-Code, BE-Abort | abstract semantic premises; BE-Code identity/admission checks do not prove codec laws | — | join-tree coverage; codec laws; failure never truncates | |
| SQ-ExpansionCounters (`backend_ir.tex`) | unverified: opt-in preclosing child counts and rule output attribution | `experiments/gpu/sequence/core/src/emit.rs`; `experiments/gpu/printer-runtime/src/expansions.rs`; `experiments/gpu/datalog/src/print.rs`; `passes/cq-sequence-pull/src/lib.rs`; `runtime/cq-storage/src/counters/expansions.rs` | no schedule changes; one atomic per block expansion; capacity retries excluded; per-rule fresh outputs may overlap | |
| STR-Canon; BE-CanonSort; BE-CanonOrdered | exact under order laws for focused host coverage: Structures/selection, runtime/load, Pull, Tasks and push barrier regressions passed; final combined gates, GPU acceptance and SF1/benchmarks unverified | `languages/cq-lang-structures/src/lib.rs`; `passes/cq-physical-realization/src/lib.rs`; `runtime/cq-storage/src/dedup.rs`; `features/own-dialect/tests/queries.rs` | Canon is the canonical index build: the default `rows` provider `dedup::Sort` sorts the whole stream; `host` requires a direct immutable load-verified carrier and still uniques complete rows inside prefix runs; `[cols]` is an existence-only complete Built index; CPU DL-InsertNew remains outside scope | |

## Decisions with no governing rule (the missing rules)

Each is a point where a lowering, printer, driver or runtime chooses something
its input IR does not denote. The "missing rule" column is what should be
written into the formal spec and checked, so that the decision becomes
printable and the consumer stops re-deriving it. Entries marked ✓ were
verified by reading the code in this audit.

| Layer | Decision in code | Anchor | Missing rule / invariant | ✓ |
|---|---|---|---|---|
| GPU emitter | **exact (was no-rule):** checked dispatch execution records partition, deferred-range publication, device-wide scan, block-partial combine and required metadata readbacks before resource accounting | `languages/cq-lang-sequence/src/gpu.rs` `derive`; `languages/cq-lang-sequence/src/resources.rs` `dispatch_shape`; `experiments/gpu/printer-runtime/src/lib.rs` `launch_grid_for_kernel`; `experiments/gpu/sequence-runtime/src/offsets.rs` `scan`; `experiments/gpu/sequence/core/src/emit.rs` `partition_rows` | GPU-Launch, GPU-LAUNCH-OCCUPANCY, GPU-OffsetScan and GPU-FoldCompletion: accounting and printers consume one launch fact, never independently recognize deferred ranges or literal grids | ✓ |
| GPU emitter | **exact (was no-rule):** capture dependencies recorded by physical definition; launch domains passed explicitly | `experiments/gpu/sequence/core/src/capture.rs` `Captures::value`; `experiments/gpu/sequence/core/src/rows.rs` `capture_arguments` | GPU-Capture consumes recorded nonempty/enabled domains, without generated-name recovery | ✓ |
| GPU emitter | LogicalTypeId/CodecId contract: logical-id-keyed registry compares recorded `Option<CodecId>`, rejecting conflicting ids even for equal codec spellings | `experiments/gpu/sequence/core/src/runs.rs` `Codecs::record` | GPU-CodecIdentity: compared columns and captures share one checked id; logical and codec types are printing payloads; source/provider logical-id mismatches reject | |
| GPU emitter | **exact:** upstream GPU binding records catalog provider ABI/layout, native permutations and download adapters; independent checker consumes supplied facts | `experiments/gpu/sequence/core/src/bindings.rs`; `languages/cq-lang-sequence/src/gpu/bindings.rs` | GPU-ProviderLayout records provider construction, payload and directory policy before printing | ✓ |
| GPU emitter | **exact (G1/G2):** declared native ABI, contiguous ranges and bounds; explicit Host schemas/codecs; GPU TPC-H SF1 advised/automatic 26/26 and Cassandra 26/26 exact | `languages/cq-lang-structures/src/provider.rs`; `experiments/gpu/sequence/core/src/runs.rs` | GPU-NativeAbi: token facts and explicit input records, never Rust constructor recognition | ✓ |
| GPU emitter | **exact (resolved, P3e):** terminal group aliases, input realization and segmented/grid-partials completion are recorded with source access, transfers, descriptor bounds, element geometry and captures before printing | `experiments/gpu/sequence/core/src/realize.rs` `select`, `check`; `languages/cq-lang-sequence/src/gpu/realization.rs` | GPU-Realization, GPU-TerminalGroup, GPU-GroupGridFold, GPU-SourceAccess, GPU-Transfer, GPU-ProviderDomain | ✓ |
| Rayon printer | **exact (was no-rule):** joins, Group drivers and worker-local lifetimes consume stable term producer identities | `languages/cq-lang-sequence/src/task_execution.rs`; `features/sequence-tasks/src/lib.rs` | BE-TaskProducer: ownership follows checked ancestry, not tokens | ✓ |
| Pull/Rayon printers | **exact (was no-rule):** TileInput/Choice carrier publications are recorded and their symbolic traffic is costed | `languages/cq-lang-sequence/src/task_execution.rs`; `passes/cq-sequence-plan/src/cost.rs` | BE-CarrierPublication extends BE-Publication to explicit carrier storage | ✓ |
| Structures language (resolved: STR-Reader, exact) | tagged Scan/Candidates/Membership roles and `result(Ident)` named consumers remain distinct through parsing, checking, printing and lookup | `languages/cq-lang-structures/src/lib.rs` (`ReaderRef`, `analyze`); `passes/cq-print/src/access.rs` (`fallback_type`) | reader identity never depends on result spelling | ✓ |
| Realization (resolved: STR-Group, SEL-Group, P-GroupState, exact under provider laws) | keyed Group has one concrete Structures accumulator entry; selection records HashMap with the engine HashState, and realization preserves explicit authored replacements and their hashers | `languages/cq-lang-structures/src/lib.rs` (`check_group`); `passes/cq-structure-selection/src/lib.rs` (`group_accumulator`); `passes/cq-physical-realization/src/operators.rs` (`group_with_accumulator`) | every concrete accumulator representation, including its default hasher, is recorded | ✓ |
| Elaboration (resolved: WF-BlockPlacement, O-BlockPlacement, O-WrittenBlocks, RL-Block, CK-BlockPlacement; exact) | VO records required Hoisted/Correlated placement; ordering chooses it and elaboration consumes it unchanged | `languages/cq-lang-vo/src/blocks.rs` (`Placement`); `passes/cq-order/src/lib.rs` (`compile_written`); `passes/cq-relational-lowering/src/block_contract.rs` (`check_placement`) | parsed/printed placement matches RA correlation provenance; written reference rejects hidden placement changes | ✓ |
| Elaboration (resolved: H-Local, O-Restrict, WF-Restrict, CK-Restrict, RL-Restrict, CK-RelationalLoweringRestrict; exact) | predicate targets were chosen in elaboration (`scope_predicates`, `optimize_predicates`); `cq-order` now records `restrict R@i where P;` from H-Local, shared with H-FilterRank and the reducers, and lowering applies only recorded restrictions | `languages/cq-lang-cq/src/hypergraph.rs` (`local_predicates`); `passes/cq-order/src/restrict.rs`; `languages/cq-lang-vo/src/restrict.rs`; `passes/cq-relational-lowering/src/lib.rs` (`check_restrictions`) | predicate placement is a recorded CQ→VO decision; WF checks legality, CK-Order consistency, CK-RelationalLowering presence on the occurrence | ✓ |
| Checkers (resolved: CK-Masks, exact) | recorded Masks edges satisfy independently checked membership/completeness and support-order premises | `passes/cq-reduce/src/correspondence.rs` (`check_masks`) | no calls to `mask_steps`; noncanonical independent order is accepted, invalid premises rejected | ✓ |
| Default planning | shared core `PlanOptions` parser and checked `compile_prefix` composition replace facade-local planning; `prefix_valuation` exposes the checked VO boundary to programmatic callers | `facade/cq-planning/src/lib.rs`; `features/planners/plan/src/lib.rs` (reexports); `facade/mini-linq-macros/src/lib.rs` (`plan`) | one reusable default composition (Plan-Prefix), including explicit order and reduction advice | ✓ |
| GPU TPC-H driver (resolved: DR-Target, exact) | explicit `Pipeline.target` is compiled once; failures propagate without retry or advice deletion | `experiments/gpu/tpch/build_stages.rs` (`compile`); `experiments/gpu/tpch/build.rs` (`compile_stage`) | target transitions are input facts, not error recovery | ✓ |
| GPU TPC-H driver (resolved: DR-Publication, DR-Recipe, exact) | explicit publication placement plus semantic nullable facts determine carriers; serializable recipe/catalog facts select checked rewrites without query labels in compilation | `experiments/gpu/tpch/build.rs` (`publication`, `compile_stage`); `experiments/gpu/tpch/src/advice.rs` (`Recipe`, `Schedule`, `catalog_recipe`) | residency and recipes are advice, not type/profile spelling | ✓ |
| GPU TPC-H invocation (resolved: DR-Registry, Plan-Prefix, exact) | shared Invocation and PlanOptions parsers; one endpoint registry generates public rewrite macros and programmatic dispatch | `experiments/gpu/tpch/src/advice.rs`; `features/rewrites/passes/src/registry.rs`; `features/rewrites/src/lib.rs` | no duplicate grammar or GPU-specific rewrite allowlist; arbitrary suffix aliases are rejected | ✓ |
| cq-stage (resolved: ST-Endpoint, exact) | terminal input language is typed `PrintInput`; macro aliases do not affect resolution | `facade/cq-stage/src/lib.rs` (`ST-Endpoint`) | compatibility derives from declared endpoints and checked `pass<L,M>` boundaries | ✓ |
| Explain (resolved: ST-Observe, DR-Explain, exact) | public facade observations replace a fixed second compiler pipeline; all twelve boundaries include VO, Sequence, decomposition, grading and final Rust | `features/workloads/xtask/src/explain.rs`; `facade/cq-stage/src/lib.rs`; `facade/mini-linq-macros/src/observation.rs` | diagnostics observe the same checked resolved queue; terminal capture is not query execution | ✓ |
| Datalog printers (resolved: DL-LayoutProtocol, exact) | typed Storage/Round protocol records grouped-map entry/default-reset versus bulk extend/push/clear; printers consume it without provider-name tests | `features/datalog/languages/dl-storage/src/lib.rs` (`LayoutProtocol`); `features/datalog/print/src/lib.rs` (`delta_rebuild`) | maintenance protocol is a recorded Storage fact, conditional on provider laws | ✓ |
| Datalog parallel printer (resolved: P-DL-PublicationOrder, exact) | early and phase-barrier publication execute the recorded operation vector; only distinct layouts within one operation overlap, and each operation joins before the next | `features/datalog-parallel/print/src/publication.rs` (`operations`, `layout_phase`) | recorded publication order is executed without owner sorting | ✓ |
| Datalog fork (resolved: DL-PlanningOwner, exact) | named phase resolution records `Hole.plan`; authored order targets it and missing/conflicting ownership is rejected | `features/datalog/passes/fork/src/lib.rs` (`check_planning_owner`); `features/datalog/facade/configuration/src/lib.rs` (`resolve`) | planning-invocation ownership is a recorded index like `Hole.lower` | ✓ |
| GPU Datalog bindings (resolved: DL-DeviceBindings, exact) | `bind_round(binding = N)` records the owner and complete input/arena invocation packets; printer checks and consumes them unchanged, including aliased macro paths | `experiments/gpu/datalog/src/bindings.rs` (`compile_with`, `check`); `experiments/gpu/datalog/src/print.rs` (`compile`) | Round contains the actual invocation before printing | ✓ |
| GPU Datalog codecs (DL-DeviceSymbols) | source `symbols(T => [7] C)` preserves ids; omitted ids in `symbols(T => C)` are interned once across advice. Storage retains named logical/codec_id/codec records; input schemas copy ids and checkers never re-intern | `experiments/gpu/datalog/src/storage.rs` (`Symbols::from_program`); `features/datalog/languages/dl-storage/src/lib.rs` (`DeviceStorage`) | logical codec ids come from declarations, not provider spelling | |
| Codec declaration and admission | Structures interns missing ids once by opaque spelling locally to the declaration context; automatic equal spellings share ids, explicit different ids remain distinct. Sequence obligations/catalog tokens compare `codec_id`, ordering and levels; `None` is identity. Custom GPU tokens use `codec_id: Some(0)` or `None`; no builtin syntactic Dense authorization | `languages/cq-lang-structures/src/provider.rs`; `languages/cq-lang-sequence/src/contract.rs`; `passes/cq-sequence-plan/src/catalog.rs` | Type spelling is used only at interning, then retained solely for printing | |
| GPU Datalog maintenance (DL-DeviceRatio, exact under radix laws) | Storage records ratio advice, default four; every runtime constructor receives it and compaction uses the GPU-RadixPublish integer-division threshold | `experiments/gpu/datalog/src/storage.rs` (`Advice`); `experiments/gpu/datalog-runtime/src/radix_run.rs` (`DeviceRadixRun`, `append_sorted`) | maintenance advice is printable and consumed; no new execution result claimed | ✓ |

## Dominant pattern

1. **Generated names are an unofficial IR.** `__lookup_lookup`, `__cq_domain_`,
   `__cq_enabled_`, `ShardedLayout`, `DeviceLayout`, `translate_pull`,
   `mini_linq::passes::plan` are all matched as strings where a recorded fact
   should be consumed. Each is a place where a rename silently changes
   behaviour.
2. **Re-derivation instead of one recorded fact.** Length (contract vs grading),
   payload/parents (checker vs emitter,
   fixed this session), provider shape (two paths in `rows.rs`), group mode
   (analysis vs cost), default plan (three routes).
3. **Drivers and printers hold decisions the design assigns to IR.** The
   design says translators decide nothing (`backend_ir.tex` §"Refinement and
   the emitter") and representation is recorded (`ir_structures.tex:153-158`);
   the table above lists where that is not yet true. Finishing the Sequence
   design means moving each row into a recorded, checked, printable fact.

## Doc drift corrected in this audit

- `doc/formal/rules.tsv` Plan-Prefix summary said Written/Coarse; now Prefix(Connected)/Coarse.
- `doc/formal/passes.tex` `plan(no_reduce)` "retaining written order" → keeps the default order.
- `doc/SEQUENCE-PLAN.md` [6] grid claim and §11 L&E/RPT/two-phase/opt-in claims (see that file).
- `doc/PLANNING-REDESIGN.md`, `CHANGELOG.md`: unverified DOOP slowdown attribution removed.
