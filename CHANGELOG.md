# Changelog

Historical commit entries below are on `codex/all-answers` and passed fmt,
clippy, workspace tests, regressions, dependency isolation, the out-of-tree
gate, the public-API check, the formal check, goldens, fuzz, and TPC-derived
kernels exact against DuckDB at SF 0.1 and SF 1.

## Unreleased

- Add the GPU `WideCanon` catalog provider after `PackedCanon`: complete
  declared key domains select its unbounded-width capability, while recorded
  codec bounds determine the runtime lane count. Stable per-lane radix passes
  deduplicate complete rows without truncation, using the existing publication
  count and host decode protocol. All packed publication consumers now decode
  arbitrary-width, cross-lane fields.

- Cleanup after the Sequence cutover and guard redesign: remove the
  test-only `cq_physical_realization::sources`, the uncalled
  `cq_print::Context::row` (and make `borrowed_pattern` private), unread
  `printer = true` metadata, and the `#[total]` migration diagnostic (a
  generic attribute error remains); share one uniqueness inference
  (`physical::inherited_key`) between Structures and the lowering check;
  dependency gate admits the Sequence adapter dependencies and still rejects
  core→feature edges; API snapshot tool honors `CARGO_TARGET_DIR`. Fix raw
  identifiers (`r#x`, `r#S`) in CQ negation. Datalog: remove the unadopted
  `print-hole` experiment and the unused `cq_rewrites` re-export. Replace
  `cq-ext-negation` (`neg!{..}`) with core `!R(..)` in every workload; the
  extension is deleted, goldens byte-identical.
- Reconcile the GPU TPC-H planner with earned guard position independence:
  host String filtering no longer relies on the removed totality assertion.
- Author H09's Part predicate immediately after its Part atom, before joins:
  the opaque guard's written domain is the predicate-bearing leaf. This fixes
  the query's intended host filtering boundary without restoring `#[total]`
  or permitting compiler motion of arbitrary Rust guards.
  Checked guard-result carriers drop dead columns only after predicate evaluation,
  allowing numeric Part keys to be published without carrying String data.
- Add nonsemantic Sequence evidence advice for row counts, keys, foreign keys,
  selectivities, bounds and sortedness. Evidence is parsed and recorded beside
  analysis obligations; false facts cannot discharge a correctness premise.
- Expose `cq_sequence_plan::Planner` and a checked `RuleBased` implementation.
  Complete Sequence input supports a swappable named `plan = path` facade
  phase; existing CQ-input `plan` remains its join-order phase. Planner advice
  has one round-trippable token format for generated and author-supplied traces.
- Add checked `lookup_first` advice: shared-attribute maximum spanning trees,
  LargestRoot count evidence, ATP chain ties, forward predicate/bridge stores,
  and deferred Expand placement with cycle-closing keys available first.
  Join/semijoin reexpression is independently proved under support; it cannot
  weaken opaque Rust Before domains.
  Support proofs deduplicate identical atoms and search the most constrained
  atom first, rejecting incompatible keys before enumerating unrelated matches.
  A planner-owned contract checker reuses interned support graphs and exact
  query-pair proofs across checked replay and grading. Canonical renaming,
  candidate-domain pruning, and live-binding memoization avoid redundant search;
  exhausted bounded searches reject equivalence rather than hanging.
  Automatic: opt-in pending box 11b. Facade Sequence `plan` defaults to
  order-preserving decomposition; `plan(lookup_first)` explicitly enables
  automatic ordering. All 13 sequence conformance crates use decomposition.
  GPU suite builds include automatic entries only with
  `CQ_GPU_TPCH_AUTOMATIC=1`; the default build is advised only.
  GPU build scripts use the dependency-free `gpu-compress-host` schema crate,
  keeping CUDA kernels out of build dependencies and avoiding embedded PTX
  collisions between host-build and device-runtime artifacts.
- Add adaptive transfer advice over published filtered-store counts. Generated
  alternatives retain their exact joins; static evidence only selects which
  checked alternative runs.
  The opt-in rule-based planner replays Lookup-first, freezes and composes every adaptive
  entry, then decomposes without discarding the ordering trace. Pure streaming
  prefixes are cloned for independent alternatives; publications and Rust
  boundaries remain shared and pinned.
- Pass the advised generic H05 SF10 performance gate: five-run median
  10.208480 ms versus authored R 10.308352 ms, with exact answers. Authored
  order and execution advice are serialized in the suite's replay artifact.
  Suite build reports now record per-query, scale, and mode generation seconds
  alongside checked compilation outcomes.
- Refine equal-work PL-Cost ordering to prefer Lookup before Expand, before
  dependency span and the existing store/dispatch tie-breaks.
- Make guard motion a relational-language property: remove `#[total]`,
  `Filter.total`, `Constraint::Total`, and the Rust method/type-name allowlist.
  Unrecognized Rust guards remain pinned to their written Rust chain and
  preceding relational facts.
