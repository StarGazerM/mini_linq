# Changelog

Historical commit entries below are on `codex/all-answers` and passed fmt,
clippy, workspace tests, regressions, dependency isolation, the out-of-tree
gate, the public-API check, the formal check, goldens, fuzz, and TPC-derived
kernels exact against DuckDB at SF 0.1 and SF 1.

Entries record successive changes, not a cumulative current API specification.
Later removals supersede earlier additions (including autoref/build-macro
dispatch and capture handles); retain those earlier entries as history. For
current contracts use the living design docs, and for the latest recorded
measurements use [planning progress](doc/PLANNING-REDESIGN.md#progress).

## Unreleased

- P6 C resident FOR-native tile realization: independently checked positional
  scan spines ending in order-insensitive keyless grid folds use existing
  1024-row vectors with 256 threads and four transposed positions per thread.
  FOR/Plain predicates compare code-domain intervals; Delta/native retain
  correct decode. Surviving first `rust_reads` consumers reconstruct fields.
  Every addition stays checked: unflagged sums are exact; incomplete overflow
  flagging can change with traversal order. No S1 format or advice knob;
  unsupported shapes stay unchanged.
  The all-query census changes only h06, h11's Stock total and h15's Revenue
  maximum (both scales); all 52 Physical/Structures/Sequence plans are unchanged.
  All 26 queries pass complete first-result comparison at SF1. The three
  affected queries also pass SF1/SF10 with FOR, Plain64, Delta and native base
  layouts. Core tests: 81; device-column tests: 20. Paired h06 resident total:
  0.771× [0.743, 0.796] SF1, 0.809× [0.808, 0.811] SF10 (CPU-exclusive).
  Review follow-up preserves GPU-Capture gating for empty tile sources and
  registers the Delta/native predicate-time decode violation explicitly.
- GPU TPC-H manifest cold loads use reusable pinned native-width staging;
  exact projected-set verification, row selection, bounds and encoding run
  on the device, streaming one raw column at a time and trimming dedup scratch
  before queries. Verified-key SHA checks, projection reuse and explicit
  HostFor/cache measurements retain their semantics. Reports expose load phases
  and peak/after-load memory (STORAGE §12.7).
  Duplicate projections now release newly loaded full-source cache entries
  before pool trimming, retaining only selected columns and earlier cache
  entries. Query time (load excluded): the 26-query SF1 paired baseline (with
  tiles) shows no systematic cost. Unchanged queries lie within [0.997, 1.007]
  and the geomean is 0.991 [0.989, 0.993]. Controlled SF10 runs show an
  address-sensitive ~2.2% difference in h06's kernel (identical SASS; STORAGE
  §12.7). Load time, a separate cold-start metric never counted as query time:
  h06 process wall is 0.661× (SF1) and 0.113× (SF10); suite process wall goes
  24.3 → 18.3 s.

- GPU base-column storage (STORAGE E1, §§12.5–12.6). `gpu-device-columns`
  adds `BaseLayout` (`For`, `Plain64`, `Native`, `HostFor`, FastLanes `Delta`
  where `detect_integer_codec` picks it) with `EncodedColumn::from_host_layout`,
  plus `from_host_native`, `pack_for` (host FOR packing byte-identical to the
  device encoder), `check_packed_layout`, `from_host_packed` and
  `from_packed_parts`. `Catalog::compress_with` stores the H05
  `tpch_domains` catalog in a chosen layout; `compress` keeps FOR. In
  `cq-gpu-tpch`, `--base-codec for|plain64|native|hostfor|delta` selects the
  layout for both the projection loader and the catalog. Generated code is
  unchanged except H05's two load calls, and the default `for` reads,
  verifies and uploads exactly as before. Results record a per-query
  `base_columns` load report.
  `--base-cache DIR` (with `hostfor`) persists packed columns bound to their
  source file identity and stored-byte SHA-256. Key columns served from them
  are rehashed against current bytes before certifying a projected set.
  Measured (26 queries, SF1/SF10, 5 rotated rounds, all validated):
  - plain is 1.16–1.18× slower than FOR, including H05;
  - size-selected FastLanes DELTA is 1.35× slower (H05 1.38×, over the
    10.35 ms ceiling) for about 10% fewer resident bytes;
  - a tmpfs read control shows no read confound;
  - persisted packed columns cut upload and encode to 0.10× and SF10 process
    wall to 0.913 [0.901, 0.925].

  FOR stays the default. Driver: `experiments/storage-bench/gpu_base_codec.py`.

- VO ordering consumes counts and keys only through the source's checked
  dataset document and explicit bindings. Removed the duplicate inline VO
  evidence grammar/type and `PlanOptions.evidence`; missing facts retain the
  existing default, and explicit orders remain authoritative. TPC-H CPU
  default/par load a checked-in neutral SF1 document; GPU automatic callers
  import their configured dataset manifest and file key catalog rather than
  a Rust count/key table. CPU compilation needs no installed dataset.
  Expected-VO steps
  remain authored independently. Sequence evidence is unchanged: its
  Count/Bounds source-binding migration is separate, and its other facts do
  not duplicate the current file model.
- Sequence considers contiguous partial groups only when Physical records no
  canon on the group's input (`PL-GroupPartial`). Canon-dependent groups keep
  group-key routing and canonicalize inside the routed parts, restoring H02
  parallel grouping without changing uniqueness analysis, input-set semantics,
  or the faster H01/H13/H15/H18 partial schedules.
- GPU workspace pins cuda-oxide `57eacecc0` / RCCL `a776d01c`: the device
  pipeline's scalarization, slot promotion and verification passes no longer do
  quadratic work. `cq-gpu-sequence-runtime` (530 device functions): device
  pipeline about 52 s -> 16 s, crate about 55 s -> 22 s. Under the previous pin
  the new backend emitted byte-identical PTX for the five runtime device crates.
  Under the new pins all 26 GPU TPC-H queries validate at SF1 (no query more
  than 1.6% slower than under the old pin, geometric mean ratio 0.9975, one
  unpaired run each) and DOOP Cassandra is exact
  on all 26 outputs.
- IO is a standalone library: `cq-load` retains its crate name under `io/cq-load`,
  and loader/schema macros move to `io/cq-io-macros` without compiler macro
  reexports. Shared evidence data moves to neutral `runtime/cq-evidence`
  without old evidence aliases; evidence-file acquisition and Cargo tracking
  move from compiler syntax to `cq_load::stats_manifest::load_build_input`.
  Bidirectional dependency checks separate IO
  from top-level compiler languages, passes, and facades, including dev/build
  and target-specific edges. Only the user-facing `mini-linq` facade may depend
  on IO, reexporting the separate library as `mini_linq::cq_load`. Public API and
  external-package gates retain IO coverage independently of compiler ownership.
- Dataset preparation can verify stored keys once and bind them to SHA-256
  content hashes. CPU/GPU TPC-H and DOOP loaders omit complete-row deduplication
  only when the actual loaded projection includes a matching verified key.
  Unprepared, changed, projected-away-key and host-built inputs retain set
  canonicalization. Manifest-path overrides allow read-only dataset adoption;
  no query annotation or compiler uniqueness fact is introduced.
- Physical checking, lowering and canonical rewrites share one operator-based
  uniqueness analysis. Every input relation is a set by the IO contract;
  collected loaders canonicalize complete rows before query execution.
  Compiler host-set certificates and Sequence trusted/planning key annotations
  are removed. Group and output boundary obligations are checked locally.
- Physical lowering alone places valuation canonicalization. Sequence executes
  the recorded canon boundaries; the `group_without_canon` rewrite and its
  separate canon-free uniqueness proof state have been removed. Grouping still
  consumes complete distinct valuations, including under partitioned execution.
- Calibration descriptor version 4 records separately printed base programs in
  `calibration::Stage::bases`: Physical for Structures, and Physical plus
  Structures for Sequence. `Stage::annotation` retains the selected layer's
  language and printed-program identity; continuation agreement also compares
  bases. Flat stages have no bases. Versions 2 and 3 are no longer accepted.
  Structures no longer embeds Physical: its parser receives Physical explicitly,
  its printer emits only annotations, and readers/checkers take separate programs.
  Sequence likewise no longer embeds Structures; lowering, planning, grading,
  and realization receive separate Physical/Structures/Sequence programs.
  Provider-changing rewrites return all changed layers. CQ entry validation
  also checks restricted Physical facts inside realization carriers.
- VO no longer embeds CQ (M3). Parsing and checking use `Annotation<&CQ>`;
  ordering continuations carry CQ and VO separately. Reduction, relational
  lowering, planning, cost export, Datalog planning, and calibration consume
  the explicit normalized CQ. Printed VO loses only its former CQ prefix.
- VO membership is derived from empty step introductions, not a `check`
  suffix (M2f). Connected ranking includes introduction priority; LE builds
  reads from occurrences without inherited phase metadata. Formal tie-breaks
  document producer choices and E-Domain is atomic-conjunct-only (the two Q19
  expected plans now omit the disjunctive container domain). The expected-VO
  harness checks all 69 rule-derived plans against their fixture routes.
- RA and Physical erase occurrence paths (IR redesign M2b). Relational
  lowering returns a `Steps` side table for the VO↔RA seam; its total entry
  accepts a checker-created `Checked` input. Downstream identities use Physical
  definition names. Planning transports its occurrence/read and `Steps` tables
  beside the program; each Smallest invocation resolves only its own selectors.
  No pass inspects the remaining queue. Calibration
  descriptor version 4 records `Steps`, not replayed origins; stream-work facts
  absent from that table remain explicit unknown bounds pending M6.
  The integrated descriptor uses version 4 because both independent branches
  changed version 2 to incompatible version 3 schemas.
- Calibration stage observation moved out of the IR crates (IR redesign M2d):
  `cq_stage::Language` no longer has `calibration_snapshot`; the facade records
  each checked stage as `calibration::Stage { language, program }` (Rust type
  name and printed program, descriptor version 2) and the calibrator
  (`cq_cost::calibration::lineage`) re-parses, checks and derives each stage's
  dependency graph for `Trace::coverage`. No IR crate references calibration.
- Ordering inputs leave the CQ grammar (IR redesign M2e). `cq_lang_cq::ordering`
  and `Program.ordering` are deleted; the `ordering(seed, delta, driver,
  permutation, shared_demand)` record is VO's own (`cq_lang_vo::ordering`,
  checked by VO-OrderingInputs) and reaches the planner as the `PlanOptions`
  `ordering(...)` option, which Plan-Prefix records unchanged in VO. Datalog
  Fork records the inputs in Round `Planning`, and `invocation_pipeline`
  serializes them into the owned planning invocation (DL-OrderingInputs).
  `DriverPolicy` is defined by DL-Incremental. Printed VO loses only the
  embedded CQ header's `ordering(...)` line.
- Ordering is total (IR redesign M2c). `cq_order::order`, `order_unrestricted`,
  `order_written`, `order_advised` and `evidence::order` take a WF-NF witness
  (`Normal`) and resolved advice (`Advice`, from `resolve`, rule
  O-AdviceResolve) and return the VO without an error type; the producer no
  longer checks its own output. The `compile*` functions are the fronts: WF-NF
  check, advice/evidence resolution, then the total ordering. Output
  invariants are checked once by the pipeline: CK-Order, CK-EvidenceLe, and the
  new `cq_order::check_advice` (CK-StepAdvice) on the facade's explicit-order
  route. `Known::place` resolves its negation, read level and bindings first
  (O-KnownResolve). O-BlockPlacement correlates a Rust-free union whose arm
  cannot place from no seed (a nested union needing a domain), which every
  ordering previously refused.

- Explicit order advice now delegates dependency, subatom and phase legality to
  the VO and lookup/expand checkers instead of duplicating producer-side errors.
  Duplicate full occurrence selectors are rejected during resolution; partial
  reads followed by a full read remain legal. The VO subatom checker enforces
  that a partial read introduces at most one variable.

- Rule coverage gate (IR redesign M1): `doc/formal/check.py` classifies every
  function in `passes/*/src` and `languages/*/src` that constructs output IR or
  returns `Err` as a decision site and requires exactly one in-file
  `Formal rule` marker whose `\Rule` premise has no empty or undefined bare
  predicate; today's 488 uncovered sites and 19 misplaced markers form the
  shrink-only `doc/formal/coverage-allowlist.tsv`. CK-Order, CK-Reduce and
  CK-RelationalLowering obligations, CK-Index/CK-Structures, the VO Schedule
  contract (six rules), RAHeading, PHeading/PMode and Entries are restated as
  mathematical rules; IX-Policy, PH-Lower, STR-Represent, STR-Staging and
  P-Realize state the five previously ruleless producers.
- Preserve realizations across I2 drainage: omit implied zero-column checks,
  realize residual root checks as Unit guards, and retain standalone provider
  phases when a moved check exposes a formerly separated candidate pair.
  Recorded crossed-prefix provenance lets a moved check keep its former
  reduction input and ordinary reciprocal absorption without enabling a new
  future-support reducer optimization on unmoved plans.
- GR-Retain now carries retained lookup cursors across checked nonintroducing
  membership probes, including checks on another key/provider before reuse.
  Producer and independent retention contracts require the original keys,
  provider and immutable snapshot; introducing joins, opaque bindings, lost
  keys and publication/canon escapes remain ineligible. Structural regressions
  cover hand-authored witnesses, positions, snapshot occurrence identity and
  rejection cases; execution regressions cover empty inputs, duplicate order
  and one native lookup per input row without scans.
- Planner dataset evidence has a strict versioned JSON schema/checker and a
  separate recorded CQ/Datalog input with explicit relation/column mappings.
  Normalization, delta/layout views and parallel scan erasure preserve that
  input without invoking legacy P3.3e root selection. The optional `cq-cost`
  export projects checked compiler records and source-variant identities for
  external advice; printers do not estimate cardinalities or choose orders.
- `cq-load` adds the offline `cq-stats stats/check` command, sharing bounded
  CSV/Parquet source readers with typed columnar loading. It records exact
  row/distinct/extrema/degree facts (including complete-tuple set semantics)
  and optional full-stream HLL observations. Versioned typed-row content
  hashing binds data order/duplicates, schema, loader options and raw metadata;
  missing/mismatched hash modes are rejected. No runtime planner or implicit
  recursive/target cost facts are introduced.
- `cq-cost advise` emits static, provenance-bearing per-variant advice from
  checked compiler records and exact-rational degree bounds. It records all
  legal candidates, amortizes delta work over declared final rows/rounds,
  accounts for shared read indexes, and retains structural orders for unknown
  or crossing costs. Generated advice uses the same RulePlan parser as macros
  and is checked through the pre-Storage compiler boundary. No default planner
  or runtime guard is introduced.
- Static advice now pins and checks the existing admission owner. Joined
  restricted-view materialization/lifetimes remain explicitly Unknown rather
  than receiving an incomplete whole-run cost; explicit single-read component
  key builders include restriction work and one key-set build.
- Bound-quality review now reports incumbent looseness and only same-prefix/
  key/version observations for proposed steps. A declared 1.05 minimum claimed
  bound-ratio screen retains noise-sized changes before shared-index ownership
  is updated. Aggregate counters never become per-key fan-out certificates;
  unobserved winning prefixes remain unvalidated, not performance evidence.
