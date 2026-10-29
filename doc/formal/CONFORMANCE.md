# Rule conformance and layering audit (2026-10-28)

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

## Rules whose implementation is not exact

Verified by reading both the rule and the anchor (✓) or reported by the audit
and spot-checked only where marked.

| Rule | Status | Code anchor | Invariant at stake | ✓ |
|---|---|---|---|---|
| PL-Select (`backend_ir.tex:465-471`) | exact (was weaker): explicitly stable-partition catalog by recorded representation, then select its first eligible token; preserves native keyed ABI | `passes/cq-sequence-plan/src/rewrites/catalog.rs:16,187-219` | explicit catalog policy determines representation | ✓ |
| ST-Entry (`composition.tex`): π = resolve(configuration, defaults) | exact: default `plan_sequential` is a decomposition selected during configuration resolution; facade starts the resulting queue unchanged | `facade/mini-linq-macros/src/lib.rs` (`defaults`, `expand_compile`); `facade/cq-stage/src/lib.rs` (`Configuration::resolve`) | executed queue equals the resolved queue | ✓ |
| BE-Length (`backend_ir.tex:298-300`) | exact (was weaker/stronger): grading consumes checked store Len; Map/Choose propagation and completed publication lengths are derived only by the contract | `languages/cq-lang-sequence/src/contract.rs:26`; `languages/cq-lang-sequence/src/grading.rs:412-418` | every separated write has a pre-write bound | ✓ |
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
| O-Leaf, O-Join, L-Retention, BE-Code, BE-Abort | abstract: no executable checker (documented as premises) | — | join-tree coverage; codec laws; failure never truncates | |

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
| GPU emitter | **exact (was no-rule):** structural logical-type codec registry rejects conflicting native declarations | `experiments/gpu/sequence/core/src/runs.rs` `Codecs::record` | GPU-CodecIdentity: compared columns and captures share one checked encoding | ✓ |
| GPU emitter | **exact (was no-rule):** one provider shape/domain record feeds both construction paths; runtime consumes recorded allocation size | `experiments/gpu/sequence/core/src/rows.rs` `ProviderLayout::check`; `experiments/gpu/sequence-runtime/src/lib.rs` `RangeDescriptor` | GPU-ProviderLayout records key reconstruction, payload, dense policy and directory entries | ✓ |
| GPU emitter | **exact (was no-rule):** complete native catalog constructor identity selects ABI/residency | `experiments/gpu/sequence/core/src/lib.rs` `native_abi` | GPU-NativeAbi: unrelated same-suffix types never imply a native capability | ✓ |
| GPU emitter | terminal `Group` recognized syntactically and rerouted to a device grouped pipeline, skipping an authored store | `experiments/gpu/sequence/core/src/emit.rs:1457-1463,1594-1596` | grouped-execution refinement recorded before translation | |
| Rayon printer | **exact (was no-rule):** joins, Group drivers and worker-local lifetimes consume stable term producer identities | `languages/cq-lang-sequence/src/task_execution.rs`; `features/sequence-tasks/src/lib.rs` | BE-TaskProducer: ownership follows checked ancestry, not tokens | ✓ |
| Pull/Rayon printers | **exact (was no-rule):** TileInput/Choice carrier publications are recorded and their symbolic traffic is costed | `languages/cq-lang-sequence/src/task_execution.rs`; `passes/cq-sequence-plan/src/cost.rs` | BE-CarrierPublication extends BE-Publication to explicit carrier storage | ✓ |
| Sequence planner | **exact (was no-rule):** adaptive consumes printable checked optional-filter identities, never generated prefixes or whole-program default-lowering equality | `languages/cq-lang-sequence/src/lib.rs:83`; `passes/cq-sequence-plan/src/rewrites/adaptive_candidates.rs:84-122` | PL-OptionalFilter: provenance is recorded and preparation proves covering support | ✓ |
| Sequence planner | `lookup_first` reorders joins after lowering; freezes on syntactic Bind/Guard; `transfer` = stored exact semijoin (not L&E SIP, not RPT) | `passes/cq-sequence-plan/src/rewrites/lookup_first.rs:66-69,331-356` | join order is a VO decision; filters are recorded as filters (`doc/SEQUENCE-PLAN.md` §11) | ✓ |
| Structures language (resolved: STR-Reader, exact) | tagged Scan/Candidates/Membership roles and `result(Ident)` named consumers remain distinct through parsing, checking, printing and lookup | `languages/cq-lang-structures/src/lib.rs` (`ReaderRef`, `analyze`); `passes/cq-print/src/access.rs` (`fallback_type`) | reader identity never depends on result spelling | ✓ |
| Realization (resolved: STR-Group, SEL-Group, P-GroupState, exact under provider laws) | keyed Group has one concrete Structures accumulator entry; selection records HashMap with the engine HashState, and realization preserves explicit authored replacements and their hashers | `languages/cq-lang-structures/src/lib.rs` (`check_group`); `passes/cq-structure-selection/src/lib.rs` (`group_accumulator`); `passes/cq-physical-realization/src/operators.rs` (`group_with_accumulator`) | every concrete accumulator representation, including its default hasher, is recorded | ✓ |
| Elaboration (resolved: WF-BlockPlacement, O-BlockPlacement, O-WrittenBlocks, E-BlockPlacement, CK-BlockPlacement; exact) | VO records required Hoisted/Correlated placement; ordering chooses it and both elaboration entries consume it unchanged | `languages/cq-lang-vo/src/blocks.rs` (`Placement`); `passes/cq-order/src/lib.rs` (`compile_written`); `passes/cq-elaborate/src/block_contract.rs` (`check_placement`) | parsed/printed placement matches RA correlation provenance; written reference rejects hidden placement changes | ✓ |
| Checkers (resolved: CK-Masks, CK-OrderBranches, exact) | recorded Masks edges satisfy independently checked membership/completeness and support-order premises; recorded dimension groups satisfy spine, pinned-probe, ear-proof and saturation premises | `passes/cq-reduce/src/correspondence.rs` (`check_masks`); `passes/cq-order/src/branch_check.rs` (`check`) | no calls to `mask_steps` or `branches::choose`; noncanonical independent order is accepted, invalid premises rejected | ✓ |
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
| GPU Datalog codecs (resolved: DL-DeviceSymbols, exact under codec laws) | declared `symbols(T => C)` map is stored explicitly; no `DeviceRuns` generic metadata remains | `experiments/gpu/datalog/src/storage.rs` (`Symbols::from_program`); `features/datalog/languages/dl-storage/src/lib.rs` (`DeviceStorage`) | logical codecs come from declarations, not provider spelling | ✓ |
| GPU Datalog maintenance (resolved: DL-DeviceRatio, exact under run laws) | Storage records ratio advice, default four; every runtime constructor receives it and compaction uses its stored value | `experiments/gpu/datalog/src/storage.rs` (`Advice`); `experiments/gpu/datalog-runtime/src/runs.rs` (`DeviceRuns::new`, `append_sorted`) | maintenance advice is printable and consumed | ✓ |

## Dominant pattern

1. **Generated names are an unofficial IR.** `__lookup_lookup`, `__cq_domain_`,
   `__cq_enabled_`, `ShardedLayout`, `DeviceLayout`, `translate_pull`,
   `mini_linq::passes::plan` are all matched as strings where a recorded fact
   should be consumed. Each is a place where a rename silently changes
   behaviour.
2. **Re-derivation instead of one recorded fact.** Length (contract vs grading),
   opacity (Physical vs lookup_first), payload/parents (checker vs emitter,
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