- Accept safe CQ negated atoms `!R(args)` with already-bound variables,
  constants and wildcards, binding no names. VO records negation as a relational
  step, not a Rust step or reduction candidate, and elaboration uses the existing
  RA anti-join. Datalog emits negated atoms against completed lower-stratum
  inputs instead of asserted-total Rust membership guards.
  Completed keys apply constants and repeated-variable equalities once per
  stratum, then project distinct variables; rule calls use borrowed host
  membership (or an empty-key witness test) without rebuilding negation indexes.
- Split CPU Sequence corpus conformance into thirteen integration-test crates,
  mirroring classic, retail, and the eleven kernel planner families. Reuse the
  existing configuration lists and share the comparison/reporting loop under
  `tests/support`; preserve all 3,436 configurations and 10,272 comparisons
  while allowing Cargo to compile the partitions in parallel.
- Add `experiments/gpu/tpch`, a single-crate/single-binary planner-driven inventory
  of all eleven CPU CQ-shaped retail TPC-H kernels at SF1 and SF10. Query bodies
  are imported directly from the CPU workload files, not maintained as GPU
  copies. The generic manifest loader resolves dictionary literals, epoch-day
  dates, and scale-2 decimals. Accepted translations have one warmup plus five
  exact-checked measurements and SF1 Sequential-of-the-same-Physical conformance
  through the H05 external-target `cq-testkit` comparison API.
  The observed release build and 13 tests passed, but all 22 query/scale
  compilations were rejected: eleven conformance cases explicitly reported
  `NOT RUN`; no GPU answer agreement or GPU timing is claimed. No compiler gap
  was patched. Full exact errors, terms, source queries and baseline metadata
  are in `experiments/gpu/compare/gpu_tpch.results.json`; the table below omits
  only catalog errors' verbose `syn::Type` debug payload.

  | query | SF1 exact | SF10 exact | GPU total ms SF10 (build/kernels/dedup) | DuckDB t24 SF10 ms | status |
  |---|---|---|---|---|---|
  | h03 | not run | not run | — | 167 | translate rejected: Sequence lookup lacks checked Physical row provenance |
  | h05 | not run | not run | — | 172 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r13`, `[nation_key, supplier_key]`, Range, `DeviceColumns` |
  | h07 | not run | not run | — | 137 | translate rejected: T-Gpu: provider predicate requires a recorded contiguous range selection |
  | h07_block | not run | not run | — | 139 | planner rejected: BE-Boundary: a dispatch must have one unambiguous outer stored space (definition `r3`) |
  | h08 | not run | not run | — | 116 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r11`, `[supplier_key]`, Range, `DeviceColumns` |
  | h09 | not run | not run | — | 4472 | planner rejected: BE-Partial: opaque Physical Rust must be evaluated at host before dispatch |
  | h21_positive | not run | not run | — | 3942 | translate rejected: T-Gpu: provider bound must be a constant or host parameter |
  | d19 | not run | not run | — | 204 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r8`, `[part_key]`, Range, `DeviceColumns` |
  | d27 | not run | not run | — | 79 | translate rejected: Sequence lookup lacks checked Physical row provenance |
  | d72_inner | not run | not run | — | 292 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r9`, `[supplier_key]`, Range, `DeviceColumns` |
  | d85 | not run | not run | — | 191 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r14`, `[part_key]`, Range, `DeviceColumns` |

- Add shared executable Sequence conformance in `cq-testkit`: Sequential and
  Rayon Tasks translations run on identical authored or deterministic inputs,
  comparing answer multiplicities (sets at canonical output boundaries).
  The workload corpus uses the public checked `store(def)` rewrite before tasks
  grading and reports agreements and explicit rejections; migrated tasks spot
  tests retain duplicate and partition-boundary fixtures.
  Hand-authored conformance fixtures include the required host scan type;
  formal rule inventory markers remain unique.
  Unify capability inference across machines: ordinary host row stores retain
  exact buckets; segmented, distributed, and collective reads require ranges.
  The corpus reports execution failures separately from genuine grading skips.
  CUDA H05 compares against default Sequential lowering of the same Physical
  program on identical SF1 data, with an explicit skip when data is absent.
- Make Sequence the shipped CPU target (`sequence` → `sequential` →
  `translate_pull`), including profile counters. Remove the direct Structures
  pull macro and `cq_physical_realization::{compile, compile_profile}`.
  Explicit Structures push terminals remain selectable; shared realization
  and source helpers remain available to `own_push`.
- Add checked GPU Sequence pipeline adapters (`machine_grade` → `translate`)
  and migrate supported resident callers. Isolate the maintained-index GPU
  Datalog bridge, its Structures printer, and blocked-run handwritten baselines
  in `experiments/gpu/legacy-datalog`; unsupported Sequence plans remain
  explicitly rejected rather than substituted.

- Datalog discharges context facts: Storage records canonicality
  certificates per host, Round records `admit R from [holes]` owners and
  per-hole facts (checked independently), fork emits `lower(consumer =
  canonical, host_set .. discharged)` packets with exact `context {..}`
  envelopes. Rule holes build no local `__cq_seen` (DOOP 156 → 0); admission is
  the single set owner. Parallel extension adds an erasable `self_dedup
  per_task` phase realized through the checked push-terminal hole rewrite.
  DOOP: sequential −1.9 to −2.7%, 15 threads −2.5 to −3.5%; exact on all six.

- Physical context facts support `host_set R discharged;` and
  `consumer canonical;` for checked stacked compilers. Lower/select argument
  packets record the assumptions; factful re-entry requires an exact explicit
  context envelope, regenerated from each checked pass's recorded output facts.
  Discharged host sets skip runtime certification, and canonical consumers own
  the returned set boundary without changing internal canonicalization.
  Redundant fact packets preserve already-recorded explicit canonical boundaries.

- `cq-datalog-testkit` executes each Datalog IR's meaning: `eval_incremental`
  (semi-naive round system, the only evaluation scheme) and `eval_round`
  (recorded phases, holes with native partial-Rust bodies). Pass, plug-in and
  parallel Round rewrites are refinement-tested on all 13 oracle programs plus
  targeted cases (instance evidence, not proofs).

- Datalog hole↔CQ contract: typed terminal registry (terminal path →
  `pull`/`push`) checked per hole; push holes stream admission through the new
  `own_push::push_into!` consumer entry (own-dialect). Round rewrites may change
  one hole's CQ sub-pipeline (e.g. `stage(batch(N))`, `represent(positions)`)
  under an independent check. Optional nested-CQ partition terminal measured
  and kept opt-in (no CGU or runtime gain).

- Fx hashing is the Datalog default (Storage and Round providers, hole dedup
  sets). Keyed aHash for adversarial inputs: `hasher(keyed)` +
  `hasher_round(keyed)`; DOOP engines `mini-keyed` / `mini-par-keyed`.
  Exact vs Ascent on all six apps; goldens unchanged.

- Sequence GPU providers reuse setup-time count owners and payload view tables;
  asynchronous pinned uploads replace stream-synchronizing descriptor copies.
  Exact-sized native range/canonicalization scratch persists across calls,
  while retained output publications keep independent metadata. Generated
  setup warms kernel modules before the measured build/query/dedup region;
  recorded layouts, capacities and scheduling decisions are unchanged.
  Providers now consume explicitly published device row and canonical stores
  through owning, zero-copy strided/packed source views, after their recorded
  publication boundary. H05 A/B retain their written reduced-Nation plans.
  Range payloads and fused gather packing support full signed `i64` values
  without truncation. The original i64 triangle now translates and executes
  directly; positive translation/runtime fixtures replace the A/B and triangle
  rejection features. Dense-u32 key directory and packed-u64 output limits
  remain explicit; no schema, provider, or plan is silently substituted.

- DOOP and oracle engines split into feature-selected crates over shared
  harness crates; the parallel CLIs no longer rebuild sequential engines.
  Warm rebuilds: DOOP 30.1 → 15.4 s (sequential), 72.2 → 30.2 s (parallel);
  oracle 8.2 → 3.2 s / 16.3 → 7.5 s. Single engine: mini 13.4 s, mini-par 21.1 s.

- Datalog printer partitions generated code into separately monomorphized
  component/hole functions (hidden facade dispatch helpers), so LLVM uses
  more codegen units. Base DOOP crate rebuild 53.9 → 30.3 s (LLVM 38.8 →
  16.2 s), parallel DOOP 72.9 → 57.4 s; runtime unchanged. Open: nested CQ
  closures still concentrate in one codegen unit (~48% of IR).

- Add the opt-in Physical rewrite `cq_rewrites::retain_streams(r,...)`
  (formal rule EXT-RetainStream): named stream results become retained `let`
  rows, so a back end starts the consumer's parallel domain there. Compiled
  H05 plan R retains Region ⋈ Nation ⋈ Customer (`r4`); its GPU stream now
  starts from every ASIA customer instead of the single ASIA region row
  (SF10 query kernel 143 → 13 ms, total 155 → 25 ms, exact).
- The hand-written H05 reference follows the constant semantics: Region's
  ASIA rows are a selected index built before the query (`region_asia_by_region`),
  and `segments` starts from it; the full Region-by-name index and its in-query
  binary search are removed (SF10 9.60 → 9.70 ms, exact).

- Keep literal atom arguments in normal form and carry their source-column
  bindings on relation occurrences. Build all indexes over the matching rows
  with literal columns dropped, retaining filtered rows for stream sources.
  Occurrences use ordinary `let` storage and generic index/row builds; only
  their row source is constant-specific. GPU streams read the occurrence table
  produced in the build phase, without a per-row literal test in the query kernel.
  Explicit guards and nonliteral argument bindings retain their existing behavior.

- Add explicit `cq_elaborate::simplify_expansions` to LE only, preserving default
  and Hypergraph plans. Eliminate unobservable expansions by checked RA identities:
  under canonical answer semantics, dead introduced columns require only a
  semijoin, and exact named-result lineage discharges already-satisfied support
  checks. Preserve opaque Rust, negative ancestry and block boundaries; no
  printer-side inference or data-dependent key assumption is introduced.

- Add experimental `cq_gpu_datalog::datalog!` using the existing checked
  strata, semi-naive variants and fork, with explicit device admission and
  negative-key policies. Exact resident union/difference publishes immutable
  SCC snapshots and returns persistent device relations. Support copied/literal
  GPU heads and stratified exact negative membership; reject unsupported Rust
  expressions and CPU-policy schedules. Seed one compatible dictionary across
  all rule holes so different string constants cannot acquire conflicting IDs.
  This baseline rebuilds indexes/sets; it does not implement a maintained spine.

- Integrate upstream CQ generators with LE scheduling; reject unsupported GPU
  generators explicitly. GPU cleanup pins RCCL to a published revision,
  removes mandatory local checkout paths, and archives duplicate generated
  evidence. Fix range-read classification, declared host-schema/literal typing,
  stage-size validation, resident answer bounds and packed-column metadata
  ownership. Parallelize membership/codec metadata work and collect timing at
  the completion boundary rather than synchronizing after every operation.

- Refresh GPU design/co-design documentation around the implemented
  Structures→Expr printer and share/witness contracts; remove the superseded
  Device grammar and duplicate proposal prose. Separate historical tuning
  evidence from current status, counts from tuples, and resident execution
  from load/setup. Clarify traversal, expansion and recursion boundaries.

- Add the external `cq_gpu::print` Structures terminal printer and its pooled
  host runtime in `experiments/gpu`: generic CUDA tile streams, exact recorded
  keyed-read provenance, retained lookup witnesses, segmented expansion
  with a bounded explicit frontier, budgeted membership bitmaps and exact multiword
  device dedup. Input batch size does not bound expanded output cardinality.
  The CPU-neutral `cq_rewrites::build_host_reads` records retained
  host modes; explicit `cq_gpu::prefix_runs` and `retype(dedup => DeviceSet<_>)`
  record device representations rather than silently replacing CPU providers.
  Initial integration evidence: nested macro expansion ran on CUDA; smoke
  agreed with pull, and a 20,000-answer fanout exercised frontier/grow/relaunch.
  Compiled retail_h05 A/B matched all 7,242 SF1 and 72,983 SF10 answers after one
  warmup and five runs. Historical SF10 median GPU totals were 19.039/117.689 ms
  versus hand-written 6.2/10.1 ms; these are not current resident measurements
  or like-for-like end-to-end speedups. Both generated kernels then used
  40 registers with 44/36-byte spill stores/loads. Current restrictions,
  measurements and tuple-transfer boundaries are in `experiments/gpu/README.md`.
- Datalog compile time: holes declare only what they read (checked
  independently in Round), one check per pass, linear `dl-source` checks (shared
  query context, DFS reachability), optimized proc macros (`build-override
  opt-level = 3`). Per DOOP engine, proc-macro work 65 s → 3.2 s (debug) /
  11.7 s → 0.7 s (release); base DOOP crate macro expansion 139 s → 3.4 s.

- Datalog Fx hashing plug-in (Storage + Round rewrites), since made the
  default; parallel admission reuses the routing hash. DOOP 1.28× faster
  sequential and 1.12× at 15 threads (geometric means), exact vs Ascent.

- Parallel Datalog slice 2 (extension): sharded parallel admission and
  per-layout exclusive parallel publication; adaptive chunk grain; recorded
  inline threshold. DOOP at 15 threads 1.41–1.63× faster than slice 1, exact
  at 1/5/15 threads. Base gains only generic publication generators.

- Datalog Storage/Round IRs record every decision formerly hard-coded in the
  printer (providers, layout maintenance, phases, publication order,
  termination, observer placement, key scope, output conversion); both
  printers consume the records. Checked alternatives: BTreeMap layouts,
  reversed member publication, later-position direct delta reads.

- Datalog contracts admit any correct choice instead of the default pass's
  exact output; per-IR `check_rewrite` obligations replace identity checks,
  making Source / Strata / Storage rewrite plugins possible. New
  `cq-datalog-testkit` reference evaluator and test-only plugins.

- Datalog compiler restructured like the CQ compiler: languages
  `cq-lang-datalog-{source,strata,incremental,storage,round}`, passes
  (`stratify`, `incremental`, `storage`, `fork`, `print_rounds`, surface
  `if let`), Datalog-owned named pipeline configuration, shared printer
  library and support crate. `stratified!`/`seminaive!` replaced by entering
  pass macros; per-hole CQ sub-pipeline and typed hole interface recorded in
  the Round IR. Parallel moved out to the `features/datalog-parallel`
  workspace (pipeline plug-in; the `parallel;` keyword is removed). Default
  expansion byte-identical; DOOP exact; core unchanged.

- Datalog `parallel;` (Rayon; since moved to `features/datalog-parallel` and the keyword removed):
  rule holes run concurrently; a body-position-0 key-0 scan (delta `Vec` or
  key-0 full layout bucket) is split into contiguous chunks, recorded and
  checked as fork partition metadata. Admission and publication stay
  sequential at the round barrier. Sequential mode binds the original whole
  host (expansion differs only by the chunk binding). DOOP: all 6 apps exact
  vs Ascent at 1–15 threads; best speedups 1.86–3.05× on 5P+10E cores.

- Lookup&Expand sharing on the R/E/C back end (see
  `doc/GPU-LE-BACKEND-CODESIGN.md`). Structures gains
  `r share = lookup b [K] first s force p;` (ST-Share: the keyed read is
  evaluated once at the eager semi and reused by the forcing probe; coupling
  invariant `memo = lookup b k`), a `witness { b [K] at s as Type }` clause on
  `repr` (STR-Witness: α of the witness is the lookup cursor; it is not a
  column, and the contract rejects any program where it would escape into a
  canon key, the head, a retained let, a union/fallback arm, a fused prefix or
  an exists before its forcing probe), and `r stage = batch N expand;`
  (ST-Expand: a stage may span a probe whose build exposes a Copy range cursor
  because the chunk's expansion is a segmented flatten, total and
  length-known; this does not bound its total length by N or remove runtime
  capacity/index-domain obligations). `SortedRun` (Copy range cursor, O(1) `nth`) and the
  `sorted_runs` retype are cherry-picked from `gpu`; `cq_rewrites::retain_lookups`
  records share+witness for eligible eager-semi/later-probe pairs on such
  builds. Pull printer carries the witness as a non-column item pruned at the
  forcing probe and realizes `expand` through `cq_storage::stage::segmented`;
  the push printer refuses both by name. Defaults record nothing.
- Workload corpus adds `le` and `le_runs` planner-axis witnesses and two
  224-configuration triangle kernel targets. A deterministic pairwise extender
  preserves the original 80 handwritten witnesses and case lists; both families
  now require all 367 axis pairs. Of 92 additional pairwise witnesses, 90 are
  accepted; D85 LE/LE-runs merge-on-HashMap requires `OrderedRead` and is pinned
  by two negative UI fixtures, matching the corpus's existing accepted-domain
  policy. TPC-H benchmarking accepts `--config NAME` to measure `default`
  alongside that named configuration, keeping separate uninstrumented timing
  workers, existing DuckDB columns and exact oracle checks.
  Three-job corpus gate: 3,440 passed, zero failed, 127.07 s wall time;
  the compile-fail test includes all eleven rejection fixtures. SF1 five-run
  benchmarks for both named configurations and their default baselines matched
  DuckDB exactly on all thirteen queries (also gated at SF0.1).
- Back end split into three recorded, checked layers (`doc/formal/backend.tex`;
  rules STR-Repr, ST-Total, ST-Stage, BK-Dual). Structures gains
  `r repr = tuples | positions {H lineage {..}, ..} owned [..];` (data
  refinement: positions carry one `RowId` per base occurrence, α fetches
  columns through lineage) and `r stage = batch N;` (evaluation unit: a
  ⊥-free segment from a `for` producer may run N rows ahead; `bind`/opaque
  `guard` end a stage; probe/merge cursors start a new one). Two new default
  passes, `represent` and `stage` (`plan, ix, lower, select, represent,
  stage, print`): defaults record today's decisions only (the built-probe
  lookahead becomes `stage = batch 128`); `represent(positions)` and
  `stage(batch(N))` opt in. Pull/push are one control choice over the same
  stream algebra, not an IR entry. Runtime: `Fetch<C>`,
  `ReadPredicate::test_chunk`, `BlockRead::{column_chunk, read_chunked}`,
  `stage::{for_chunks, Chunk, buffered}` with representation laws.
  `cq-print` now has a column-level environment (`access`, `carried`,
  `bind_columns`, `carrier`, `forward`): operator templates bind only the
  columns they read, so carriers shrink after a column's last reader and
  representation/staging no longer touch every template. The own-dialect
  push printer is ported to it and refuses `positions` explicitly. Join
  transport of positions (index payload = `RowId`) is deferred. Q6 gains a
  `staged` configuration (`positions` + `batch(1024)`).
- Join filters admit `SortedRun` payload builds: both exact payload providers
  enumerate the same distinct keys, so `sorted_runs` keeps existing join
  filters. Generated TPC-H measurement workers use the workload crate's
  recursion allowance so counter/timing wrappers can represent the full
  iterator chain. Sorted-run payload construction counts copied occurrences,
  not set-insertion attempts, matching HashIndex and the independent
  operator-profile checks. All thirteen TPC-H-derived queries register
  `le_runs` (`order(le)` with `sorted_runs`).
- Add `cq_planners::order(le)`: early bound-key lookups in GYO ear order,
  demand-delayed coarse full-row expansion, recursive block scheduling, and
  the default Masks reducer. Independent CK-Le checks occurrence phases,
  written column ownership, Rust Before obligations and normalized CQ
  correspondence. Retail profiling accepts `--config le`.
- Add payload-exact `SortedRun` storage with Copy borrowed lookup cursors,
  stable key clustering and bounded integer rank directories. The external
  `cq_rewrites::sorted_runs` Structures rewrite records eligible one-level
  payload retyping without changing default representation selection.
- Merge upstream measurement, feature lockfile and CI fixes through `7e5c2c6`
  while retaining readable workload cases, pairwise configurations, the triangle
  kernel and UI fixtures. Drop generated-corpus sharding: its two commits add
  no queries, scenarios or non-sharding coverage assertions.
- Datalog integration: moved HW4 and YA acceptance tests into their owning
  package test directories; split std-only `cq-testkit/golden` from DuckDB
  (which still enables it) and switched the Ascent oracle to `golden`; selected
  raw-rustc backend rlibs by the running testkit's linked `TypeId`, with
  zero/ambiguous-match diagnostics and regression coverage. Evaluated the
  proposed D85 bushy-branch fix but left it out: paired SF1 median-of-three
  default runs measured 15.725 ms without and 16.159 ms with it; all 13 queries
  were exact in both runs. Written-order planning never invokes that
  Filtered/Coarse-only branch chooser.
- Measurement: operator profiles drain each build's input before insertion
  and report its exclusive construction time (`excl=`, `source_elapsed`,
  `BUILD_EXCLUSIVE_TOTAL`), comparable to a DuckDB hash join's sink plus
  finalize. The TPC bench adds `duckdb_1_execute_median_ms`: the statement is
  prepared once untimed, then execution and Arrow materialization are timed
  (MiniLinq plans at compile time). DuckDB join-phase numbers in
  `doc/STATUS.md` come from a timer-patched DuckDB 1.5.5 built outside the
  repository; its totals match stock DuckDB within run-to-run noise.
- CI: free runner disk (preinstalled SDKs, docker images), 8 GB swap, no
  debug info in dev/test builds, three build jobs, and removal of isolation
  and feature build directories between steps; recent runs died of
  `No space left on device` or a starved runner.

- Plan advice (rule R-Advice): `plan(reduce(T with S), skip(T with S))`
  records an author's cardinality knowledge as VO reduce steps on top-level
  atoms sharing a variable: `reduce` adds `T ⋉ S` after the Masks reductions,
  `skip` drops a Masks reduction (always sound). Kernels may declare
  `advise plan(..)`, adding an `advised` configuration. D72 is advised
  `skip(Lineitem with PartSupp), reduce(PartSupp with Lineitem)`
  (`ps_availqty > 0` keeps every row): 191–195 → 141–145 ms at SF 1
  (PartSupp index 800k → 124,846 rows; DuckDB 1T 107.7 ms).

- `cq_planners::order(transfer)`: robust predicate transfer (SIGMOD'25) as a
  diverging planner through the public pipeline (rules Plan-Transfer,
  CK-Transfer). On the written order it adds, to the Masks forward pass, the
  stream's reductions by its filtered children and a backward pass from
  filtered sides, parents first; blocks are never extra supports. Every
  retail kernel gains a `transfer` configuration. Against the default it wins
  on D72 (195 → 176 ms; PartSupp index 800k → 125k rows), D85, H08, D19 and
  loses on H03, H05, H07, H07-block, H21+, D27, H09 (175 → 336 ms); see
  `doc/STATUS.md`. Transfers follow join predicates only; scopes with opaque
  Rust keep Masks, since the structural RA check rejects some sound transfer
  schedules there (fuzz seeds 91, 97). `cq_reduce::semijoin` is public for
  external schedules; e2e and fuzz index planners by name/`Planner::ALL`.

- Default plan is left-deep in written order (Written/Coarse): the first
  clause of each scope streams and no pass commutes join operands. Reductions
  (`Reductions::Masks`, rules H-SpanningCore, R-Masks, CK-Masks) reduce each
  indexed clause by its filtered subtrees of the stream-rooted reduction tree,
  children first, before the stream starts, on acyclic and cyclic scopes; the
  stream itself is never reduced or stored. Removed: static build orientation
  (`orient`, L-Orient, CK-Orient), `plan(stream(..))` advice and its `advised`
  kernel configuration, the unused join-filter advice configurations,
  redundant-support absorption in the default chain, and `doc/QUOTATION.md`.
  TPC-derived kernels are written fact-first (H03, H05, H07, H07-block, Q19
  Lineitem; H21+ the other Lineitem; D85 PartSupp; H09, D72 Orders).
- Join filters follow SQL Server's bitmap pushdown: every eager probe or
  semijoin into a statically filtered hash build pushes an exact key set to the
  scan its left spine reaches, through intermediate probes and semijoins, one
  filter per `i64` key column (composite keys filter by each projected column);
  a scan may carry several. A key bound only by an intermediate build is not a
  scan column and is never filtered. Builds over unions qualify. Each covering
  exact operator still runs. At runtime a build whose distinct keys exceed 35%
  of its source occurrence's rows constructs no filter (the RPT+ creation
  threshold); the abandoned filter passes every key.
- `BlockRead::read` shares `read_owned`'s loop; the adapter chain it replaced
  ran twice as slow when pulled row by row (6M rows: 18.9 → 10.7 ms). Dense
  `AdaptiveKeySet` membership inlines into scan loops.
- SF 1 bench (median of five, this machine) vs DuckDB 1 thread: 0.23–1.91×;
  at or below DuckDB on Q19, D19, D27, D85, H05, H07-block, H07, H21+;
  H09 1.03, H03 1.13, H08 1.38, Q06 1.48, D72 1.91. Against the old
  Filtered/orientation default on the same machine: H05 52.8 → 33.7,
  H07-block 106.9 → 35.5, H07 54.4 → 36.5, H21+ 156.2 → 92.5,
  H09 182.3 → 168.8, D85 145.3 → 29.3; D72 141.0 → 205.7 regressed (see
  `doc/STATUS.md`). All answers exact against DuckDB.

- `cq_storage::Filter`: filtering as a representation operation with the law
  `α(s.filter(c, p)) = { o ∈ α(s) | p(c(o)) }`; every read agrees with it and
  filtering twice is filtering by the conjunction. A semijoin is a filter by
  `Membership`. Realizations make filtered tuples unreadable by construction:
  positional hosts filter to a `Selection` bitmap (its only state; every read
  consults it), selections narrow, and `Selection::build` compacts survivors.
  Exhaustive law tests. The `filter_microbench` example (1.5M rows, 2M probes)
  measured the compact rebuild faster than an all-row index whose lookups skip
  cleared ids at every retained fraction, even with that index prebuilt (1%:
  0.5 ms build + 6.5 ms probes vs 124.5 ms probes; 100%: 83.4 + 116.9 vs
  220.8 ms); no masked index type is provided.

- Exact dense key-set construction stages keys in one contiguous buffer, then
  sets bitmap bits directly; it no longer hashes and allocates a shared cell
  per distinct key before discarding that hash table. Sparse and partial-codec
  inputs retain the exact hash/sorted-key fallback. The temporary buffer holds
  input occurrences (including duplicates), and its capacity is accounted.
- Default facade pass `mini_linq::passes::plan` (renamed from `plan_written`,
  no alias): static Filtered/Coarse ordering, checked Hypergraph reduction,
  redundant-support absorption (`cq_elaborate::simplify_reductions`), and
  static build orientation recorded in RA before IX. `plan(no_reduce)` emits no
  semijoins. Tree reduction only on acyclic scopes and only along
  filter-supported edges; exact block cuts on any scope.
- Relational disjunctions derive implied per-occurrence domains, keeping the
  exact predicate; checkers recompute domains. (Guard motion was later made a
  relational-language property; see the guard redesign entry above.)
- Aggregates: redundant dedup layers removed, `SingleGroup` for constant-group
  folds, fast hasher for grouped containers; `AdaptiveKeySet` no longer
  allocates per duplicate key.
- Fixes: reduced prefix-guard RA correspondence, positional predicate reuse in
  replayed unions, correlated-block provenance after reordering, partial
  absorption of shared supports, planner-checker branch erasure.
- Join filters: Structures `join_filter` entries (exact key set or Bloom, with
  the exact probe recorded as covering check) filter streamed scans.
- Bushy plans: checked independent dimension branches in VO, built once.
- `HashIndex` uses hashbrown's Swiss table; `IndexRead::probe` batches 128
  prefetched probes on scan spines; no per-row `Rc`; `HashIndex::keys()`.
- Plan for staged quotation (plan control without touching upper stages) in
  `doc/QUOTATION.md`.
- Formal incremental/Datalog design in `doc/formal/incremental.tex`, with seven
  abstract rules. Distinguishes snapshot refinement from total fixpoint
  equality, candidate dedup from freshness, and representation laws from
  shared maintenance/version contracts. Signed changes remain out of scope.
- SF 1 vs DuckDB 1 thread: 0.96–3.8× (Q19 0.20×), from 2.5–48×; D19 at parity;
  H07 880 → 44 ms, H09 2,170 → 107 ms, H05 269 → 41 ms. D85 regressed to
  61 ms in the integrated tree (open item). Details in `doc/STATUS.md`.

### Datalog (first integration)

- `features/datalog`: Datalog on the unmodified compiler. Checked stratified,
  semi-naive stages resumable from text; core `!R(..)` negation; Datalog-owned
  indexed hosts give 0 core index builds per round. Sequential Ascent
  differential tests and Ascent-generated goldens.
- Workload tests are plain case files with pairwise configuration coverage
  plus a full triangle kernel; 14,357 → 2,982 tests, cold build 874 → 169 s.
- Core changes awaiting approval are listed in `doc/STATUS.md`.

## Execution model (exact semijoin pushdown, hashing)

- `bf5206a` Fast hashing as a recorded type: default Structures types carry an
  ahash `HashState`; printers print exactly the recorded type; `retype` can
  still choose `std`. Compact `HashIndex` layout. SF 1: H09 default
  3,907 → 2,020 ms, D85 default 2,609 → 1,512 ms, D85 hypergraph 131 → 73 ms.
- `8d8f759` Push printer realizes selections, key sets and fused reads.
  Workloads advise `positional` for `Vec`/slice hosts.
- `8bbc0ba` Structures `selection`/`keyset`/`read` entries with lineage checks;
  the selection pass decides, users advise with `select(positional(..))`; pull
  printer fuses filters into scans. Physical unchanged on 180 cases. SF 1 D85
  hypergraph rows copied 863,457 → 53,082.
- `c5bd1dc` `cq-load::ZoneRows`: per-block min/max; block skipping through
  `BlockRead`. Reproduces the measured D85 skips (46% of 2,048-row blocks).
- `5615239` `cq-storage` contracts: `PositionalRows`, `BlockRead`,
  `ReadPredicate`, exact `AdaptiveKeySet` (dense bitmap or hash + sorted keys),
  immutable `Selection`, block/row/copy counters.

## Language and planning

- `3b21e5d` Blocks no longer disable relational hoisting (H07 block: Lineitem
  date filter before its build).
- `241f37c` Disjunctive blocks: `{..} | {..}` (union) and `{..} else {..}`
  (per-row fallback); multi-rule heads are sugar for `|`; `union`/`anti` in
  RA′, IX, Physical.
- `b4740da` Filtered build-side index: `foo("a", b)` builds one index keyed by
  `b` over matching rows, without the constant column.
- `81d1a77` Correspondence checks are structural contracts over fixed
  occurrences (`R@clause`); both homomorphism engines deleted (1,752 lines).

## Documentation

- `7cb9da6`, `c25ffdd` Formal paper rewritten: one section per IR, one per
  pass, every rule tied to code (`doc/formal`).

## GPU negation

- GPU Sequence translates anti membership with the same provider probe as
  positive membership and an inverted keep bit, preserving inactive lanes.
  The retained Physical Anti node supplies polarity (E-Neg); lowering is unchanged.
- The GPU TPC-H suite imports H04, H04-not-exists, H16, and H22 directly from
  the CPU workload sources. Derived input initializers are reused once during
  loading, and advised ordering remains ordinary checked pipeline advice.

## GPU grouped folds

- Add the GPU machine interpretation of BE-Group: recorded canonical valuations,
  radix-ordered keyed owner segments, typed count/sum/min/max reductions, and
  empty-global identities. Nullable extrema use validity columns, not sentinels.
- Add checked grouped translation, synthetic runtime coverage, and an SF1/SF10
  H01 acceptance test that computes its DuckDB DISTINCT-valuation oracle and
  reports the t24 timing comparison in-process.

## Checked automatic Sequence planning

- Reuse invocation-local structural proofs and intern Lookup/Expand support
  graphs, avoiding repeated token rendering and deep analysis copies.
- Preserve live intersection fallbacks, interpret membership compact rows
  without extra iterator nesting, and name distinct provider layouts explicitly.
- Make catalog levels read permissions, separate from host construction;
  retain opaque predicate order and publish required count dependencies.
- Resolve canonicalization catalog permissions at the owning Sequence definition,
  rather than comparing its name with unrelated Physical result names.
- Add BE-Resource: target-independent dispatch demands checked against a
  machine-owned layout and bounded level capacity. GPU grading checks the
  101,376-byte block limit before translation; planner candidates add explicit
  stores between expansions and alternatives when unstaged schedules are illegal.
- Pretty-print generated GPU suite source and scope generated-code warnings to
  its module, avoiding pathological source-map work on multi-megabyte lines.
- Exercise automatic ordering explicitly in the CPU conformance harness.
  The facade's automatic-default release gate remains unchanged until acceptance.