- Optional `planner-calibration` captures exact canonical input statistics,
  seed/delta/final/round counts and actual Physical operator rows, with checked
  VO lineage, immutable task/host-binding provenance and true row-alias fusion.
  It re-costs only after completion, emits full diagnostic evidence and an
  explicit planning-facts projection, and never adopts a plan at runtime.
  CPU and synchronous CPU-par hooks erase when disabled; relational-progress
  and device counting require separate recorded models/producers.
- Evidence iteration scope is checked against actual Round components:
  coalesced acyclic IDBs may be iterated, seed-only components remain Frozen,
  EDBs cannot claim RecursiveFinal, and a file cannot choose its component ID.

- Structural planner step 1: ready fully-bound checks drain before every
  variable introduction, including lets, for automatic, authored and L&E
  schedules. Executable VOs independently check readiness, earliest checks
  and connectivity; illegal authored/Written orders are rejected. Elimination
  keeps its variable preference only after connected-ready candidates.
  Transfer preserves written introducing-binding boundaries through the shared
  `WrittenConnected` policy. Known-placement advice also obeys earliest-check
  drainage. CPU and GPU H07 authored advice now uses a legal connected chain.
  Datalog records seed/delta, driver and permutation facts and optional
  no-op stratum demand. Incremental ordering and prefix-based Storage remain
  until the separately censused step 1b ownership cutover.
- Expanded-code gates support `--baseline-archive [CACHE]`: a locked,
  revision-keyed archive and build cache without git worktree state changes.
  CQ-to-RA proof references are now occurrence facts and WF binding analysis,
  not unchecked executable VOs.
  Independent RA correspondence additionally recognizes adjacent selection
  conjunctions and a later complete-row witness that repairs projected views
  only through positive, binding-preserving lineage. Semantic and projection-
  loss mutation regressions cover this certificate.

- P7.3: every join filter records one mandatory `policy` in Structures:
  `abandon P of B keys key_count`, `abandon P of B keys ACCESSOR` (authored
  borrowed/attached advice only), or `always`. It replaces the optional
  `abandon … [using ACCESSOR]` field. SEL-FilterPolicy produces it,
  STR-FilterPolicy checks it, and the printer emits it unchanged. The veto-only
  P7.3 cost rule found no departure in the 29 TPC-H kernels or the DOOP rules,
  so default/radix code is unchanged. Census:
  `/data/design-notes/p7-3-design.md`.

- Move the shared `bench-check` library/CLI from `tools/bench-check` to
  `testkit/bench-check`, keeping feature consumers within the core dependency
  roots and including the external-consumer check in tooling gates.

- GPU flat publications record a compact per-publication layout: the
  publication's resident heading (deferred provider payload excluded), unique
  fields, dense stride. The independent checker validates coverage, stride,
  union staging and provider-descriptor reads against the producer record;
  stores, allocation extents, parent copies, union staging, host downloads and
  provider descriptors print the recorded offsets. The shared five-slot
  carrier stride is gone (H03 Lineitem 40 B → 16 B, Orders → 24 B). H03 SF1
  vs `dbe4c2cb`, exclusive CPU: 0.660 [0.659, 0.661] (5 pairs; GPU-lock-only
  10 pairs 0.710 [0.701, 0.721]). Publication records now print (ToTokens)
  and GPU TPC-H plans.json stages carry them as `gpu_publications`.

- P7.2 merged advice-only: Structures records `Construction::Ordinary` or
  `Statistics(BuildDemand)` (rows/max-group), with declared `statistics_build`
  support and independently checked publication witnesses. Ordinary builds do
  not accumulate demanded statistics; statistic builds use `BuildWithStatistics`.
  CPU run/hash statistics, borrowed ordered-run filters and attached sidecars
  remain explicitly authored. Default activation is unchanged; P7.3 is design.

- `tools/remote_gpu` builds execution bundles locally and runs gates/benchmarks
  on disposable RunPod pods, with pinned HF dataset seeding on a persistent
  volume and a 60-minute deletion watchdog. Bundles include Python benchmark
  drivers, the prebuilt DuckDB CLI and HF's `click` dependency; pod execution
  exports `DUCKDB_CLI` and puts bundle binaries on `PATH`. `--no-tests`,
  `--gate-bundles` and `BUNDLE_DIRS` support benchmark-only and interleaved
  multi-bundle sessions. Extraction uses `--no-same-owner`; result retrieval
  streams tar over SSH. Sessions prune older pod directories and remove their
  own bundles after successfully pulling results to avoid filling the volume.
  Benchmark ceiling failures are recorded without skipping exactness checks.

- Timing protocol: builds/tests take the shared timing lock per command, never
  around a multi-command runner. CPU measurements take the gate and exclusive
  timing lock; GPU timing holds only the GPU lock, rotates interleaved samples
  and records load average. Claims whose CI lies within 5% of one require a
  repeat with the CPU exclusive lock acquired after the GPU lock.

- DriftFix6/7 repair ordering readiness, asynchronous empty GPU publications,
  lazy owner lifetimes, recorded radix windows/logical identities and kernel ABI
  launch caches; radix work counters are feature-gated. Rejection tests retain
  specific behavioral coverage rather than diagnostic wording. The workspace
  split test is gated and benchmark reports mark provisional/unconfirmed effects.
  `retail_h07_block` parallel compilation is restored through declared Positions
  capability; external fixtures consume recorded provider tokens/types.

- Recorded 2026-11-02/03 comparisons (not new runs of this tree): CPU TPC-H SF1
  default/par geometric means are 0.908/0.820 versus `c1b75718`. Corrected GPU
  TPC-H SF1 is 0.948 [0.946, 0.949], an exploratory three-pair result; H03
  remains 1.141 with publication-write layout under investigation. Pod DOOP GPU
  xalan/batik ratios 0.239/0.724 are low confidence (up to 10x same-binary
  spread), not a fresh SRDatalog comparison. See planning progress for evidence.

- GPU TPC-H separates final host downloads into `download_ms`, outside build
  and total, restoring the reference benchmark boundary without removing work.
  Checked stage-use facts retain intermediate downloads inside query execution;
  reports expose the executed accounting paths. The 2026-11-02 GPU TPC-H ratios
  are boundary-confounded: current included terminal downloads, reference did not.

- Timing validation uses one shared `tools/bench-check` implementation for CPU
  TPC-H, raw/prepared DOOP, GPU TPC-H and paired pod benchmarks. Each
  binary/configuration's first measured sample is fully compared; later samples
  retain exact per-relation counts and seeded actual-tuple membership checks
  (up to 1,000 tuples, all for smaller outputs). References are indexed outside
  sample loops, validation evidence/timing is explicit, and Python drivers use
  the native checker rather than repeated complete tuple comparisons. Tests and
  explicit correctness checks stay full; query timers and fresh-process pairing
  are unchanged. Mac and storage microbenchmarks retain their existing checks.

- P8 measures Masks against the unconditional Tree reducer on all 29 CPU TPC-H
  SF1 default kernels and the 30-scenario planners corpus. Masks stays default:
  Tree's supported-kernel geometric mean is 1.03541x, versus 1.26056x on the
  supported corpus. H09 and H12 improve substantially; the planning entry
  records exact paired evidence, cyclic/advice exclusions, intermediate-size
  counters, and an unimplemented count/filter-rank selection proposal whose
  census selects those two strong winners. No compiler, checker or printer
  behavior changes.

- Tasks key partitioning preserves native positional Selection publications:
  automatic advice retains their selected carrier, and Sequence rejects a
  partitioned store pretending to publish that carrier. Sharded providers now
  implement their declared flat `Build` capability as a singleton cover,
  preserving exact reads without rerouting or copying input rows. The KP-1/KP-2
  regression executes exact answers with one and four workers.
  P8's separate routing-projection census closes at a measured stop: 66 exchanges,
  782 MB of carriers and no droppable columns at SF1; no projection is added.

- P5's authored CPU blocked-scan checkpoint connects Plain/Bytes ScanTables to
  checked Structures and printable Sequence execution facts. Pull and Tasks
  share bounded masks, source-qualified positions and demanded-only reads;
  canonical ordinal keys are gathered once without semantic reconstruction.
  Q06 alone adds explicit blocked Plain/Bytes serial/parallel routes, leaving
  defaults unchanged. Join/index consumers, encoded-base grouping, restricted
  source renames and undischarged host-set verification remain checked
  unsupported boundaries of this initial checkpoint, not decoded-row fallbacks.
  The P5 planning entry records the rollout boundary and measurement evidence.
  The completed SF1 Q06 comparison retains this as opt-in: Bytes/row is
  1.112 [1.097, 1.130] serial (slower) and 0.766 [0.719, 0.843] with 24 Tasks
  workers. Five paired processes per comparison, exact answers in all 2,520
  samples and 600 warmups, page faults and same-source linker controls are
  recorded in `experiments/storage-bench/results/p5-s3-q06.json`. Stop the
  rollout at Q06; no default promotion or other-kernel authorship.

- Non-GPU rejection tests assert failure and applicable formal rule IDs rather
  than diagnostic prose. Behavioral and compile-fail coverage remains independent
  of error-message wording; GPU tests are unchanged.

- Consolidate 28 Cargo workspaces into three: root, `experiments/gpu` and
  `experiments/mac`. Portable compiler, feature consumers, formal examples and
  CPU benchmarks share the root lockfile and target directory. Keep GPU's oxide
  toolchain and Metal's platform dependencies in separate workspaces; preserve
  standalone external-packaging fixtures as isolation test inputs. Regression
  rows select explicit packages and retain formerly grouped consumer tests.
  Closed-core dependency checks remain independent of workspace membership.
  Preserve package versions/MSRVs, workload debug settings and benchmark LTO
  profiles; relocate external consumers using materialized public dependencies.
  Relocated consumer dependencies retain their directory group's sibling datasets.
  Root default members exclude the five packages whose dev-dependencies enable
  counters; explicit package regression gates retain their instrumented coverage.

- GPU Sequence placement keeps descriptor-backed `abi columns` index builds at
  their host publication boundary. It no longer copies whole resident columns
  through unpartitioned device publications merely because the index token
  differs from the scan token. Row-oriented native provider rebuilds still
  publish device rows; the choice is recorded in Sequence grades and checked
  through ordinary catalog/grading obligations. The GPU batch gate now runs
  advised H05 SF10 and enforces its unchanged 10.35 ms benchmark ceiling.

- P4-GPU slice E records complete-codec active-bit sort windows and admitted
  input certificates in device Storage. Full/delta consumers share one sorted
  batch per recorded physical order; canonical-order input is adopted without
  sorting. Independent checks reject stale permutations, omitted significant
  bits and false sortedness claims. Full-key sets use explicit implicit-unit
  offsets and no item-source map; occurrences retain materialized group offsets.
  Opt-in phase work counters distinguish canonical/layout radix work, modeled
  traffic, merges, group readbacks and metadata materialization. Directory and
  provider defaults remain conservative; performance evidence is recorded in
  the P4-GPU plan entry.
  Five exact pairs versus `1b1540bc` retain the combined change: fixpoint ratios
  xalan 0.96599, batik 0.95177, Cassandra 0.96163; each 95% CI is below one
  and below its same-source linker-shuffle control CI. Planned radix passes
  drop 15–16%; sort/merge counts stay unchanged and unit-offset allocations
  disappear. GPU/CPU/API gates and all 104 advised/automatic TPC-H exactness
  cases pass. The separate H05 performance ceiling fails on pristine baseline
  too (951.291 ms versus current 952.262 ms). Full evidence:
  `experiments/gpu/compare/p4-gpu-e.results.json`.

- P4-GPU slice D cuts Datalog EDB/full/delta/negative layouts over to ordinary
  complete-key `RadixLayout<_, (), W, u64, ()>` sets backed by
  `DeviceRadixRun<W, u64, ()>`, with `radix_relation` /
  `NativeAbi::RadixRelation` and no old `DeviceRuns`/`DeviceLayout` aliases.
  Per-layout records include offset, full-key-set construction, key-prefix
  reads, legal configurable radix bits (default zero), ratio and launch
  geometry. Independent checks validate those facts, schemas, initializers
  and source erasure without replaying realization. Read-only `fresh`
  returns a `SortedBatch`; all secondary permutations use that same admitted
  batch, delta moves and frozen negative deduplication are preserved, and
  maintained views use `republish(&state)`. Prefix lookup compares encoded
  intervals and decodes only demanded complete-key fields. Lane scheduling
  and per-hole reservation are unchanged. Checked host-derived Word witnesses
  preserve function-local type aliases without leaking them into CUDA modules.
  GPU runtime/core/facade suites pass; xalan/batik match 43/43 duplicate-free
  tuple sets, Cassandra 26/26. Root, quick, formal and API gates pass; all 46
  CPU expansions are identical to `b7e83c6f`. Five exact interleaved pairs give
  current/baseline fixpoint ratios 1.01059 [0.99901,1.02241] for xalan,
  0.98145 [0.97875,0.98556] for batik, and 0.49983 [0.48944,0.51058] for
  matched-workflow Cassandra. Xalan is neutral/slightly slower; sort/unique
  work did not shrink, and unit-offset maintenance increased. Nsight kernel/API
  attribution and per-command gate times are in
  `experiments/gpu/compare/p4-gpu-d.results.json`; slice E targets measured
  sorting and metadata materialization costs.

- P6b GPU host preparation: `Realization.launches` records invocation
  occupancy or a program-owned one-block/resident launch envelope.
  Program slots use the CUDA module's opaque compiled entry identity,
  including callable specialization, and retain validated function/configuration
  with the context and query scope. Generated typed scalar-ABI wrappers borrow
  those preparations; runtime checks only context and grid-envelope bounds,
  without repeated occupancy, function/device-attribute or module lookup calls.
  Row-dependent grid caps, root sharing, count/failure readbacks, replay and
  stream synchronization are unchanged. `Execution.grid` is removed rather
  than duplicating the target realization's launch decision.

- GPU-7/8/9: GPU bindings record separate checked host collection/index
  adapters instead of collecting a device `RangeProvider` in host subplans.
  Closed range subjects obtain their capture context from row-valued bounds.
  Checked macro rejections produce one diagnostic expression, including the
  deliberately unsupported grouped-answer-gate case, without secondary Rust
  syntax diagnostics. Each finding has a default-tier regression.

- Restore the 26-output DOOP GPU pipeline after declaration-local codec IDs:
  its storage schema explicitly records `DoopSymbols` as codec 0, and its
  decomposition catalog explicitly authorizes that identity, ordering and
  host/device/block/lane levels. No checker or implicit codec authorization
  is relaxed; the integer-only SRD DOOP pipeline needs no symbol token.
  Closed symbolic answer captures also retain their declared answer-slot
  logical identity during GPU codec/carrier recording, so arena metadata uses
  the recorded code carrier rather than requiring the logical symbol to
  implement `Scalar`. The symbols oracle covers a closed symbolic head.

- Fix (DriftWatch5 #1): a hoisted arm without a finite domain sees its free
  columns but no seed provides them. VO records `provided_before` per step
  (the scope's seed plus its earlier steps), so a nested domain-taking block's
  free columns enter `domain_requires` and WF-BlockPlacement, the reducers'
  domain boundary and cq-order readiness (schedule, Fine and the Filtered
  check) wait for the arm's own generator instead of lowering
  `project Unit keep {x}`. Regression:
  `a_nested_domain_block_in_an_unseeded_hoisted_arm_follows_its_generator`.

- Elaboration redesign slice 5 (pure rename): `cq-elaborate` is now
  `cq-relational-lowering` (`cq_relational_lowering::{compile, check}`), and
  its rules use the `RL-` prefix: E-Emit→RL-Emit, E-Relation→RL-Occurrence,
  E-Check→RL-Check, E-Project→RL-Project, B-Root→RL-Root, B-Join→RL-Join,
  C-Guard→RL-Guard, C-Bind→RL-Bind, C-For→RL-For, E-ReduceWith→RL-Reduce,
  C-Finish→RL-Finish, E-Constraint→RL-Constraint, E-Restrict→RL-Restrict,
  E-Neg→RL-Neg, E-BlockPlacement→RL-Block, CK-Elaborate→CK-RelationalLowering
  and CK-ElaborateRestrict→CK-RelationalLoweringRestrict (C-Ready in
  `cq-order` no longer shares a prefix). The formal subsection is "Relational
  lowering: VO → RA′"; stage-differential labels read `ra:relational_lowering`.
  Generated code is unchanged.

- GPU fuzz regressions: catalog selection uses its checked retention contract
  rather than the strict Physical-preserving Structures wrapper; whole-relation
  reads demand recorded source rows and counts, including filtered Cartesian
  operands. Independent literal-column restrictions remain predicates instead
  of being forced into one interval. Dense range providers normalize signed
  keys with a recorded base and checked domain size, preserving signed payloads.
  Inferred resident-IDB answer binders carry the declared head's scalar inference
  constraint into bounds and arena publication metadata.
  Closed computed restrictions evaluate as recorded host Boolean captures;
  ordered query answer gates return empty row/arena results before later
  captures or inputs, and host subplans reuse outer capture bindings.
  Grouped publication with an answer gate remains explicitly unsupported and
  is rejected before printing. Minimized cases are default-tier regressions.

- Resolve planning issue 7 with declaration-owned `cq_syntax::LogicalTypeId`:
  relation and provider schemas preserve explicit identities and intern omitted
  identities once. Sequence codec obligations, GPU codec registries and Datalog
  symbols look up logical IDs rather than Rust types; mismatched schema IDs and
  conflicting codec IDs reject, while equal spellings with explicitly different
  IDs remain distinct. GPU Value captures also record `CodecId`; their checker
  rejects identity drift, and capture/device-input codec payloads must match
  their declarations before printing. Runtime codec laws remain premises.

- Documentation audit: distinguish current compiler/runtime contracts from
  historical mechanisms and benchmark snapshots; add report provenance and
  same-workload supersession links without changing measured data. The GPU
  profiling guide separates the removed cvc5 backend from the later P6b DOOP
  evidence, and explicitly flags unavailable measurement provenance.

- DriftFix4: domain-id exhaustion is a parse error rather than overflow;
  IX checks recorded supply legality without replaying demand classification.
  Whole-relation reads are checked access facts consumed by pull and push
  printers, preserving their generated code. Physical evaluator panics and
  malformed reads now fail differential tests and count as accepted-mutant
  holes; generated seeds have no blanket exemptions. Remove the obsolete
  zero-column-index fuzz pass and unfinished strings profile flag; narrowing
  mutations use typed column metadata. Formal and conformance inventories
  describe the current rewrite, retention, planning and provider rules.

- Elaboration redesign slice 4: a hoisted block that takes a finite prefix
  domain (an arm lacks a generator for a free column) now takes it from the VO
  prefix that binds its free columns. VO records this as `domain_requires`
  (all free columns outside the scope context, no longer only Rust-produced
  ones); WF-BlockPlacement requires the block, and every reduction naming it,
  to follow those columns; `cq-order` waits for them and `cq-reduce` places
  reductions at that boundary. Lowering and CK-BlockPlacement no longer join
  the written prefix in written order. H-Local loses its hoisted-domain cut
  (and `local_predicates` its `context`/`hoisted` parameters): a predicate
  written after such a block restricts the atoms before it, since the domain
  is the restricted prefix. The checked RA→RA rewrites `simplify_reductions`
  and `simplify_expansions` move from `cq-elaborate` to the new pass crate
  `cq-ra-rewrites`. `cq_reduce::filter_ranks` (Masks, Hypergraph, planner
  transfer and their checkers) ranks through the scope's recorded
  `restrict` steps instead of rerunning H-Local, so a VO with fewer
  restrictions (the written reference) never ranks a support as filtered;
  `hypergraph::filter_ranks` takes `(target, predicate, written)` triples.
  Fix F6: an equality-class reduction (repeated atom argument, Fine split +
  Tree) now joins the support projected to its recorded subatom columns, the
  wiring CK-Elaborate states, so `stage_differential -- --ignored` passes for
  seeds 557/877/941 (regression test in `pass_contracts`).

- The opt-in GPU fuzz harness selects the GPU catalog, downloads device CQ
  answers explicitly, and builds generated consumers with the pinned oxide
  materialized-cubin release path. It attempts both CQ and Datalog lanes and
  reports each outcome instead of letting the first failure hide the second.

- P7 construction and authored filter publication preserve deferred choice
  ownership: exact indexes, attached sidecars and filter predicates are owned
  outside lazy alternatives and initialized only by the selected arm. A runtime
  regression checks both choices, one build, and exact answers after the choice.
- GPU Datalog input bindings now instantiate their declared Structures providers
  through the shared CPU/GPU declaration producer, preserving the construction
  record instead of inventing a printer default. Scan argument printing reads
  recorded Round provider types directly; host-only tests cover ordinary input
  construction and rejection of unrequested statistic construction.
- P7.2 implementation, subject to integrated gates: CPU run/hash providers expose
  O(1) optional build statistics (`RT-BuildStats`) without forcing lazy work.
  Structures now records ordinary versus statistics construction with a demand
  mask (`STR-Construction`), checked against provider-declared `statistics_build`
  support and matching immutable publications. Printers no longer infer a
  constructor from a MaxGroup accessor. No-consumer construction emits plain
  `Build::build`; the runtime's ordinary Build does not accumulate statistics.
  `BuildWithStatistics` and `BuildWithSidecar` carry the recorded row/max mask.
  Lazy hash demanded-initializer selection is outside the ordinary OnceCell
  closure, avoiding the H09 hot-loop register-allocation regression identified
  with expanded code, counters, CPU perf and an isolated diagnostic. Cheap
  lengths and explicit direct-constructor behavior remain unchanged.
  Routed unit-sidecar finalization reuses its original shard-handle allocation;
  the allocation-identity regression checks the absence of a second vector.
  Borrowed run filters provide exact point and interval tests (`RT-RunFilter`);
  `run_filter` source capability and mutually exclusive `borrowed_filter` /
  `attached_filter` realizations are declared token facts. P7.2 optional-statistic
  activation and unit-sidecar publication are authored-advice-only: the built-in
  default retains main's ordinary Build, exact key-count threshold and projected
  filter construction. The activation-enabled candidate's 20 paired measurements
  are recorded in PLANNING-REDESIGN P7; H05/H21/d72 losses prohibit promotion.
  Automatic activation, device and committed-round publication remain unselected.
- P7.1 records provider statistic declarations and immutable build publications
  with typed units, precision, producer/accessor ABI, availability and domain
  epochs (`STR-BuildStats`). Sequence statistic guards check publication
  dominance, exact reference identity and host scalar placement, with explicit
  unavailable-value fallback (`BE-StatsRead`); no automatic choices are added.
  Evidence retains per-fact authored/original-VO provenance. Legacy ambiguous
  Sequence `key` is replaced by advice-only `planning_key` and explicitly
  semantic `trusted_key`; only the latter feeds key-dependent rewrites.

- Codec identity is now `cq_lang_structures::provider::CodecId(pub u32)`.
  Structures `InputColumn`, Sequence `Obligation::Codec` and `CodecToken`
  record `codec_id: Option<CodecId>` beside the printing-only codec type;
  `None` denotes identity. The Structures declaration boundary interns omitted
  ids once by opaque token spelling, so automatic equal spellings share an id
  but explicitly different ids stay distinct even with identical spellings.
  Input syntax accepts `codec [0] Type` or an omitted id to be interned.
  Checking, GPU compatibility and catalog admission compare recorded ids,
  never codec types. Builtin syntactic `Dense` catalog tokens are removed;
  custom codecs must be supplied explicitly. GPU arguments use
  `decompose(codecs [CodecToken { codec_id: Some(0), codec: Some(Type),
  ordered: true, levels: [[host], [host, device]] }])`; identity uses
  `codec_id: None, codec: None`. Codec injectivity/order laws remain premises.
  GPU Datalog advice also accepts `symbols(T => [7] Codec)`; omitted ids
  in `symbols(T => Codec)` are interned once across the advice. Storage retains
  named logical/codec_id/codec records, and input schemas copy those ids;
  checkers compare recorded ids without re-interning.

- P4-GPU slice C replaces the packed/wide canonical stores with ordinary
  full-row/unit-item `RadixLayout` construction and the constrained `radix_set`
  Distinct token. Complete-key uniqueness shares stable ordering with the
  occurrence engine; device count publication and sorted owned-prefix
  adoption avoid a synchronizing snapshot. Checked `SetBuild` records explicit
  construction parameters including block width; the consumed FOR codec owns
  domain/width provenance, without a redundant classification. Set construction
  reuses growing per-thread/pool sort, permutation, compaction and count scratch,
  separate from immutable result owners; query teardown releases it in stream
  order. Canonical grouping, union and chained callers use this one protocol;
  h06's keyless fold and CPU provider selection are unchanged.

- P6 slice A adds an opt-in shared S1 Plain/Bytes format/runtime boundary
  and checked per-consumer carrier-read facts. Explicit domain/layout/family/
  carrier and snapshot metadata, optional encode-time vector extrema and
  initialized-byte transfer preserve the existing format rather than aliasing
  GPU FastLanes. Read authorization remains a Structures decision, not a
  runtime or printer inference. No automatic/default selection, shared-format
  FastLanes selection, retirement of the 74 P6 V sites, or performance claim;
  untouched default expansions must remain identical. Formal/conformance
  registration describes obligations and fixtures, not executed gate results.

- F5: task decomposition recognizes independent union arms beneath unary
  consumers (including the canonicalization of lowered set union), publishes
  each arm separately, and concatenates/canonicalizes at host. The dispatch
  checker remains unchanged: identical-valued relation occurrences do not
  make their stored spaces interchangeable. Default tasks-route regressions
  check grading and empty/duplicate-containing answers.
- Updated the task routed-membership test's stale token declaration to the
  canonical `AdaptiveMembership<_, 16777216>` declaration; catalog equality
  checking remains strict.

- GPU integration: explicit `decompose(codecs [CodecToken { ... }])`
  declarations admit custom codecs by exact identity; the symbols fixture
  declares `TestCodec`. Publication builds record device-layout indices,
  excluding host-only indexes. Whole-relation lookups are recorded and
  checked before emission, and scan resident or published rows without
  constructing empty-key indexes (TPC-H h07, h07_block, h11 and h15).

- Elaboration redesign slice 3b: H-Local is order-independent. A relational
  predicate restricts every covering positive atom of its scope, written
  before or after it; the only positional premise left is the domain cut (a
  predicate written after a hoisted domain-needing block does not restrict the
  atoms before that block). Ordering, H-FilterRank, Masks and lowering share
  the rule, so `R(a,x), where x < 5, S(x,b)` and `R(a,x), S(x,b), where x < 5`
  restrict the same occurrences, and `A(a,b), B(b,x), where x < 5, C(x,d)`
  now restricts `C`, the Masks support its rank names.

- Datalog Source interns code-domain identities across its full declaration
  table before producing Storage/Round or individual CQ holes. Hole subsets
  preserve those recorded ids instead of assigning new local identities;
  exact declaration correspondence remains unchanged.

- DriftFix1: IF-Capture binds each host value once and gives every `move`
  closure that reads it an owned `Clone::clone` at closure creation, rebinding
  the capture reference inside that closure. Nested closures own their own
  clones; there is no per-row clone inserted into the closure body, type-based
  ownership choice, compiler-introduced `Rc`/`Arc`, or forced answer collection.
  A closure created per row still pays its own creation-time clone per row.
  Typed `Capture::Domain` records carry the declared domain, operation
  (`exact`, `lower`, or `upper`) and closed operand through the IR; printing
  realizes that record rather than recognizing generated Rust calls.
  Domain identity is interned once in the declaration table and recorded by
  id in captures; A-Domain and WF-Domain compare those ids, not Rust syntax.
  The former `cq_storage::capture::Value::new(c).share()` handle protocol and
  its runtime module are removed.

- GPU realization records a single provider-build owner: a source or a
  publication step, never both. The realization checker rejects duplicate
  source/publication ownership before emission; printers consume the checked
  records instead of selecting another provider owner.

- Retire obsolete Arc/share-handle approvals from the optional
  `tools/expanded_gate_classes_interned_strings.json` expanded-output ledger.
  Its historical exact hashes do not approve the owned-clone/typed-domain
  cutover. Unrelated reviewed rules remain; removing obsolete approvals is
  fail-closed, so new capture hunks require their own reviewed hashes.
- Independently validate GPU Datalog storage facts instead of replaying
  realization. Record merge tiles and auxiliary launch widths, remove the
  temporary sorter's hidden ratio, and make worker lanes consume recorded
  arena initializers and release thread-local scratch at component boundaries.

- GPU Sequence checkers validate recorded facts by legality instead of
  replaying their producers: constant guards, source intervals, device
  carriers, downloads, radix reads, launch partition/share/metadata, and the
  realization (union arms, transfer coverage, provider origins, captures,
  auxiliary geometry). A provider now has one owner that precedes every
  reader, so a resident rename republished below host is no longer built twice. Publication storage records
  the arena encoding of a raw-candidate terminal; the printer reads it, the
  recorded BE-Resource payload parents, terminal-group and combine geometry,
  per-family auxiliary kernel geometry and numeric upload bounds provenance,
  which the runtime takes as explicit `Geometry`/`Auxiliary`/`UploadColumn`
  arguments. Bindings name declared provider tokens (`packed_provider`,
  `host_scan`) and the GPU grouping check compares tokens, not spellings.

- Check TPC-H grouped publication slots against their head arguments and bounds
  residency against recorded table-loader or staged-input placement; reject
  missing placement records instead of silently assuming device residency.

- Fix automatic Datalog in-plan admission after CQ reorders positive reads:
  only explicit `known(at(...))` advice records a numeric `Planning.known` pin;
  automatic anti-joins run once their actual head bindings are ready. CPU
  heap-allocation coverage checks captured filters, changed constants, and empty
  inputs. Known-placement diagnostics retain the valuation-contract error.

- Elaboration redesign slice 3a: relational predicate placement is recorded in
  VO. `cq-order` emits `restrict R@i where P;` at the head of each scope
  (O-Restrict) from one shared definition, `hypergraph::local_predicates`
  (H-Local), which H-FilterRank, Masks/Hypergraph reductions and planner
  transfer also rank through. A written predicate with a restriction target on
  every written instance has no prefix step; E-Domain implied domains exist only
  as restrictions. Every VO producer (ordering, advice, step advice, evidence,
  LE and the transfer planner) records them; `compile_written` records none.
  The LE planner schedules `cq_order::compile_unrestricted` and then calls
  `cq_order::restrict`, so prefix predicates still drive its expansions.
  WF-Restrict checks legality, CK-Order checks consistent step omission, and
  CK-Elaborate checks each restriction selects its own occurrence. Lowering
  applies only recorded restrictions: `scope_predicates`, `optimize_predicates`
  and `cq_elaborate::compile_written` are removed (the written reference is
  `cq_elaborate::compile` of `cq_order::compile_written`). The cq-testkit VO
  evaluator executes restrictions. Masks no longer reduces by a support whose
  filter elaboration never applied (`A(a,b), B(b,x), where x < 5, C(x,d)`).

- GPU Datalog negative key providers are selected in Storage before Fork
  declares their CQ inputs; binding an arena must not replace a provider
  underneath its recorded host-read capabilities. A CPU-only regression covers
  `device_storage` through `bind_round` with negation and rejects fabricated
  host membership declarations without weakening Fork correspondence.

- Datalog invariant materialization preserves the complete projected column
  schema, including its declared code domain. The independent invariant
  certificate checker requires the same domain, preventing hoisted frozen
  filters from joining interned codes through an undeclared domain.

- Sequence Analysis now owns checked BE-PrefixBound row/key records and
  BE-GroupKey canon-free group facts. Retention replay consumes these records,
  with explicit tile/threshold parameters and checked root, first-bound and
  following-expansion premises; it no longer reruns bound production.
  Group canon removal is selected only from the outermost Group constructor's
  recorded fact (slot zero), never a nested Group's fact, and rewrite failures
  propagate instead of silently becoming negative uniqueness decisions.

- Sequence catalog admission no longer admits silently. `Obligation::Codec`
  records the declared Structures input `codec_id` of each column's logical
  type (`None` for identity; conflicting ids are a BE-Code error), and only a
  `CodecToken` matching that id, ordering and every graded level discharges it.
  `Obligation::Canon` records the answer boundary's identity index
  (`answer_index`) and the `level` grading assigns; the catalog check reads
  them exactly instead of accepting any Distinct entry or defaulting to host.
  Codec types are retained only as `codec: Option<syn::Type>` printing payloads.

- Sequence placement recognizes a native host provider by its token's declared
  `abi` fact (GPU-NativeAbi) instead of inferring nativeness from catalog
  entries without a host read level. A rebuilt index over such a scan is row
  work published below host; level shape alone no longer confers nativeness, so
  `abi columns` scans (e.g. `DeviceColumns`) now count even when the target
  catalog does not register their token. Tests cover the declared, host-level
  alternative, matching, undeclared and PL-Catalog-rejected omission records.

- Datalog review fixes: fork records Round planning decisions and lowers them
  through a mechanical pipeline, checked by an independent scan/negation
  correspondence checker. Storage `Negative` records now carry their key-set
  provider token and hasher (checked as `set`-protocol instances over `_`);
  `hasher(keyed)` selects them and fork copies them into Round, so the
  `hasher_round` Round rewrite is removed (drop it from `round = [...]`).
  Aggregate lowering no longer builds a redundant `seen` `HashSet`.

- Align current storage documentation and task-catalog comments with direct calls:
  document `build typed`, native-read and `abandon N of H` grammar, declared host
  reads, explicit provider codecs, and recorded dense membership and abandonment.
  Match IF-HostDecl's empty-read and present-support-clause premises to the checker.
  Generated-code compatibility: merge readers select a recorded ordered fallback
  rather than a native ordered lookup, which Sequence cannot execute. The checker
  continues to reject authored native ordered Sequence lookups.
  BE-NativeBind records the required capability per actually checked lookup
  `(provider, reader, keys)`. Grading promotes only the affected lookup
  requirements and rechecks each against its own native read, not an aggregate
  requirement shared by unrelated readers.

- IX correspondence now checks per-read requirement coverage, no extras, and
  first-use naming without replaying demand construction. Physical lowering
  consumes matched recorded supply and key ordinals instead of reclassifying
  selections or intersecting join headings; independent demand correspondence
  remains in place. Valid programs retain the same lowered operators and keys.
- Sequence resource accounting now consumes Structures' shared carried-column
  demand and Rust dependency helper instead of duplicating the Physical DAG
  analysis, preserving checked-program payload layouts.
- Structures argument slots (RevMiddle F5, S1, S2): a provider declaration may
  declare `items N [rows T, columns T]` or `choice N [eager A, lazy A]`; an
  instance spelling ends before that argument and the checked analysis prints
  it from the recorded fact. `radix_run` declares its layout slot, so the
  `items rows|columns` fact alone prints RadixRun's layout argument and
  `radix_runs` records the token's default instance (STR-ItemLayout); retype
  advice states `items rows|columns`. `keyset` declares its laziness slot,
  printed from the served reader's Physical eager/lazy choice
  (STR-ReaderChoice), replacing the hand-spelled `KeySet<_, false>`.
  Adaptive thresholds are declared once per token: `adaptive_membership`
  (`AdaptiveMembership<_, 16777216>`), new `join_filter_membership`
  (`<_, 67108864>`) and `positional_keyset`
  (`AdaptiveKeySet<_, I64Codec, 67108864>`). Printed spellings are unchanged
  except that eager host key sets now print the default `KeySet<_, true>`
  explicitly.
- Physical records the head-dedup obligation (RevMiddle F6, PH-HeadDedup):
  `return [canon] [dedup] r as H`. Lowering records `dedup` when a computed
  head may collapse valuations and the return is not `canon`; the Physical
  checker requires it where answers collapse and admits an extra one.
  Structures checks only that an answer identity index types the recorded
  obligation; `cq_lang_structures::head_needs_dedup` is removed.

- Fix CQ normalization of `where` predicates over hoisted closed `let` captures:
  capture binders remain in scope and both predicate operands resolve as closed
  host values, including previously canonicalized equalities. Regression tests
  cover literal and opaque let/guard/head captures and facade answer correctness
  with host-once evaluation.

- GPU correctness: preserve full-width secondary membership keys, intersect
  folded equality bounds with preceding constraints, and reject unmapped live
  row reads instead of substituting zero. Flat publications now write only
  recorded resident columns; deferred payloads are read at expansion. GPU launch
  legality checks offset/synchronization records and positive tile sizes.
  Removed the unused dense-row carrier path and narrowing membership helper.
  Regressions reproduce the failures before repair; Sequence/core and SRD
  exactness pass, and TPC-H SF1 advised/automatic are each 26/26 exact.
- P4-GPU slice B: GPU CQ built indices over `RadixLayout` and genuine
  `probe smallest`. The GPU catalog registers `radix_layout` (reads
  exact/range/membership/candidates, abi radix) after the existing tokens;
  each instance carries authored `radix { word; offset; ratio; radix_bits;
  merge_tile }` advice (GPU-RadixAdvice). GPU binding records
  `ProviderAbi::Radix` with key/lane carriers, advice and read forms
  (GPU-RadixProvider); builds use `RadixLayout::from_sources`
  (GPU-RadixSourceBuild); probes encode once into whole-word intervals
  (GPU-RadixEncodedProbe); `device::distinct`/`distinct_at` give exact union
  candidate counts (GPU-RadixCandidates). GPU-Smallest records each
  PH-Smallest probe's column, sides, exact counts and first-in-order ties and
  rejects non-radix sides (never a First probe); the blanket T-Gpu smallest
  rejection is removed. Retain/Expand stay unregistered. Packed canonical
  field bounds saturate at `i64::MAX` instead of rejecting 64-bit spans with
  a positive base. The parent row counts and chooses once and passes the
  choice in a recorded position tag (`ChoiceCarrier::PositionTag`). The new
  authored `triangle-smallest` binary (radix sides, smallest, WideCanon) is
  exact on DBLP and on full-domain signed i64 DBLP, but 1.54× slower in
  paired timing than `triangle-compiled`, which is now the `probe first`
  Range plan. Radix smallest is not a default anywhere.
  GPU TPC-H SF1 advised/automatic are 26/26 exact each, with generated
  `queries.rs`/`plans.json` byte-identical to `17a14b93` apart from the
  build directory and generation times; srd-doop-gpu field-flow exactness
  passes; all 46 expanded-Rust targets match `17a14b93`, 29 with the
  `cq-sequence-tasks` runtime fingerprint change (`radix: None` field).

- Sequence rewrites now reject non-retention Structures/Physical changes with
  `BE-PhysicalPreserved`, including support-equivalent join reordering.
  Removed the unused join-reexpression certificate; result-support equality
  remains for construction checks. Formal rules and conformance now record
  this boundary and GR-RetainPrefix's stop at the first unknown bound.
- GPU provider identity (contract 7, C7Provider G1/G2): provider declarations
  record native ABI, contiguous-range eligibility and resident-column bounds;
  input instances record ordered logical/storage schemas and explicit codecs.
  Native binding and range publication consume these records rather than Rust
  constructor/generic-argument recognition. Structures checks schema cardinality,
  required native Host input and declaration consistency. GPU builds and suites
  pass; TPC-H SF1 advised/automatic are each 26/26 exact, SRD field-flow fixture
  checks all 43 outputs, and Cassandra matches all 26 complete tuple sets.
  Advised and automatic Sequence execution records and kernel sources preserve
  identity after erasing added facts/module hashes. All 46 expanded-Rust targets
  match `c395fe83`; 29 require fresh CPU verification for the conservative
  `cq-sequence-tasks` runtime fingerprint change (catalog initialization only).
- No fallback tiers (contract 7). Autoref dispatch tiers are deleted
  (`Host`, `BorrowedHost`, `PointHost`, `RangeHost`, `build!`, `AutoDense`,
  `BuildTarget`/`BuildProvider`, `JoinFilterKey` eligibility); every decision
  is a recorded, checked fact printed as one direct call, and rustc reports
  mismatches. Inputs declare keyed reads with `relation R(..) [= (e)] host
  [k..] [borrowed] reads [exact|membership|candidates|range|ordered]
  [supports [retain|expand]]` (IF-HostDecl); undeclared hosts are row
  sources read through recorded fallbacks. Structures records `R native
  <reader> = <read> [key]` in host declaration order (SEL-/STR-NativeHost),
  Sequence binding accepts it (BE-NativeBind), and the printer emits one
  `dispatch::*` call with a `require::native_*` guard naming the missing
  trait. Providers build through `<T as Build<_>>::build` (`build typed`
  replaces `build macro`); key sets and sorted runs spell `I64Codec`;
  one-column membership records `dense_membership` (requires `OrderKey`),
  wider keys `adaptive_membership`. Join filters (selection and the
  `join_filter` advice) record `dense_membership` and `abandon 35 of <base>`
  (SEL-JoinFilterAbandon). `OrderKey` gains `i8`, `i16`, `isize`, `u8`,
  `u16`, `usize`, `char` and `bool`. Tests declare the hosts they read
  natively; a compile-fail test checks rustc names `IndexRead`/`ExactMembership`
  for an unsupported declared capability or key type. Ordered-read rejection
  fixtures declare their native host contracts explicitly; undeclared maps may
  legitimately build an ordered index. Literal native reads have no string
  special case: the declared map fixture uses interned integer codes.

- Datalog provider tokens declare CQ host reads (contract 7, no fallback
  tiers). Storage `Token` gains `reads {[exact | membership | candidates |
  range | ordered]}` and `supports {[retain | expand]}` (WF-DL-Storage checks
  each listed once, supports only with `exact`). `hash_set`: membership;
  `hash_map`, `sharded_layout`, `run_layout`: exact, membership, candidates;
  `rows_vec` and device tokens: none; no base token declares supports (their
  cursors are not `Copy`). Fork and Round correspondence give every layout
  and key-set hole input one `host [leading key columns] reads [..]`
  declaration from its token (none for key-zero layouts, deltas and scans);
  a key set with projected columns collected by a non-batch key hole stores
  borrowed columns and declares `borrowed reads [membership]`.

- Datalog provider identity (contract 7, C7Provider C2). Storage/Round
  providers are recorded `@token as Type` instances: `Layout.provider` plus
  `hasher { fx | keyed | none }`, `Providers.delta/next` (`rows_vec`) and
  Round `KeySet.provider`/`hasher`. Storage `Program.tokens` declares each
  token once with its spelling function, `protocol`, `admission`
  (`sharded_hash` | `sorted_subtract` | none), `hashed` and `task_hasher`;
  WF-DL-Storage checks every spelling against its token's spelling of the
  recorded key/payload schema. `hasher(fx|keyed)` records the hasher of hashed
  tokens and respells them (no Rust path or generic-argument recognition);
  `shard_storage` converts `hash_set`/`hash_map` to `sharded_layout` or
  `run_layout` by token identity (A-127); the parallel printer reads admission
  and task hasher from the canonical token (A-128). Partition admits non-bulk
  layouts by declared protocol, so authored grouped-map/set tokens partition.
  GPU Datalog device storage declares verbatim `device_layout`/`device_view`
  tokens (compile-only). Generated code unchanged: expanded gate 46/46
  identical vs `c395fe83`; DOOP CPU engine expansions identical.

- Evidence-driven Lookup/Expand ordering and grading (P3.3 slices A, B).
  The VO records `valuation evidence from "<source>" { count R = n; key R
  [cols]; }`; `PlanOptions` gains `evidence …` (exclusive with `order`) and
  `known(R@i at k)` (O-KnownAt, applied before Masks, re-checked after; the
  pinned read follows the bindings and filters directly after the k-th read,
  so it sees `let`-named computed head terms).
  O-EvidenceLe streams the largest-count key-functional root that dominates
  the written root (count ≥, filter rank ≥, no neighbour other than the
  written root filtered as strongly), expands keyed reads by filter rank and
  looks up other ready filtered keyed reads first; CK-EvidenceLe checks the
  recorded choice against the recorded evidence. Grading bounds prefixes from
  recorded counts and keys: a GPU root below one tile retains the first prefix
  reaching `retain_prefix_rows` (65,536) and tiles over it (GR-RetainPrefix);
  later keyless expansions are segmented (GR-SegmentFanout); `retain(name,
  rows)` advice is re-checked. Only the GPU TPC-H automatic route records CQ
  evidence. Paired GPU vs `c1b75718`: H04 automatic SF1 0.661, SF10 0.500;
  H02/H11 SF10 advised 0.733/0.422; H05 step-R order SF1 0.121, SF10 0.070;
  d85 identical code.

- CPU evidence and Datalog fan-out evidence (P3.3e). CPU TPC-H `default`
  and `par` record CQ evidence (R-TpchEvidence): the `query!` driver in
  `features/workloads/src/tpch_runtime.rs` records, for each canonical base
  relation (`Lineitem`, `Orders`, …, bound by every kernel to
  `data.<table>`), its dbgen SF1 row count and its TPC-H primary key when
  every key column is declared, and compiles with `plan(evidence …)`
  (O-EvidenceLe/CK-EvidenceLe unchanged). Only `retail_h04` changes plan:
  Lineitem streams and looks up the date-filtered Orders by key. 5 locked
  AB/BA pairs vs `c395fe83`, SF1 exact: `default` 0.565 [0.543, 0.598]
  (minor faults 67.9K → 38.5K), `par` 0.229 [0.213, 0.248] (151.6K →
  40.8K). Datalog: source `evidence from "…" { key R [c…]; }`
  (WF-DL-Evidence); fork records `RuleHole.fanout` (DL-Fanout: a read is
  `multiply` iff it introduces a variable and binds no recorded key) and
  FK-Known puts an unadvised hole of a planned queue in plan iff a later
  read is recorded `multiply`; correspondence checks the record and the
  placement. DOOP mini/SRD sources record keys checked functional on the
  xalan and batik facts; with them no DOOP hole is in plan by default and
  every DOOP CPU engine (`doop-mini`, `-keyed`, `-par`, `-par-keyed`,
  `srd-doop-mini`, `-par`) expands byte-identical to `c395fe83`.
- ADM-InPlan for CPU Datalog (P3.3 slice C). Fork records `RuleHole.known =
  admit | in_plan` (FK-Known). Advice `plan(L, known(at(level k)))` puts a
  hole in plan: it binds computed head terms with `let` and appends `!C(x̄)`
  over the head's committed canonical layout after the k-th positive read
  (CQ `known(C@i at k)`, O-KnownAt), so later fan-out reads expand only
  unknown heads. Without advice (or with `known(admit)`) holes stay `admit`:
  the structural default "head binds before the last read" measured DOOP
  xalan `mini` 2.36x [2.30, 2.42] and batik `mini-par` 2.95x [2.84, 3.06]
  slower than `c1b75718` (early anti-joins probe every intermediate row where
  later reads only filter), and CPU Datalog records no fan-out evidence.
  With the advice-only default, paired CPU DOOP vs `c1b75718`: xalan `mini`
  0.950 [0.911, 0.976], batik `mini-par` 1.024 [0.980, 1.075].
  `order(..)` is optional with `known`. Fork correspondence erases exactly
  that suffix; Round checks DL-AdmitOnce and records the committed head read
  in the schedule. The serial printer omits the committed probe for in-plan
  holes; the parallel printer admits covered and checked candidates as two
  batches (`ShardedLayout::admit_batches`, runs skip subtraction). GPU Datalog
  records `admit` and rejects in-plan advice (follow-up). Admit holes print
  unchanged code.
- P4-GPU slice A (runtime only): `gpu_datalog_runtime::RadixLayout<K, V, W,
  O, C>`, the typed relational view over one immutable `DeviceRadixRun`
  publication (GPU-RadixLayout). The key packing is checked against `K` and
  `W`; each `V` field needs a `C` lane holding its domain (`LaneKind`).
  `DeviceRadixRun::snapshot` now returns a cached `Arc<RadixSnapshot>` per
  publication with a process-unique identity and a resident occurrence count
  (no host sync); `republish` returns the same layout for unchanged runs and
  shares field descriptors otherwise. `probe_groups`/`ItemRange` are replaced
  by `probe_matches` returning `RadixMatches` of `RadixMatch` cursors
  (snapshot, run, run-local `[begin, end)`; GPU-RadixCursor) that retain
  their snapshot and `gather` one demanded `ItemColumn`. Reads name an
  execution pool sharing the snapshot's memory (GPU-RadixLaneRead); pool
  checks use `Pool::shares_memory`. New `tests/radix_layout.rs` laws. No
  catalog token, compiler record or Candidates/Distinct claim; no generated
  code uses the provider yet.
- P6b GPU Datalog, per-hole op reduction and wave lanes. GPU-ResidentMeta
  (`Execution.meta`): a `reserve_previous` arena terminal with a local fold
  keeps its count/failure/cursor words in arena-resident words that its last
  block re-zeroes, so attempts no longer allocate or clear metadata (and skip
  the zero-length partials buffer). `DeviceLayout::republish` shares field
  descriptors with its predecessor and returns the same layout for unchanged
  runs. xalan profile: memsets 17.8K→6.2K, H2D copies 16.0K→6.2K per
  fixpoint; 43/43 relations byte-identical. DL-WaveLanes: the Round schedule
  records `streams` and per-wave `lanes` (WF-DL-Lanes), written by the explicit
  rewrite `::cq_gpu_datalog::lanes(streams = K)`; the GPU printer forks lanes
  onto persistent worker threads with `Pool::lane` streams sharing the
  allocator (`Pool::shares_memory` identity) and joins by events before
  admission. `srd-doop-gpu` records `lanes(streams = 4)`: fixpoint xalan
  0.865× [0.852, 0.892], batik 0.811× [0.800, 0.816] of one stream (locked
  pairs), peak pool memory unchanged. `doop-gpu` records `lanes(streams = 4)`
  (Cassandra 0.723× [0.702, 0.765]) and `roots(spread)` again (0.953×
  [0.923, 0.960] with relational heads); 26/26 outputs exact. Host printers
  reject recorded lanes. `radix_run.rs` uses `RadixRun::merge` and compiles
  again.
- P5 S1 adds runtime-only `cq_storage::scan` Plain/Bytes block columns,
  same-input-pass optional min/max summaries, caller-buffer vector decoding
  and exact encoded point/window/membership filtering. Bytes caps residuals
  at 16 bits and otherwise retains plain native-width ordinals. Runtime law
  tests cover inverse decoding, enclosure and predicate preservation; no
  compiler selection or source migration is part of this slice.
- P5 S2 adds incremental block builders, validated `ScanTable` column tuples,
  and explicit-family columnar Arrow/Parquet/CSV loaders, including interned
  string-code columns whose final remap is the block encode pass. Payload and
  optional summaries share input traversal, without an eager tuple vector.
  Columnar loader macros preserve existing row-loader declarations. The zone
  Parquet diagnostic and TPC-H example use encoded scan columns; the obsolete
  `ZoneRows::from_batches` API and fourteen P5-owned decode violations are
  retired. CSV hash/btree grouping still awaits P4 encoded keys. Loader laws
  cover round-trip, batch independence and summary enclosure; compiler
  selection is unchanged.
- GPU TPC-H source authoring moves to 26 explicit `queries/*.stages` programs.
  The checked, printable frontend records table/dictionary/host producers,
  captures, stage dependencies, owned publications and terminal projections.
  It no longer discovers stages through Rust `map`/`collect`/`chain`, follows
  `data`/`load` conventions, strips dereferences/`clone`/dictionary publication,
  or evaluates calls named `date`. Dictionary domains feed shared CQ
  host-once operand normalization; Rust payloads remain opaque. Reports retain
  the authored stage record alongside each Sequence and publication record.
  Independent CPU queries remain exactness references, not compiler input.
- Strings are always interned (A-Domain, WF-Domain; clean cutover). A column
  declaration may name its code domain, `c: StringCode in (D)`, with `D` a
  query-closed `cq_storage::CodeDomain`. CQ normalization translates every
  closed operand compared with such a column (in `where`, atoms, negated
  atoms, membership and ranges) into one host-once capture
  `::cq_storage::domain::{exact,lower,upper}(&(D), &(t))`; `<=`/`>` use
  `upper`, ranges `lower`/`upper`, literal membership becomes equalities.
  WF-Domain rejects joins and comparisons across domains. Ordered
  translation needs `OrderedCodeDomain`, which `StringDictionary`
  implements (absent `exact` is `StringCode::MAX`, bounds are sorted ranks)
  and unordered domains such as DOOP `IStr` do not. Removed with no
  replacement branch: `StringValue`, `relational_string_eq`,
  `relational_str_cmp`, `StringLiteral`, the `&str` `PredicateValue`
  adapters, `Dictionaries::literal` (use `Dictionaries::domain`), every
  printer string branch (a string bound no longer forces a scan), the
  literal-value union disjointness proof (only fallback construction proves
  disjointness), the `Expr::Lit` tests in aggregate `constant_group`
  (keys must be A-Capture-recorded lets) and GPU closed extraction, and
  Datalog fork literal key embedding (`where v == (e)` after the scan).
  P5 S0: 17 TPC-H sources write their literals in atoms/`where` over
  declared dictionary domains, and typed_* workloads intern text.
  contains/LIKE stay load-time `dictionary_where` code sets. A selection
  equating a column with a closed operand (any capture, including every
  A-Domain code) is a point-read key like a literal (`Select::constant_keys`
  returns `Term`s): leading map keys read directly through
  `cq_storage::dispatch::BorrowedPointRead`, which owns the borrowed key once
  per read (`OwnKey`) and otherwise falls back to the scan. The following
  IF-Capture handle design is historical, superseded by DriftFix1's owned
  clone per closure above: each `let` capture was bound in a handle that
  `move` closures cloned and rebound, keeping lazy answers uncollected.
  The printer emitted `cq_storage::capture::Value::new(c).share()` and Rust
  method resolution selected either the `Copy` value itself or one shared
  `Arc` for a non-`Copy` value. Historical measurements of that removed
  design: an `Arc` for every capture made CPU `retail_h07` 1.058
  [1.047, 1.137] slower than `c395fe83`; with handle choice it was 0.961
  [0.805, 1.104] against `17a14b93`. These are not measurements of the current
  owned-clone design. GPU TPC-H timing now includes the final answer download for
  every query (the stage frontend times every stage's `to_host`, which the
  removed standalone path did not); paired GPU comparisons use a base built
  with the same boundary.
- Query-closed values are host captures (A-Capture, A-CaptureRef,
  WF-Capture, IF-Capture). CQ normalization records a top-level `let`/`if`
  that reads no query variable, a closed head operand and a closed aggregate
  argument once in `capture { let p: T = e; if g; }`; RA (`Plan.captures`)
  and Physical (`Body.captures`) carry the list unchanged and check it. A
  capture is never a heading column, so ordering never places it: a closed
  `let` no longer becomes an empty-key unit prefix that builds an index
  (`cq-order` `ready()` placed it first since `0124315f`; the counters test
  `empty_negative_keys_test_existence_without_building_indexes` passes
  again). A query-closed union-branch `let`/`if` becomes a fresh capture and
  the branch keeps its binder as `let p = capture c` / `if capture c`, so
  block interfaces are unchanged. Printers bind each capture once before
  any loop, copy it into answers, group keys, fold arguments and capture
  references, and an `if` capture gates the whole answer. GPU Sequence
  binds captures on the host, folds `sum(capture)` from a kernel scalar
  (GPU H12) and publishes a capture answer column from a host-encoded kernel
  scalar (GPU H12 `answer(shipmode, 0)`); capture group keys, `if` captures
  and device-placed capture references are rejected as unsupported targets.
  The Datalog reference interpreters evaluate captures before their loops.
- Provider identity is a recorded token id (contract 7, slice C1). Every
  provider-bearing Structures entry records `@token as Type` (inner instance
  for nested/routed tokens) and the program's `providers { .. }` table
  declares each referenced token's facts once (STR-Provider, STR-Routed,
  PL-Catalog); passes read `Analysis::facts`, and `CatalogEntry::covers`
  matches by token (and inner token), never by Rust type. `ProviderToken`,
  `elements` and `matches_type` are removed. The base `cq_storage`
  declarations live in `cq_lang_structures::provider::base`. Structure
  selection no longer recognizes spellings: `supports [keys]` and join-filter
  eligibility come from the chosen declaration. Sequence catalog matching,
  catalog rewrites/placement, key partition (`@sharded(..)`, routed rejection)
  and publication read recorded routing; cq-print's accounting guards, build
  labels and retained-key guards read declared facts. `features/sequence-tasks`
  (levels from `shared`), `features/rewrites` (index/structure rewrites by
  token) and `features/workloads` (HashIndex override by token) follow. The GPU
  Sequence core only compiles against the new API; its recognizers remain for
  G1. Gates: expanded gate vs `c1b75718` has identical expansions on all 46
  targets (29 runtime-changed, verified by TPC-H SF1 29/29 DuckDB-exact for
  default and par with counters unchanged); DOOP xalan `--check` ok;
  regressions 119/119; root tests 1,207 passed; sequence-tasks 63, rewrites
  197, workloads 2,308 passed.

- DOOP heads are relational projections. Since contract 7 every Ascent-style
  head copy `rel(*x, ..)` was opaque Rust reading a Symbol-encoded column, so
  BE-RustRepresentation put all 155 `doop-gpu` holes partly at host: each
  dispatch downloaded complete rows, ran the copy and re-encoded the result
  (cassandra fixpoint 2.1 s at `4944b917`, 6.8 s on main). The shared DOOP
  source now writes `rel(x, ..)`; Datalog fork projects batch key-hole columns
  as plain variables instead of generating `*column`; and each Round hole
  records its answer ownership (DL-AnswerOwnership: on host interfaces a head
  variable whose defining binder is a positive atom is borrowed, everything
  else owned; batch columns owned), printed in Round text. WF-DL-Hole checks
  soundness without rerunning fork: `borrowed` only where a positive atom
  defines the column on a host interface, `owned` always. DL-InsertNew builds its owned key from either representation and forms
  the owned NEW row only for admitted candidates; printers that materialize
  candidates clone at collection;
  the hard-coded owned admission is gone. DL-Source checks head range
  restriction by free variables instead of recognizing `*x` / `x.clone()`.
  Cassandra `doop-gpu`: no host stage in any Sequence program, 26/26 exact,
  fixpoint 6.85 → 1.94 s (5 interleaved pairs, new/main 0.284, 95% CI
  0.276–0.292), 307K → 34.5K launches, D2H 1,392 → 0.2 MB. CPU DOOP `--check`
  passes on xalan and batik for every engine; CPU paired timings show no CI
  entirely above 1.0
  (`experiments/gpu/compare/doop_cassandra.relational_heads.results.json`).
  Plan issues 9 and 10 are now closed: recursion heads use plain relational
  variables and Context records per-column answer ownership, while frozen-filter
  invariant helpers project plain variables and use existing DL-AnswerOwnership.
  Removed Rust-copy recognition and the primitive-type ownership classifier.
  Old recursion `*x`/`.clone()` heads now diagnose writing the plain variable;
  non-Copy String and alias copies remain supported without type inspection.
  Recursion's uniform Clone realization is recorded in owned hole heads before
  CQ planning/canonicalization, not after sorting borrowed tuples. The latter
  was rejected by paired timing (+2.8%); the retained form preserves all
  expanded operations except four dereference-to-Clone spellings. Extension,
  Datalog, parallel, workload, String/alias and full-set xalan checks pass;
  five locked timing pairs show no CI entirely above 1.0 for recursion-path
  or invariant-enabled xalan (`tools/c7_open_issues.results.json`).

- `doop-gpu` builds again (broken since contract 7, `0124315f`). Placement
  predicted descendant reads of a pipe-host handle with its own blanket
  rejection, while the catalog check admits them under an explicit `Resident`
  token (PL-Resident, added in `1c0523a7`). A resident `DeviceLayout`
  anti-join membership or exact probe feeding opaque host Rust over encoded
  Symbol columns (MethodImplemented, both MethodLookup inheritance rules) was
  therefore left at host, where no token admits it. Placement now predicts
  read levels with the catalog's own `provider_admitted` predicate. Cassandra
  matches `doop-mini` on all 26 outputs; GPU TPC-H advised modules and every
  `srd-doop-gpu` planned Sequence program are unchanged.

- `HashIndex` initialization reserves the remaining source rows once its first
  65,536 rows carry distinct keys, instead of doubling through a chain of
  Swiss tables and arenas. Large unique-key builds (H12's 1.5M-key Orders
  directory) no longer rehash, and no longer allocate, fault in and free the
  intermediate tables on every run: CPU TPC-H SF1 H12 drops from 22.4K to 8.7K
  page faults and from 368M to 286M instructions per query. Sources with
  repeated keys in the prefix keep doubling growth.

- P3d track A, canonical reference cutover: a canon result is stored in its
  canonical index, an ordinary index chosen by index selection. Structures
  records `r canon = rows | [cols] | host` (STR-Canon) and `r head = Type`;
  default selection types rows by the sorted-array provider
  `::cq_storage::dedup::Sort` (SEL-Canon), an existence-only single complete
  Built index may be the reference, and a direct load-verified host rename
  uses `host`. Canon results lower to `let` unless they are host-rename pipes
  (PH-Canon); single-consumer rows records move by value (STR-RowsByValue,
  shared predicate `Analysis::rows_by_value`), so Sequence lowers a by-value
  canon `let` without a Store, exactly like a canon pipe: Tasks still routes
  it and canonicalizes per part, folding partial groups over tiles.
  Sequence catalogs offer `Capability::Distinct` providers (PL-Distinct)
  instead of canon tokens, and `Obligation::Canon` carries the canonical
  source. Removed: `CanonStrategy`, the `sort`/`ordered`/`using` canon
  syntax, `CanonToken`/`Catalog.canons`, and the
  `cq_storage::maintain::{Fresh, Publish}` traits with their impls, laws and
  D-only `Default` impls; `RadixRun` publication becomes `RadixRun::merge`.
  Formal: IX-Fresh, IX-Publish and IX-RunPublish are replaced by IX-RunMerge.
- Formal: EXT-MergeRetained (`merge_joins` treats a single-unkeyed-reader
  canon `let` as a canonical stream, not retained merge order) and
  BE-OrderedFallback (an ordered host read uses its merge reader's recorded
  fallback provider).

- Unify dense membership and radix/rank keys under `cq_storage::OrderKey`:
  one total, injective, order-preserving ordinal covers stored and absent keys.
  Remove `DenseOrdinal` and `RadixKey`; custom keys implement `OrderKey`.
  `KeyCodec` is now a sealed representation policy: enabled adapters reuse
  that canonical ordinal, while `NoDense` retains the generic fallback.
  Partial/custom codec implementations are replaced by the shared key law.
  Signed i32 and unsigned u32 adapters now use the same ordinals as radix runs;
  native u64/u128 membership and existing provider reads remain supported.
  Export `cq_storage::HashMap` with configurable/default `HashState`.
- Radix index runs use ratio-bounded immutable runs, stable merge-path and
  narrow group offsets; the inherent `RadixRun::merge` (IX-RunMerge) is a
  multiset union. Membership/count/range reads do not touch payloads.
  `RadixRun<K,V,C=Vec<V>>` supports explicit `ItemColumns` layouts, including
  tuple-of-vector SoA columns and selective `column_groups` reads; old flat
  constructors and contiguous item APIs remain available. Radix lookup now
  returns a Copy/ExactSize cross-run cursor through `IndexRead::Cursor`.
  Set-bucket hash maps support ordinary reads with independent key/payload
  hashers. Single-run retained radix expansion resolves its directory once;
  key scans merge runs without sorting.
- P3.3-lite: remove `lookup_first`, adaptive optional-filter advice/provenance,
  and automatic Sequence relational replanning. Default and GPU automatic
  routes now use order-preserving decomposition with existing grading.
  Authored Sequence recipes, `choose`, key partitions, evidence and the custom
  `Planner` interface remain supported; the advised GPU path is unchanged.
  Retire EXT-SeqLookup, EXT-SeqAdaptive and PL-OptionalFilter; retain BE-Choose.
- P3.1 hand planning: default `plan(order(...))` accepts explicit
  `R@k(cols)` lookup/Fine subatoms alongside full `R@k` expansions. WF-VO
  and a shared CK-Le checker validate dependencies, occurrence phases,
  written ownership and outer stream; no-advice planning is unchanged.
  Physical `smallest_intersections` gains checked k-way and per-site
  `smallest(R@a, S@b, ...)` advice.
- Contract 7 (doc/PLANNING-REDESIGN.md): every Rust `if`/`let`/`for` is one
  opaque, pure, total-by-premise expression placed by its free variables;
  relational predicates are written `where P` (`where x in lo..hi`,
  `where x in [..]`, `where x == y` as a join equality). Closed Rust operands
  print as `@(expr)`. Removed `cq_syntax::{guard, arithmetic}`, CQ-Arith,
  wrapping/Decimal/widening rewrites, Ord-f32 checks and
  `AggregateBinding::value_type`: the compiler no longer inspects types for
  meaning. Sequence records `representation { identity, encoded }`
  (BE-RustRepresentation); GPU runs verbatim Rust only on identity-represented
  columns. Type-dependent provider choices use Rust trait dispatch (`build!`,
  `DenseOrdinal`, `Keys`, `encode_predicate`). Datalog accepts `where`; DOOP
  sources were regenerated. Expanded Rust of all 29 TPC-H queries and the
  migrated library was audited against the previous commit: 1,440 hunks, all
  in recorded classes; paired storage counters identical for 73/79 TPC-H
  configurations, with the other six classified placement or arithmetic
  changes.
- Contract 8 storage design (doc/STORAGE.md): no denotation change; laws for
  decode, summary enclosure, grouped-run factorization, bounded-error
  directories and per-codec preserved operations. Two storage roles: scan
  columns and plain grouped index runs (RadixRun). Dictionary code assignment
  is treated as a publication decision. Byte layout is shared across targets;
  memory placement is per target (CUDA vs Metal unified memory).
- Storage evidence (experiments/storage-bench): CPU/CUDA codec
  microbenchmarks with exact checks and bootstrap CIs. Results: plain wins
  for lookup/expand and merges on both targets; CPU scans favor byte
  truncation over bit-packing (bit-packing pays only at 24 threads); GPU
  compression pays mainly for PCIe transfer; grouping on dictionary codes is
  12–16× faster than `String`. TPC-H in load order gives no zone-map or
  join-filter block skipping.
- Design anchors: 22 `DESIGN <id>` comments at decision owners, registered in
  `doc/DESIGN-ANCHORS.tsv`; `doc/formal/check.py` requires each exactly once
  and rejects unregistered anchors.
- GPU TPC-H after contract 7: every manifest dictionary column is declared
  as published `u32` codes; string predicates are authored
  `dictionary_where` code-set relations evaluated once per dictionary entry
  (h02, h09, h12, h13, h14, h16, h20, h22); literals translate once (absent →
  empty match). `Decimal`/`Date` are transparent POD scalars that the device
  reads unchanged; layouts come from `Scalar`/`ScalarTuple` trait constants.
  Authored `let` annotations record identity representations for computed
  bindings. A query-scoped program cache in `sequence-runtime/src/group.rs`
  removes the ~28 ms per-group module reload: h06 SF1 7.42 → 0.44 ms. All 26
  queries × 2 modes exact at SF1; no advised regression above 1.15×.
- Fix: in `operator-timing` builds `CountingHasher` built a fresh
  `RandomState` per hash, so `DistinctSet` failed to deduplicate equal rows
  (915/1000 duplicate inserts accepted). It now uses fixed seeds; regression
  test in `runtime/cq-storage/src/counters.rs`.
- Decode-site registry: every code→value conversion carries a
  `// DECODE R|P|V|I|?:` comment registered in `doc/DECODE-SITES.tsv`
  (audit in `doc/DECODE-AUDIT.md`, widths in `doc/DECODE-COLUMNS.md`);
  `check.py` enforces the correspondence. Engine-wide rules (strings interned
  at input, sort-based dedup, decode only for Rust reads and publication)
  are recorded in `doc/STORAGE.md` §13 with the h01 measurements.
- Restore dense codecs for sorted runs and positional keysets through Rust
  build-capability dispatch rather than compiler type-name inspection. An
  explicit `AutoDense` build request selects `BorrowedI64TupleCodec` for supported
  borrowed integer tuple keys and `I64Codec` for supported scalar keys; other
  keys resolve to `NoDense`. Explicit/default `NoDense` providers stay unchanged.
  Routed builds apply the same dispatch per shard, retaining their recorded
  routing, parallel construction, and allocation-free singleton source path.
- Preserve streaming output for captured relational path parameters: borrow the
  existing value directly instead of treating it as a temporary-producing closed
  expression and eagerly collecting the answer.
- Keep staged positional selections as position streams between relational
  `where` predicates, including conjunctive windows on successive columns.
  This preserves recorded positional sources when compiling TPC-H Q6.
- P0 regression gates: document supplementary-filter capability dispatch as
  public API rather than hiding it. Publication regressions check parsed
  operations instead of closure-block and tuple-punctuation spelling; fan
  counters distinguish membership construction from candidate/probe savings.
- Keep the scalar positional scan inlined into residual iterator chains.
  This removes repeated scan-state reloads introduced by late positional
  Decimal filtering without changing predicates or generated query Rust.
  SF1 default paired timing against `4944b917`: Q6 1.003× (15 quiet-window
  pairs, 95% CI 0.968–1.053), H6 1.006× (5 pairs, 0.957–1.058), exact against
  DuckDB. The Q6 confirmation retains both conflicting earlier five-pair
  datasets in `experiments/gpu/compare/cpu_q06_confirmation_sf1.json`.
- Recorded-facts cutover (doc/formal/CONFORMANCE.md). Decisions that lowerings,
  printers and drivers previously made from names, syntax or literals are now
  recorded, checked and printable IR facts; each has a formal rule, a unique
  `rules.tsv` marker and an `exact` conformance row (308 rules, 363 rows;
  `check.py` now validates the conformance tables).
  - GPU launch: `Dispatch::gpu` records partition, occupancy policy, deferred
    lookup ranges, device-wide offset scan, fold completion and sync points
    (GPU-Launch, GPU-LAUNCH-OCCUPANCY, GPU-OffsetScan, GPU-FoldCompletion).
    Grids come from kernel residency × SM count; no literal `2272` remains;
    deferred ranges run multi-block followed by a CUB exclusive scan; whole
    folds use block partials plus a combine kernel; BE-Resource reads the same
    recorded fact as the emitter. H05 automatic SF10 1859 → 115 ms (exact);
    advised 8.53 ms (unchanged).
  - GPU realization: capture domains recorded per definition (no kernel-name
    reparsing), one checked codec per logical symbol type (conflicts rejected),
    native ABI by catalog constructor identity, one provider layout with
    explicit directory allocation (GPU-Capture, GPU-CodecIdentity,
    GPU-NativeAbi, GPU-ProviderLayout).
  - Sequence planning: optional-filter provenance recorded and consumed by
    `adaptive` (PL-OptionalFilter; no generated-name prefixes); BE-Length uses
    checked `Len` facts; PL-Select builds an explicit representation-preserving
    catalog order; PL-Cost reads checked group modes.
  - Front/middle: typed Structures `ReaderRef` (STR-Reader), recorded group
    accumulator (STR-Group/SEL-Group/P-GroupState), VO block `Placement`
    (WF-BlockPlacement/E-BlockPlacement), independent CK-Masks/CK-Order checks,
    CQ-Atom and WF-VO rules corrected to the intended semantics.
  - Drivers: one core default plan composition (`facade/cq-planning`,
    `mini_linq::planning`) used by the facade, GPU TPC-H advice and explain;
    printers declare `print_input = structures|sequence` (ST-Endpoint) instead
    of path-name classification; TPC-H device→host fallback, publication
    residency and recipes are explicit records (DR-Target/Publication/Recipe);
    explain observes the real pipeline at all 12 boundaries (DR-Explain).
  - Datalog: recorded `LayoutProtocol` (no provider-suffix dispatch),
    parallel publication executes the recorded operation order, holes record
    their planning and binding invocations, GPU codecs from declared symbols,
    explicit run-ratio advice (DL-LayoutProtocol, P-DL-PublicationOrder,
    DL-PlanningOwner, DL-DeviceBindings, DL-DeviceSymbols, DL-DeviceRatio).
  - Rayon/pull: producer identities replace token scanning; Tile/Choose staging
    buffers are recorded carrier publications (BE-TaskProducer,
    BE-CarrierPublication).
  - Verified: root, datalog, datalog-parallel and workloads workspaces; GPU
    suites; regression inventory 119/119; TPC-H SF1 default 29/29 exact, worst
    ratio vs `performance_cpu_sf1.tsv` 1.08 (h22, 24 ms); DOOP GPU xalan 2.2 s,
    batik 6.4 s fixpoint, all 40 counts equal to SRDatalog.
  - Still open (CONFORMANCE.md): placement first-child machine spine and
    string host policy in shared Sequence placement; typed provider
    capabilities and arithmetic IR across stages; `lookup_first` demotion.
- Planning redesign, phase 1 (doc/PLANNING-REDESIGN.md). Driving case: SRDatalog
  DOOP `HeapAllocHelper`, an acyclic rule with a captured-constant guard written
  last that compiled to a written-order cross product (xalan: 2.0e9 pairs for
  3.27e6 answers) with zero reductions.
  - CQ guard recognition: a field path rooted at a captured name
    (`constants.c_x`) is a host parameter like a bare name; `Term::Parameter`
    now carries `constraint::Parameter { root, members }` (`@name.field` in IR
    text). Comparisons against such values become relational selections,
    placed where their variables are bound and used as Masks supports.
  - `mini_linq::passes::plan` default order is `Prefix(Connected)/Coarse`:
    written root, then a ready atom sharing a bound variable before a
    disconnected one. `Heuristic::Written` stays the exact reference and
    `plan(order(..))` the authored override. All 119 regression targets and the
    root workspace pass; TPC-H SF1 default rerun recorded in
    experiments/gpu/compare (see that entry).
  - GPU Datalog route realizes filtered reduction supports: `!=` selections are
    row work placed on device (`cq-sequence-plan` placement), run-derived
    computed providers publish after device materialization and measure their
    key bounds (`rows.rs`, `sequence-runtime` SourceView bounds), catalog
    matching recognizes wildcard native tokens and keeps concrete native keyed
    layouts (`catalog.rs`, `lookup_first.rs`), and the new
    `::cq_gpu_sequence::select_catalog` adapter retypes computed structures;
    `srd-doop-gpu`, `doop-gpu` and the GPU tests use it.
  - Regression fixtures on every route: `facade/mini-linq/tests/doop_heapalloc.rs`
    (RA shape through library passes and `compile!`, execution work ≤ 8×(input+output)
    with `cq_storage` counters), `features/datalog/tests/counters.rs`
    (`datalog!` hole), `experiments/gpu/datalog/facade/tests/heapalloc.rs`
    (GPU exactness vs CPU).
  - Authored Datalog orders reach the hole planner: a `plan(Label, ..)` variant
    is marked `authored` in the Incremental IR and Fork passes its order to the
    hole's default CQ `plan` as `order(..)`, so storage layouts and the hole's
    atom order agree (`doc/formal/datalog.tex`, fork and counters tests).
  - Masks docs match the code: it is the bottom-up Yannakakis pass restricted
    to filter-carrying subtrees, not a full reducer (`passes.tex`, `STATUS.md`).
  - BE-Resource and the GPU emitter agree on shared-memory parents: backward
    liveness removes a map's written columns, and a lookup key still live after
    its flatten is carried from the parent on both sides.
  - Not adopted: treating head copies as transparent to `lookup_first`. It let
    the Sequence rewrite every recursive DOOP hole (xalan fixpoint 2.20 → 9.35 s,
    batik OOM); the cause was not attributed to plan nodes.
  - Docs: `SEQUENCE-PLAN.md` [6] records the GPU emitter decisions that violate
    T-Gpu ("deciding nothing"): grid 1 vs literal 2272, deferred lookup ranges,
    one-block serial offset scan, whole-fold finalize, per-dispatch host sync.
    §11 now states what L&E and RPT actually specify (Bloom SIP, iterator
    Expand, two-phase only for α-acyclic) versus our exact-semijoin `transfer`
    and re-probing Expands, and that GPU automatic entries are opt-in. H05
    automatic SF10 1859 ms vs advised 8.55 ms; one-block launches are 98% of
    its kernel time.
  - Result, SRDatalog default DOOP query on GPU, fixpoint: xalan 6.72 → 2.19 s,
    batik 9.10 → 6.21 s (SRDatalog 1.01 / 1.82 s); all 40 relation counts equal.
    TPC-H SF1 default: no regression vs `performance_cpu_sf1.tsv`; h09 completes.
- Datalog per-rule join-order advice, consistent with CQ `plan(order(...))`.
  Rules take optional labels (`Label: head :- body;`); `incremental(..)` accepts
  `plan(Label, [delta(R@k),] order(R, S@i, ...));` using `cq_order::OrderAtom`
  selectors (`OrderAtom::resolve_in`). Advice targets one version (base or one
  changing occurrence) and yields the incremental pass's checked occurrence
  permutation; other versions keep `driver(..)`. `translate.py` emits
  SRDatalog's rule names, and its 18 DOOP `with_plan` orders only with `--plans`
  (measured slower: batik 9.1 → 21.3 s); `block_group` is recorded as unlowered
  in the schema JSON. `srd_doop_engine!(run, incremental(..), pipeline {..})`
  applies advice; `srd-doop-gpu` uses `driver(delta_first)`. The `default-query` feature selects SRDatalog's
  default (non-field-flow) DOOP program. SRD `HeapAllocHelper` no longer probes
  `HeapAllocation_Type` with an empty key (1.5e10 xalan pairs); the only
  remaining empty-key probe is the inherent `Precompute0_String` product.
- Reconcile the canonical MCCL tree with its standalone upstream, preserving
  ordered GPU hashmap, blocked Bloom filter and multiset-difference APIs alongside
  shared-relation indexes and GPU RadixRun. Publish identical source trees from
  the primary checkout; keep build instructions valid in both repository layouts.
- Apply the workspace formatting reported by CI and preserve explicit autoref
  dispatch tests with scoped receiver bindings, satisfying strict Clippy without
  suppressing diagnostics or weakening fallback/lifetime checks.
- Add opt-in compiled H09 execution over a source-bound GPU or native CPU
  RadixRun. Retain full i64 Lineitem payloads with original row IDs, explicit
  fallible construction and immutable source ownership. Both choices execute
  the same compiled query and preserve the authored schema and join order.
- Extend native host dispatch with an explicit `IndexedColumn<N>` contract for
  single nonleading borrowed-key reads, preserving payload order and ordinary
  fallback for unsupported slots/capabilities. Verify no hidden Lineitem rebuild,
  wrong-slot safety, duplicate chronology and retained source lifetimes.
- Record paired compiled H09 build/query and fresh-ingestion end-to-end timings,
  including the small-input GPU loss rather than choosing an automatic threshold.
- Consolidate active Mac work into this Git checkout under `experiments/mac/`,
  including MCCL and the CQ Mac provider; remove active sibling-checkout paths.
  Merge current mainline while preserving generic provider support, RadixRun
  comparisons and native/GPU implementations. Adapt routed providers to the
  declared `Keys` contract and retain the closed portable-core boundary.
- Add explicit GPU-built grouped RadixRun and stable grouped merge through MCCL:
  GPU grouping, exact parallel error32 spline construction, collinear endpoint
  compaction and adaptive radix directory. Share the native CPU reader and lazy
  membership policy through borrowed storage views. Preserve u32 occurrence
  order, selected original row IDs and pinned old cursors. Record completed
  build/merge and prepared-read comparisons without a CPU spline finishing pass.
- Add the matched prepared-reader diagnostic over shared-source GPU/CPU MacRun,
  native RadixRun and runtime HashIndex, validating exact occurrence sequences
  and recording dense, sparse, skewed and duplicate-heavy samples.
- Add completed immutable GPU-built CPU-readable indexes and online flat or
  binary-carry snapshots, with failure-atomic publication and retained readers.
  Expand H03/H09 into matched manual executions with typed source-row-ID gathers.
  Compare explicit CPU and GPU constructors through one query body.
- Tune CPU byte-radix construction with varying-byte histograms and buffered
  scatter; separate GPU kernel throughput from fresh-constructor wall time.
  Audit storage/cache roles and isolate first-touch, driver scheduling and
  workload-history effects. Preserve historical raw-Metal workspace results
  while moving active ownership, caching and completion to MCCL/MLX.
- Replace per-index host tuple packing with reusable immutable shared relations,
  GPU column projection, selected-value validation, original row-ID generation
  and stable mask compaction. Charge ingestion separately and retain current
  constructor/read/maintenance measurements under their explicit boundaries.
- Express H05, H07-block, D27, and H08 string restrictions as explicit equality
  predicates instead of literal-bound relation occurrences. Keep branch-local
  names, SQL answers, date conditions, and H05's checked order advisory intact.
  This enables predicate-read planning without changing literal-binding semantics
  or any compiler/storage implementation.
- Add H05's checked `advised` order: close the Supplier/customer-nation cycle
  before Nation/Region expansion while preserving the written date-guard
  prerequisites. Keep the authored query, SQL, and default configuration
  unchanged; no compiler or storage changes are required.
- Replace provider-name cases in general Structures checks with recorded,
  trait-checked `supports [retain, expand, keys]` contracts. Infer retained
  cursor types, use the existing `Keys` enumeration ABI, and distinguish
  candidate directories from complete payload layouts. Retyping, target
  catalogs and replay preserve live obligations without stale capabilities.
  Preserve native host dispatch without imposing unused fallback bounds;
  unsupported target decisions fail explicitly instead of disappearing.
  Column markers now prove declaration-order binding through `ColumnSlot`,
  including aliases; codec/layout checks no longer inspect provider spellings.
  Add custom-provider/alias execution and false-contract compilation coverage.

- Broaden the Mac CPU index comparison to all 29 catalog kernels and include
  authored materialization-matched hash/RadixRun pairs automatically: 121
  configurations, checked against DuckDB before and during measurement.
  Preserve the remote H19 string-interning implementation across all three
  local provider configurations.
- Integrate the isolated Mac Metal sorted-index experiments alongside the native
  join optimizations, preserving their source files and historical measurements.
  Add a Mac implementation plan with flat/LSM-like snapshot maintenance, exact
  publication/cursor contracts, and runtime versus compiler-change boundaries.
  This does not add a production Metal provider or revive the removed GPU
  maintained-index backend.

- Optimize RadixRun construction with stable cached numeric sort keys for
  disordered inputs, avoiding repeated borrowed-key dereferences. Add a bounded
  lazy exact membership bitmap without changing payload lookup or default hash
  selection. Verify concurrent membership reads and stable retained cursors.
  Allow `profile --tpch --output DIR` for local macOS profiling artifacts.
- Add controlled TPC-H default/RadixRun/retained configurations and the
  `--index-comparison` benchmark mode, including every stage of grouped kernels.
  Provide matched `built_hash`/`built_radix` configurations where explicit host
  materialization is needed to make the provider change real. Remove H09's
  disconnected Part/Orders prefix: measure local SF1 cardinalities, rank connected
  orders, and retain the faster measured single-key-Lineitem order without
  changing its SQL or Part predicate domain.
- cuda-oxide `299eda41b` / RCCL `1d33079a`: device modules with at least 16
  kernels split into kernel groups whose `opt` and `llc` run in parallel
  (`CUDA_OXIDE_PTX_PARTITIONS`, default available parallelism up to 16); the
  group PTX merges into one module. DOOP: 16 groups, opt+llc 5.7 s -> ~1.2 s,
  all 211 kernels assemble to identical SASS; release build 47 s -> 39 s.
- GPU Datalog holes whose query reads a local of the code around `datalog!`
  (SRD DOOP's `constants`) keep the closure form; a standalone hole function
  cannot capture locals. Free names are checked with `cq_syntax::rust`, callees
  (`sym("…")`) count as items and macros as environment reads. DOOP keeps 154 of
  155 holes standalone; `srd-doop-gpu` (merged from `codex/wt-schedule`) builds
  in 16 codegen units and passes its exact 43-relation test.
- cuda-oxide `1c1c42105` / RCCL `8649af3e`: standalone PTX assembly uses
  `ptxas --split-compile=0` (identical SASS). DOOP cubin assembly 5.53 s ->
  1.40 s; DOOP GPU release build 52 s -> 42 s, cassandra exact.
- GPU Datalog host compile time. Generated holes, strata, phases, relation and
  layout initialization, the initial memory snapshot and output decoders run as
  `#[inline]` closures through partitioned `#[inline(never)]` runners
  (`cq_gpu_sequence_runtime::__codegen`), so each is a separate function in its
  own codegen unit instead of one program function (336,826 unoptimized IR
  instructions; register allocation dominated). Each hole is a separate `fn`
  item taking only the hosts it reads (closures are type-checked with their
  parent: `doop_gpu::run` typeck 21.2 s), and `bind_inputs` receives only that
  hole's bindings instead of all 457. Arena holes no longer construct unused
  output adapters, range builders, dictionaries, status vectors or zero-width
  row/range buffers (each adapter/builder loaded device modules). `doop-gpu`
  uses 16 codegen units; the GPU workspace builds proc macros and their pass
  crates at `opt-level = 3` (`build-override`; they were unoptimized). DOOP GPU
  release build 23 min 06 s -> 52 s (`doop_gpu` crate: expansion 24.9 -> 5.3 s,
  typeck 29.3 -> 4.0 s, host LLVM 1,266 -> 9.1 s; the serial cuda-oxide device
  pipeline, 19.3 s, now dominates). Cassandra exact; GPU evaluation 2.19 s ->
  2.11-2.13 s.
- Rebase the Datalog scheduling/GPU work onto the combined main history.
  Preserve query-scope cache release alongside per-pool stratum scratch
  release, and pin cuda-oxide `e7d4a63ba` / RCCL `d4c95133` without local
  dependency patches. GPU Datalog phase timing is opt-in; DOOP `--gpu-only`
  skips the CPU reference while the default mode retains exact comparison.
- GPU host overhead: arena publication without terminal fences (timing opt-in,
  `GPU-Arena-Timing`), recorded `output reserve_previous` arena protocol that
  starts from the last validated count and reruns on shortfall
  (`GPU-ReservePrevious`), and layout sorts of injective permutations without
  dedup or count readback (`DL-SortKnownUnique`). DOOP GPU 3.47 s -> 3.14 s,
  exact; event synchronizations 66,981 -> 5, stream synchronizations
  24,457 -> 13,672. About 1.0 s of host time between kernels remains outside any
  CUDA call; metadata readbacks plus their syncs bound further batching gains at
  151 ms.
- Delta-first occurrence order (`DL-OccurrenceOrder`): variants record a
  checked permutation of their first positive-atom prefix; the opt-in
  `incremental(driver(delta_first))` advice leads with the delta, then connected
  reads. CPU default stays written order. GPU DOOP selects it: the hot hole
  kernels (cast, virtual dispatch, field load) scanned full `reachable` before
  testing their delta; now they scan the delta. Cassandra exact; GPU evaluation
  9.60 s -> 3.67 s, kernel time 7.72 s -> 1.90 s (cast kernel 4.08 s -> 0.066 s).
- AOT cubins keep LLVM PTX codegen: cubin materialization had forced the libNVVM
  path (35-43% more SASS); now PTX is assembled with `ptxas` at build time.
  Module loading 5.59 s -> 0.12 s; kernel SASS identical to driver JIT.
- GPU workspace builds from pushed forks only: cuda-oxide `mini-linq/main`
  (`496c01bf`, upstream NVlabs main `9df9e7ce` plus 7 patches: RCCL compiler
  support, PTX debug/merge fixes, merged-PTX module cache, static shared
  namespacing, i128 shift legalization) and RCCL `mini-linq/main` (`8e329e25`);
  no local `[patch]` paths. Toolchain Rust 1.99.1 (`RUSTC_BOOTSTRAP=1`) and
  CUDA 13.0, which upstream bindings require. DOOP GPU exact, 11.8–12.0 s warm;
  the first run after a rebuild takes ~25 s while the driver JIT-compiles PTX.
- DOOP on the GPU, exact on cassandra (all 26 outputs equal `mini`). Strings
  never reach the device: the DOOP loader interns `method_descriptor` at load
  (`IStr::code`/`from_code`), `device_storage(symbols(T => Codec))` stores
  symbol columns as `SymbolCodec` u32 codes, closed constants are evaluated on
  the host (`GPU-Capture`, the closed case of `BE-Partial`/`IF-Parameter`), and
  the checked `capture_constants` rewrite turns literal atom arguments into
  guards (`PH-ConstantGuard`). Packed words span multiple 64-bit lanes. Fixes: a
  dynamic u128 lane mask miscompiled by the CUDA backend (constant lane shifts
  instead), stratum-scoped arenas, pool trimming, and an RCCL pool growth reset
  (`8a7e87b`). 11.8 s vs 4.27 s `mini`; kernels 7.7 s, PTX JIT 5.6 s.
- GPU Datalog: whole Datalog programs run on the GPU through the ordinary
  pipeline. `gpu-datalog-runtime` packs fixed-width tuples most-significant
  first (`DL-DevicePacking`) into sorted, unique, disjoint device runs with
  sort/unique/anti-join admission and ratio-4 merging (`DL-DeviceRuns`); GPU
  Sequence reads `DeviceLayout` inputs as runs (`GPU-Runs`) and publishes
  candidates through `translate_arena` (`GPU-Arena`); Round holes gain a
  `batch` interface (host printers reject it); `cq-gpu-datalog` provides
  `device_storage` (`CK-DL-DeviceStorage`) and `print_rounds` (`P-DL-Gpu`).
  Programs on one thread share a device pool (`thread_pool`). Oracle: 13
  programs exact (416 comparisons). 2048-node random graph: TC 0.36–0.49 s vs
  1.58–1.79 s `mini`; same generation 0.41 s vs 3.58 s. The local cuda-oxide
  branch namespaces static shared symbols per merged PTX bundle (`c53311d3`).
- Sorted-run storage, CPU reference for device runs (`DL-RunLayout`,
  `DL-RunAdmission`, realizing `STR-RunRead`/`STR-RunCompact`):
  `RunLayout<K, V>` (immutable (key, payload)-sorted runs, ratio-4 leveled
  merge) and `RunSeen<T, S>` (sort, dedup, anti-join against accepted runs,
  append). Selected by `shard_storage(layouts(runs), seen(runs))`; exact on the
  oracle and DOOP cassandra. Not a CPU default: cassandra 15 threads 3.11 s
  (evaluation 82%, per-run binary-search probes); `seen(runs)` alone 1.73 s vs
  1.28 s hashed.
- Sharded native layouts (`DL-ShardedLayout`): `ShardedLayout<K, V, S>` splits a
  grouped layout map into hash-routed disjoint shards; bulk publication fills
  each shard by one task. `shard_storage(layouts(sharded))` rewrites keyed IDB
  full/delta layouts (checked erasure, `CK-DL-ShardStorage`); unit-key scan and
  EDB layouts are unchanged. `cq_storage::PointRows::new` is public for
  external grouped providers. The parallel DOOP engines select it: cassandra at
  15 threads 1.67–1.76 s → 1.28 s compute, exact; publication 0.61 s → 0.22 s
  (42% → 20% of step time), evaluation now 65%.
- Datalog Round records an evaluation **schedule**: per component, each rule
  hole's positive read set derived from Storage reads and an ordered wave
  partition of its seed and step holes (`WF-DL-Schedule`; `DL-WaveCommute`
  states why waves change no value). Fork records one wave per phase. The new
  `cq-datalog-schedule` Round rewrite (`round = [cq_datalog_schedule::schedule(
  whole | per_hole | by_head | strata(..))]`, `CK-DL-Schedule`) changes only
  that record. The sequential printer evaluates holes in wave order; the Rayon
  printer joins each wave in its own scope and spawns a head's sharded
  admission alongside the next wave once all of that head's holes have run
  (`P-DL-Waves`). The parallel oracle corpus adds a `by_head` configuration;
  DOOP cassandra remains exact at 15 threads.
  The schedule also records per-hole realization `policy` (`grain`,
  `inline_below`; advice `grain(N)`, `inline_below(N)`, `policy(..)`) and a
  derived per-member `publication` wave (`DL-PublishAfterReaders`: after the
  last wave holding a writer or reader). The Rayon printer decides chunking per
  hole and always runs a wave concurrently on a multi-worker pool; a member
  whose publication wave precedes the phase's last wave is admitted and fully
  published in one task alongside the next wave and leaves the phase barrier.
  `schedule(relational)` selects relational progress (`WF-DL-Progress`,
  `DL-AsyncCovering`): after synchronous seeds, each member commits its own
  batches, every delta reader runs once per batch, and holes read under a
  frozen cut. The Rayon printer realizes it through a new step-loop hook with
  an event-driven executor (`P-DL-Relational`) over the runtime
  `RelationalScheduler`/`drive` (`DL-RelationalScheduler`): isolation is
  scheduler gating with draining, host locks are only ever tried, so nested
  work stealing cannot deadlock. The sequential printer realizes it as
  synchronous rounds. Exact on the oracle corpus and DOOP cassandra; ~8%
  slower than synchronous there.
- Integrate the parallel Tasks work with the closed-core layout: key-partition
  advice and the Tasks provider-preserving catalog selection live in
  `cq_sequence_plan::rewrites`; eager-membership host retention moved into
  `cq_structure_selection::build_host_reads`; Tasks conformance links the exact
  `cq_sequence_tasks` and Rayon artifacts from `features/sequence-conformance`.
  Join-filter identity bindings use the scan-local filter index. Catalog
  selection and host retention require ordered payload (Exact and Ordered)
  for Merge reads, so an ordered key set is never chosen for a merge build
  (the refreshed claw/oriented-chain BTreeMap merge registrations under Tasks).
- Share one non-generic Rayon index dispatcher across Tasks pipelines and
  sharded index construction/release. Borrowed tile access and ordered
  per-index result slots keep generated row and closure types out of Rayon
  plumbing; contiguous tile groups retain ordered publication and bounded
  task-local buffers without changing planner decisions.
- Feature lockfiles pin root third-party versions. Feature workspaces share
  `features/target` (workloads keeps its own); CI runs facade consumer targets
  once (`cargo test --workspace`) and `tools/regression-targets.tsv` keeps only
  rows that workspace testing does not cover. Root sources are rustfmt-clean.
- Keep admitted native Sequence providers when a checked retention or retyping
  rewrite is inapplicable, without discarding applicable choices for other results.
  Tasks admits serial lazy providers only at host and upgrades them to shareable
  representations when legal. Workload conformance records the same host-only
  restriction for its `Rc`-backed sorted indexes.
  Candidate directories keep their native representation instead of being
  routed through unsupported sharded protocols. Automatic lookup-first advice
  declines reorderings that would require scanning or retaining an authored
  filtered index; explicit requests still report a checked rejection.
- Sequence records checked, uniform `workers < N` scheduling guards without
  changing support or count semantics. Adaptive native provider plans route iff
  `rows >= low && (workers > 1 || rows >= cache)`; forced plans stay unconditional.
  Positive in-band `Machine`/Tasks `Config` cache cutoffs default to the measured
  6,291,456 rows, not a universal optimum or cache-byte guarantee. Replay records
  both cutoffs and nested guards; no new macro option or translator policy.
  A proved nonexpanding, unshadowed host bound can choose singleton before
  demanding filtered input length. The proof follows actual producer occurrences,
  not just set support; expanding joins and duplicate unions never supply it.
  Pull supplies one worker and target hooks receive an explicit worker-count
  expression; Tasks uses the current Rayon pool's available worker count.
- Recorded singleton provider covers build directly from immutable published
  sources, avoiding the redundant row-part buffer; routed covers keep their
  owned parts and router. Worker count is captured once inside the executing
  pool, and partition-buffer counters reflect only buffers actually allocated.
- Check single-consumer raw staging fusion as `DeferredRows { consumer }`:
  recorded nested worker/cache singleton leaves consume total producers once
  without a raw row vector when reached before the staged count is demanded;
  exact-count fallback or routed demands retain one shared owned buffer.
  Pure named producer views are eligible, but opaque dependencies, shared or
  replayed rows, carrier reheading, prior stores and shadowable join-filter
  sources keep their original publication. PL-Cost retains worst-case owned
  traffic; skipped buffers produce no row-build or retention profiling activity.
- Derive checked Sequence publication ownership once (`BorrowHost`, `Alias`,
  owned rows, or key parts) for PL-Cost and Tasks, preserving source provenance
  across reheading. Real retained key-input copies pay a row-traffic tie-breaker
  after compute span rather than being equated with hash/group evaluation;
  borrows and aliases remain free. Native routed-provider representation and
  stored-key coverage are checked upstream rather than guessed by the printer.
- Tasks applies recorded join filters inside borrowed positional tiles and
  routes borrowed host inputs without an intermediate whole-host vector.
  Key routing groups contiguous chunks into scheduler-fold-local buckets and
  reuses the first owner buffer while preserving per-part occurrence order.
- TPC-H bench dev loop: generated worker sources are stable (no forced
  rebuilds; `compile_unit_ms` is NA for reused workers), per-query crates build
  their compiler proc macros at opt-level 3, `--runs 0` verifies exactness only
  (timing worker, no counting/DuckDB timing), `--config a,b` runs exactly those
  configurations, and SF 0.1 measurement skips the duplicate pre-gate. The CPU
  report collector no longer pins `CARGO_BUILD_JOBS=1`, keeps build logs in its
  cache, and fails if its generated-worker patch anchors drift.
- The facade no longer depends on or re-exports the Tasks feature: generated
  Tasks code names `cq_sequence_tasks::combine` directly and consumers select
  `cq_sequence_tasks_macros::{plan, tasks, translate_tasks}`.
- Join-filter bindings start as identity predicates and are shadowed when their
  covering provider is built, so publications read before that build stay
  exact (IF-JoinFilter `builtBefore`). Routed-provider routers use the stored
  key after constant elision; nested tiles re-apply collectives to joined rows.
- Test infrastructure: sequential machine fixtures use
  `cq_sequence_pull::machine()`, grouped TPC-H tests evaluate each DuckDB oracle
  once per dataset (and all four grouped suites compile again), linked
  fixture artifacts are memoized per process, and stale feature lockfiles are
  refreshed.
- Add checked runtime choices for Tasks provider builds using published input
  length and a machine-supplied threshold, with authoritative force/forbid
  advice. Singleton covers build and read native providers without routing;
  routed covers build directly from row parts and release their providers in
  the pool. Shared H01/H13 sources now carry dbgen key evidence through both
  CPU and GPU pipelines, removing unnecessary complete-valuation canonicalization.
- Extend checked partial grouping to finite-map monoids over globally unique
  valuations. Tasks stream total producer spines into bounded-count local
  accumulator maps and merge them at host; PL-Cost retains key-routed grouping
  as an alternative when merging many groups would dominate.
- Preserve authored relational order and selection stages in the CPU Tasks
  configuration. Contiguous host tiles borrow input rows (positional tiles
  enumerate base-relative ranges); named stream adapters remain inside their
  task through publication, with one buffer per scheduler task. Terminal task
  buffers retain their joined answer iterator instead of making another host
  copy. Task profiling accounts for retained Sequence buffers and native
  canonical sets. PL-Cost charges key routing and bucket publication before
  selecting routed providers; eager indexes keep singleton payloads inline,
  and adaptive membership's immutable fallback keys support shared task reads.
  Named single-use canonical inputs move inside checked group-key partitions;
  global collectives no longer inherit fictitious parallel span from flattened
  contiguous tiles. Primitive work counters remain repeatable; concurrent
  retained-memory peaks are reported as scheduler-dependent observations.
- Restore closed-core release boundaries: shared Sequence transformations now
  live in `cq_sequence_plan::rewrites`, with retyping and host-retention helpers
  in `cq_structure_selection`. Remove the old feature implementation crate and
  migrate direct callers; feature macro names remain unchanged.
  Move CPU target conformance and workload-dependent tests to the external
  `cq-sequence-conformance` workspace, keeping `cq-testkit` language/runtime-only.
  Replace Sequence lowering's self-recompiling checker with direct coverage,
  operator, retention and output checks, accepting arbitrary fresh output names.
  Align feature locks to reviewed root versions; vendor every feature closure
  and canonicalize relocation paths so macOS temporary-directory aliases are
  not mistaken for source escapes.
  Runtime fixtures link their exact Cargo storage/Rayon artifacts instead of
  checkout source paths. Refresh inadmissible handwritten corpus registrations
  without removing queries, configuration definitions or custom negative cases.
- Validate transfer across all 29 registered TPC-H-derived/full-query cases,
  adding real per-stage transfer configurations to the 12 standalone retail
  runners. Preserve query text, SQL, numeric contracts and cached projections.
  Fix H09's catastrophic Cartesian prefix by connecting atoms only within
  written relational runs, without moving predicate/binding/block boundaries.
  Extend checked absorption to redundant left operands, retaining right-support
  pruning precedence and all shared-witness/opaque/negative constraints.
  H21-positive no longer repeats 356,794 membership checks before its exact join.
  All 29 transfer cases match DuckDB at SF0.1/SF1; H09 SF1 completes in 487.8 ms
  instead of reaching the 120-second worker cap. Written-order H09 remains an
  unchanged timeout control. Remove incidental plan-text/operator-count pins;
  retain authored-plan validity, semantic checks and negative certificates.
- Repair the predicate-transfer path: apply checked redundant-support absorption
  after elaboration, preserving the complete VO schedule and final CQ-to-RA check.
  Piped positional read spines now feed one retained snapshot instead of allocating
  intermediate masks; selection narrowing enumerates set bits and retains block
  skipping. Equivalent scan filters share construction and tests only after every
  covering witness is validated, without removing exact joins.
  Predicate-form H05, H07-block, D27 and H08 matched cached DuckDB answers under
  default and transfer at SF0.1 and SF1. Runtime regressions cover snapshot
  allocation, sparse occurrence enumeration and filter construction/probe counts.
  Public API: add `Selection::from_positions` for host-issued IDs; filter emitter
  identities now take a scan-local canonical index rather than a covering operator.
- Legacy GPU Datalog workspace removed.
- Add native `RadixRun` payload indexes with radix12/error32 spline routing,
  full-width integral key encodings, exact bounded search, and stable duplicate
  groups. Opt in with checked `cq_rewrites::radix_runs`; default hash selection
  is unchanged. Wire retained lookup cursors, segmented expansion, join filters,
  and Sequential Exact/Range/Membership capabilities without granting Ordered
  or GPU capabilities. Add an executable backend comparison and native runtime
  benchmarks against binary runs, the backend HashIndex, and hashbrown.
- Retire Datalog-side hooks kept only for the (already broken) legacy GPU
  Datalog bridge: `KeyRepresentation`, `LayoutRealization`, device-owned
  negation keys, frozen/leveled layout selection, `compile_with_ratio`, the
  storage `Representation` and device-set policies (−397 lines). `ddisasm` is
  now a Datalog workspace member with Ascent differential tests. Default
  expansions remain byte-identical.
- Add the GPU `WideCanon` catalog provider after `PackedCanon`: complete
  declared key domains select its unbounded-width capability, while recorded
  codec bounds determine the runtime lane count. Stable per-lane radix passes
  deduplicate complete rows without truncation, using the existing publication
  count and host decode protocol. All packed publication consumers now decode
  arbitrary-width, cross-lane fields.
- Add CQ-Arith: declared fixed-width integer/decimal-coefficient arithmetic
  uses wrapping addition, subtraction and multiplication, plus wrapping integer
  division by a nonzero literal. Total conditionals and explicit lossless
  primitive widening close the grammar. Typed arithmetic bindings and guards
  earn position independence; arbitrary methods, lossy casts, mixed widths and
  runtime divisors stay opaque. CPU and GPU row interpretation share the checked arithmetic
  grammar, allowing aggregate arguments to execute below the host boundary.
- Box 12d: add `R(a,b) key` evidence syntax and checked
  `group_without_canon` advice, with complete-evidence proof cache identity.
  TPC-H loaders assert declared keys only when their projections retain them.
  Key-covered groups consume resident columns and only materialize aggregate
  arguments, not complete named valuations; unadvised groups retain canon.
  With CQ-Arith, exact advised SF10 medians improve H01 from 9,107.255 to
  590.690 ms, H06 from 3,310.908 to 11.776 ms, and H12 from 44.427 to 25.930 ms;
  the unchanged CPU grouped corpus passes all eight tests.
- Import staged GPU-suite programs as their complete authored dependency graph:
  H02/H18/H20 have two stages, H15 has three, H11 has four, and H13 preserves
  its two terminal branches and one-time order preprocessing. Plan each stage
  independently, retain resident numeric publications and key codec bounds
  between GPU stages, and explicitly report host-planned publications when
  device grading is unavailable.
  Sum stage timings and chain independently selected Sequential interpretations
  of the same Physical graphs for conformance.
- Add a native Mac index/join benchmark against pinned PGM/PGM++, RadixSpline,
  compact Hist-Tree, and dense-hash artifacts, plus labeled local SIMD-block and
  Bloom baselines. Compare random, retained-range, sorted-probe, and merge
  processing with explicit preprocessing costs and exact result checks. Record
  a sanitizer-discovered PGM projection-overflow fix as a separate applied patch;
  preserve unsupported artifact domains and machine provenance in the results.
- Mac cost-model experiment: price bulk and independent join-chain probes against
  the built index, including its footprint and live runs. Add explicit per-round
  build-first dependencies and trace reset semantics; do not overlap dependent
  CPU probes with GPU builds. Preserve semi-naive probe-before-absorb behavior.
  Refresh the affected structure rankings and document run-publication constraints.
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
- Add Sequence evidence advice for row counts, keys, foreign keys,
  selectivities, bounds and sortedness. Estimates rank checked plans.
  Keys are author-asserted physical uniqueness premises; false keys can
  change grouped answers through checked canonicalization elimination.
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
  and migrate supported resident callers; unsupported Sequence plans remain
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
  sets). Keyed aHash for adversarial inputs: `hasher(keyed)` (originally paired
  with the since-removed `hasher_round(keyed)`); DOOP engines `mini-keyed` /
  `mini-par-keyed`.
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
