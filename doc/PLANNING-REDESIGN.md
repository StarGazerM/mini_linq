# Planning redesign: one planner, denoted decisions, mechanical lowering

Status: in progress. Driving case: SRDatalog DOOP `HeapAllocHelper`
(`features/datalog/doop/srd/src/generated_default.rs`), an acyclic four-atom
rule with a captured-constant guard that compiled to a written-order cross
product with no reductions (xalan: 2.0e9 pairs expanded, 3.27e6 kept).
Regression target: `facade/mini-linq/tests/doop_heapalloc.rs` and its
per-route siblings listed below.

## Contract

1. **Join planning happens once, at CQ → VO.** Join order, reductions,
   lookup/expand phases and predicate placement are decided by `cq-order` /
   `cq-reduce` and recorded in VO. Elaboration, index policy, physical
   lowering, structure selection, Sequence lowering and every printer are
   refinements: they may choose representations (index kind, probe/merge,
   tile, store) but never reorder, delete or insert joins or move predicates.
2. **Acyclic scopes are output-bounded by default.** The default plan for an
   acyclic body is connected (no join of two operands sharing no column while
   a connected atom is ready), reduced on every join-tree edge (Yannakakis
   bottom-up pass, not filter-gated), and places every recognized predicate at
   the first point its variables are bound. Written order remains available
   as the explicit reference (`Heuristic::Written`) and through `plan(order(..))`.
3. **Superseded by contract 7.** (Was: a closed Rust expression takes part
   in constraint recognition. There is no recognition any more.)
4. **Superseded by contract 7.** (Was: opacity is a recorded property. Every
   Rust expression is opaque and total; there is no other kind.)
5. **Datalog hands its order to CQ.** The incremental occurrence permutation
   that storage layouts follow is passed to each hole's CQ `plan` as explicit
   order advice, so layouts and the hole plan agree by construction.
6. **Docs match code.** `R-Masks` either reduces every inner edge on acyclic
   scopes or stops claiming to be the Yannakakis pass; Plan-Le's composition
   matches the implementation.
7. **One Rust expression; relational predicates have their own syntax**
   (decided 2026-10-28).
   - `if e`, `let p [: T] = e`, `for p in e`: `e` is a Rust expression with
     free variables `fv(e)` (the whole environment when lexically opaque). It
     is opaque, pure and **total by premise**: a panic means the plan is
     wrong, and the compiler does not reason about it. It is placed wherever
     `fv(e)` is bound; there are no Before sets or written-order chains for
     Rust. The compiler never splits, recognizes, rewrites or type-checks it,
     and never prints it differently from how it was written.
   - `where P` is a relational predicate with its own denotation: selection
     by the column representation's equality/order (a stated representation
     law, not a Rust evaluation). `P ::= x == t | x != t | x < t | x <= t |
     x > t | x >= t | x in [t, ...] | x in t..t | x in t..=t | x == y |
     P && P | P || P`. Operands `t` are query variables, literals, or closed
     Rust expressions (`fv ∩ query = ∅`), evaluated once on the host. Only
     `where` yields point/range/membership reads, filter ranks, Masks
     supports, derived domains and join equalities.
   - Atom literals remain occurrence restrictions.
   - Types exist only in the host language. The compiler never inspects a
     `syn::Type` for meaning: no integer/`Decimal`/`String`/`Option`/float
     lists, no wrapping rewrite, no Ord/Eq syntax checks (rustc enforces
     trait bounds). Sizes and encodings come from recorded representation
     facts (codecs, providers).
   - GPU: a row-dependent Rust expression is emitted verbatim and compiled by
     cuda-oxide. It may run on device iff every column it reads has an
     identity device representation (no codec) and its other inputs are
     closed captures; otherwise it runs at host. Codec-encoded columns are
     compared only through `where` (the closed operand is encoded on the
     host).
8. **Storage is one columnar block: encoded payload plus summaries; scans
   prune, then evaluate in the encoded domain, then materialize late**
   (decided 2026-10-28). Sources, read for this decision:
   Data Blocks [Lang et al., SIGMOD 2016, §§3.1–3.4, 4.1;
   https://db.in.tum.de/downloads/publications/datablocks.pdf];
   SMA [Moerkotte, VLDB 1998, §§2–3;
   https://pi3.informatik.uni-mannheim.de/~moer/Publications/MA-98-01.pdf];
   BtrBlocks [Kuschewski et al., SIGMOD 2023, §§2–3; artifact
   github.com/maxi-k/btrblocks `btrblocks/compression/SchemePicker.hpp`];
   FastLanes [Afroozeh & Boncz, PVLDB 16 p.2132, §§1–2.4; artifact
   github.com/cwida/FastLanes]; PFOR/PDICT [Zukowski et al., ICDE 2006,
   §3.1]; tile-based GPU compression [Shanbhag et al., SIGMOD 2022;
   artifact github.com/anilshanbhag/gpu-compression `src/ssb/q11r.cu:70-137`];
   DuckDB (`/data/codex/duckdb-src`: `storage/table/column_data_checkpointer.cpp`,
   `row_group.cpp` CheckZonemap, `compression/{bitpacking,rle,dict_fsst}.cpp`,
   `execution/operator/join/physical_hash_join.cpp` join filters).
   - **Block = payload + summaries, built together once** (at load or
     checkpoint) per column: codec id, packed payload, min/max summary (SMA),
     optionally a positional summary (PSMA). A summary is never computed over
     a different, decoded copy of the data.
   - **Codec chosen per column block by measured size** (DuckDB analyze or
     BtrBlocks sampling), recorded as a representation fact; uncompressed
     when compression does not pay. No loader-wide fixed codec.
   - **Scan order is fixed:** (1) prune blocks by summaries before touching
     payload; (2) evaluate predicates in the encoded domain where the codec
     allows it (constants translated once into the code domain on the host;
     FOR residual comparison, dictionary/RLE value-once tests), otherwise
     decode one vector/tile into cache/registers fused with the predicate;
     (3) produce positions; (4) materialize only the columns and rows still
     needed. No whole-column decode, no decode into owned row tuples.
   - **Decode to the smallest processable type** (FastLanes §1): carriers
     and lanes follow the representation's width, not a uniform `i64`.
   - **GPU:** decode cooperatively per tile inside the query kernel (artifact
     `binpack_kernel.cuh`), never per-row random access in the hot loop;
     per-vector summaries computed at encode time are kept and used to skip.
   - **Join filters** (build-side min/max or exact keyset) prune probe blocks
     through the same summaries on every target.
   - **Read in place:** scans and index builds borrow block storage; a copy
     is made only for the reasons recorded in the representation (source
     not retained, mutable between rounds, crosses a device boundary,
     computed key).
   - Every one of these is a recorded representation fact the compiler plans
     with (contract 7: no type inspection; sizes and encodings come from
     representation), never a hidden runtime or loader choice.
   - Detailed design, semantics placement, data structures, compiler support
     and merging of compressed indexes: `doc/STORAGE.md`.

## Inventory of mislayered decisions (audit 2026-10-27)

| Layer | Decision made there today | Disposition |
|---|---|---|
| `cq-syntax guard::term` (retired) | Rust recognition and field-path parameters | replaced by explicit `where` and query-closed `Term::Closed(syn::Expr)` (contract 7) |
| `cq-order Written` | identity sequence, cross products kept | keep as explicit reference; new default (contract 2) |
| `cq-reduce Masks` | reduce only filter-ranked subtrees, never the root | fix on acyclic scopes (contract 2/6) |
| `cq-elaborate` | predicate binder choice, branch join topology | record in VO or keep as pure refinement |
| CQ `normalize` | computed head becomes `Bind` | retain original Rust; placement uses free variables, not expression classification (contract 7) |
| `cq-physical-lowering` | probe first / eager / point-range / Let vs Pipe | representation only; allowed |
| `cq-sequence-plan lookup_first` | reorders and deletes joins after lowering; freezes on any Bind/Guard | demote to representation (retained leaves, providers); ordering comes from VO |
| `cq_planners::order(le)` | Lookup = dropped-column subatom, phase not recorded | fold into the default planner with recorded phases |
| GPU emitter | tile defaults, virtual-range publication, retry protocol | representation; document |
| Datalog `fork` | independent CQ plan after incremental permutation | pass order advice (contract 5) |

## Architecture (replanned 2026-10-28)

One rule organizes every contract: **each decision lives in exactly one IR
as a recorded, checked, printable fact; every later stage consumes it; laws
sit where a representation is introduced.**

| Layer | Owns (decisions) | Semantics / laws | Contracts | State |
|---|---|---|---|---|
| CQ surface | atoms, `where` relational predicates, opaque total Rust `if/let/for` | set denotation; `where` = selection by representation Eq/Ord; Rust placed by free variables | 7 | implemented; expanded-Rust audit in progress |
| CQ → VO → RA (planning) | join order, reductions, **Lookup/Expand order and Expand placement** (L&E §4), block placement | VO/RA refinement of the written order | 1, 2, 5 | order + Datalog advice done; L&E still lives in Sequence `lookup_first` (P3) |
| Structures (representation) | provider role (scan column vs index run), codec, summaries, grouped runs, copy vs read, join filters (exact / min-max / Bloom), positions | α abstraction laws: decode, enclosure, factorization, bounded-error directory, preserved ops | 8 (`doc/STORAGE.md`) | designed; type-blind provider dispatch partly implemented |
| Sequence + grading (execution shape) | tiles aligned to vectors/blocks, deferred ranges from stored offsets, launch/scan/fold/sync facts, adaptive choose from evidence | positions and sizes; BE-* rules | translator-decides-nothing, 7 (representation facts) | launch facts done; storage-aware shape open |
| Translators (pull, Rayon, CUDA, Metal) | nothing | per-construct correctness | T-Pull/T-Tasks/T-Gpu | audit violations fixed; anchors pending |
| Runtime (cq-storage, cq-load, device-columns, MCCL) | implements recorded representations | storage laws, tested | 8 | AdaptiveKeySet/ZoneRows off-design; RadixRun on remote |
| Datalog maintenance | runs, tail/frozen blocks, merges, ratio, dictionary policy | STR-Run*, DL-DeviceRatio | 5, 8 §5 | runs exist (plain words); compressed/grouped open |

Evidence/advice (counts, keys, selectivities, ratios) never carries
semantics: it chooses among checked candidates.

## Plan (replanned 2026-10-28)

### Gates

Refactors are gated by code identity, **not a timing tolerance**:
`python3 tools/expanded_gate.py --baseline <rev> --targets all` compares every
TPC-H catalog query and the migrated library kernels. The JSON report and table
in `target/expand-gate/report` distinguish `identical`, `runtime-changed`, and
`differs`; every changed hunk needs an exact reviewed rule supplied through
`--classes tools/expanded_gate_classes.json`, otherwise it remains unexplained.
The ledger separates a–f contract-7 refactors from g–l intended changes and
explicitly defined additional classes. Intended changes require execution
evidence, not an identity claim. The module docstring specifies normalization
and the classes-file format.

Cache keys include normalized Rust, linked runtime sources, toolchain/build
inputs, and data/oracle manifests. Identical targets reuse verified exactness
and counter outcomes without compilation/execution; runtime changes require
fresh evidence. `--verified-results` imports explicit results with hashed
evidence, or `--verify-command` runs a shell-free verifier. The default gate
only expands Rust and never runs timed benchmarks.
`--classify-report <report.json>` updates only the captured snapshot's
classifications without expansion or execution; it does not validate a later
working tree. Its table distinguishes identical, refactor-only, intended and
unexplained changes while retaining runtime-hash and evidence requirements.

Full changes also pass root + feature workspaces, `tools/test_regressions.sh`,
GPU suites, TPC-H SF1 default (29 exact), H05 GPU SF1/SF10 both modes,
DOOP GPU xalan/batik counts and time, and `doc/formal/check.py`.

### Steps

- **P0 Close current work.** Finish the expanded-Rust audit (generated Rust
  identical to HEAD except recorded classes), full gates, commit.
- **P1 Governance.** Design anchors in code (hard comments citing the
  contract, sources and forbidden moves) registered in a table that
  `check.py` verifies; this file + `doc/STORAGE.md` are the single sources.
- **P2 Merge remote** `origin/codex/all-answers` (RadixRun, RadixSpline,
  MCCL, Mac work, provider-contract refactor), reconciling on the type-blind
  design.
- **P3 Lookup/Expand (L&E) without a new IR** (revised 2026-10-29 after
  reading Birler/Kemper/Neumann PVLDB'24 in full). The paper's result laws
  reduce to flat RA (`e(R→S) ≡ R⋈S`, `head(R→S) ≡ R⋉S`, lookup reordering is
  semijoin reordering), so VO/RA already express L&E's meaning: a lookup is a
  subatom that binds nothing (E-Check, `⋉`), an expand is the full subatom
  (B-Join, `⋈`); Plan-Le/CK-Le check it. What the paper adds is operational
  and lives where match sets are values: reuse of the lookup's match set for
  the expand (Structures ST-Share/STR-Witness; Sequence buckets, BE-Bind),
  iterators stored in builds (Fig. 4), and per-tuple smallest-side
  intersection (Physical `probe smallest`, generalized; Expand3).
  DOOP evidence (offline, final relations; `local://doop-skew-*`): 6–12% of
  rules are α-cyclic; keys are heavily skewed; smallest-side intersection
  saves 4.8×/15.4× on generic AIPT_Store and 3.4×/2.4× on array load only if
  placed early; ~1× on virtual dispatch and Assign_Param.
  - **P3.1 Checked advice for every L&E decision** at its owner layer:
    step-level VO order advice (`R@k(b̄)` lookup, `R@k` expand, adjacent
    fine subatoms as intersection points; WF-VO + CK-Le), k-way
    `smallest(...)` Physical advice, existing `retain_lookups`/`join_filter`
    Structures advice. Plus counters at each fused expansion (GPU
    `emit.rs` fused expansion, CPU pull printer) per rule/delta/stage.
  - **P3.2 Hand-advised experiments** measured with counters and paired
    timing: DOOP generic AIPT_Store, DOOP array load, TPC-H H05 plan R.
    Also measure the existing CPU `join_filter`/Bloom advice there (rows
    removed before lookup). Building filters during the hash build, bitmap
    vs Bloom choice and small-to-large filter order stay in P7; revisit
    after P3.2 whether to pull them earlier (decided 2026-10-29).
    **Results (2026-10-30;** `experiments/gpu/compare/h05_step_advice.results.json`,
    `experiments/gpu/compare/p32-doop-advice.json`, `experiments/gpu/README.md`):
    - H05: the CQ order of plan R alone does not explain R. Preserved step R
      is 124 ms at GPU SF10 (automatic 68.7, authored R recipe 8.6) and 10x
      slower on CPU SF1 (retained memory 5 MB → 133 MB). R's win is its
      Sequence schedule: a retained Customer prefix and segmented
      Orders/Lineitem fan-out; a 1-row Region root tile has no parallelism.
    - The automatic Sequence planner (`plan(machine)`, `lookup_first`)
      re-plans authored CQ orders. On DOOP, decomposition-only beats the
      default by 1.070x [1.064, 1.124] (xalan) and 1.234x [1.231, 1.238]
      (batik), all 40/40 exact.
    - GPU `probe smallest` was silently emitted as sequential range
      enumeration + membership; it is now rejected (T-Gpu) until P4 ranges.
    - DOOP AIPT_Store is 2.6% of the traced batik fixpoint: skew-heavy rules
      are not where fixpoint time goes; per-hole time from the counters must
      choose targets.
  - **P3.3** Default planner emits L&E phases from evidence; delete
    `lookup_first`/automatic Sequence re-planning first (measured win on
    DOOP); grading learns R's schedule shapes (retained small prefixes,
    segmented fan-out) from recorded evidence;
    `lookup_first` and EXT-SeqLookup; GPU match-set carrier after P4.
- **P3b Profile the engine gaps** (`doc/STORAGE.md` §12.4): GPU h06
  reduce/dedup at 16–74× a hand kernel; CPU h01 at ~140× a String-grouping
  microbenchmark. Profiles (perf, nsys/ncu) decide whether these come before
  the storage slices.
- **P3c Engine-wide storage rules** (`doc/STORAGE.md` §13): intern strings
  at input for the CPU catalog as on GPU; sort-based dedup (self: radix
  sort + unique; against full: merge with sorted runs) replacing hash `Dedup`
  sets in CQ printers and DL-InsertNew in CPU Datalog; remove V decode sites
  (`doc/DECODE-AUDIT.md`), measured per target.
- **P4 Index runs.** RadixRun as the index-role provider: one order-
  preserving key law (merge `KeyCodec`, `DenseOrdinal`, `RadixKey`), narrow
  offsets, columnar items, key predicates as group ranges, membership/count/
  semijoin without items, retire AdaptiveKeySet staging/`Arc`; Datalog runs
  become grouped runs merged by merge-path. The dictionary policy is decided
  here from measurements (`doc/STORAGE.md` §10): code assignment is a
  publication, aligned with round/load boundaries.
  Memory placement is a per-target fact (`doc/STORAGE.md` §11): shared byte
  layout and laws; CUDA and Metal differ in allocation, copy reasons and cost.
- **P5 Scan columns (CPU).** Measured rules only (§12.4): plain default;
  byte truncation for scans; bit-packing only for bandwidth-bound parallel
  scans; encoded execution on dictionary codes first. Block format, codec trait, summaries built with
  payload, columnar loader; prune → encoded predicate (preserved-op law) →
  positions → late materialization; STR-Blocks / BE-Blocks rules.
- **P6 GPU storage** (compression for transfer/capacity first; resident
  scan codecs only where measured). Device blocks with per-vector summaries, tile decode
  fused with predicates, narrow carriers, summary skip; GPU Datalog runs as
  grouped runs.
- **P7 Join filters and evidence** across targets (exact min/max and
  membership from runs; published key/row/group statistics for `choose`).
- **P8 Remaining items.** Key-partition routing projection, placement spine
  (only when a branching machine appears), Masks as full reducer (a cost
  question), delta=k SRDatalog plans.

## Progress

- P2 remote inventory: reviewed the messages and `git show --stat` for all
  nine remote-only commits (`9426e27b`, `33e1ffa6`, `eebf5498`, `2698fc8f`,
  `c142c37d`, `477ef1ba`, `90ecf46d`, `42955248`, `fbd35f5a`). The four
  historical merge commits carry earlier mainline integration; their inherited
  changes are reconciled against merge-base `43d22720`, not reapplied over the
  newer local contract-7 and input-interning changes.
- P2 Mac/Metal and MCCL: retained the calibration/cost-model experiments,
  indexing measurements, Metal providers, source-bound compiled H09 example,
  and upstream MCCL collections. These remain in their isolated
  `experiments/mac` workspaces rather than Linux workspace membership.
  Historical Mac measurements are retained as historical evidence, not claimed
  as measurements of the merged tree. Storage §11 remains authoritative:
  shared provider laws do not imply identical CUDA/Metal placement or costs.
- P2 storage/runtime: retained RadixRun/RadixSpline grouped providers and the
  indexed-column/retained-read dispatch used by source-bound GPU RadixRun H09.
  Combined their explicit provider support with local `DenseOrdinal`/`KeyCodec`
  trait dispatch and membership inference; no provider-name recognition is
  needed to select retained/expand operations. These are the plain grouped
  index-run capabilities described by storage §8, not a change to scan layout.
- P2 compiler/contracts: combined explicit `supports [retain, expand, keys]`
  obligations with typed host reader references, recorded membership/layout
  facts, and native catalog identity preservation. Catalog selection retains
  local representation-preserving order while checking live support; packed
  canon bounds remain runtime codec facts rather than Rust scalar-type
  inspection. Formal rules and fixtures retain contract 7's opaque Rust premise
  and remote provider support requirements.
- P2 workloads/publication: reconcile remote radix configurations and grouped
  query paths with input-time `StringCode` kernels and interned test fixtures,
  retaining the remote connected/cycle query orders. Relational operators keep
  codes; opaque Rust reads and publication retain the registered DECODE boundary.
  The local decode registry, design anchors, storage rules and expanded-code
  identity gate remain acceptance obligations for the integrated tree.
- P2 GPU boundary reconciliation: native input handles require the explicit
  `Resident` catalog capability before descendant reads; ordinary pipe-host
  adapters still require retention, and all type/element/read/level checks
  remain mandatory (`PL-Resident`). The GPU Structures stage validates target
  support instead of inheriting CPU batching; GPU scheduling stays in Sequence,
  and unsupported authored stages/witnesses/filters remain rejected. GPU answer
  origins now follow positive relational AST atoms, never negation or opaque
  Rust token occurrences; compact-shell and closed-dictionary tests reflect
  contract-7 normalization and one-time opaque literal evaluation.
- P2 validation complete: root tests passed 1,173;
  workload tests passed 3,732; all 119 regression inventory entries passed;
  seeded oracle fuzz passed 200/200. The 28 public API snapshots pass after
  reviewing updates to `cq-lang-sequence`, `cq-lang-structures`, `cq-load`,
  `cq-print`, `cq-sequence-plan` and `cq-storage`; dependency, formal and root
  formatting gates pass. The expanded gate passes all 46 targets with imported
  fresh exactness/counter evidence: 729 emitted hunk occurrences have 684 exact
  reviewed `merge-remote` rules and zero unexplained hunks. CPU timing used five
  paired observations per target. Seven numerical CIs remain wholly above 1.0:
  H14/H16/H20 and `aggregate_two_keys`, `correlated_scalar`,
  `surface_disconnected`, `surface_let_sum`. The accepted static classification
  is binary code-size/layout effects, not changed default-query work: remote
  configurations add modules; default expansions add only empty trait witnesses
  and binding/return wrappers around the same providers. Hot hash/runtime
  implementations are unchanged, counters agree, and the four library entry
  instruction sequences agree after relocation normalization. Numerical CIs and
  the rejected empty-wrapper trial remain recorded without relabeling them as
  timing passes; no further CPU timing is required for these unchanged defaults.
  Evidence: `target/p2-gate/cpu/static-classification.json` and `final-report.json`.
  GPU acceptance passed 192 tests (two optional tests ignored), 52/52 SF1
  query/mode combinations and all four H05 scale/mode combinations. H05 SF10
  automatic/advised totals are 68.567/8.603 ms versus 112.1/8.64 ms references.
  Xalan and Batik each match all 40 SRDatalog relation counts; outer evaluation
  totals (6.888/11.856 s) include setup/output and are not fixpoint-only clocks.
  The final core/workload/regression/fuzz/API/dependency/formal/format gate pass
  succeeded. New provider configurations passed 92 SF1 DuckDB exactness checks
  across all 29 CPU targets (29 radix, 29 radix-retained, 17 built-hash and
  17 built-radix), in addition to 29 default checks. Formal inventory is
  314 rules, 368 conformance records, 22 anchors and 519 decode-site markers.

- Contract 7 documentation cutover: finite-set pure-total denotation, explicit
  `where`, free-variable Rust placement and recorded device representations.
  New rules are `CQ-Where`, `CQ-Closed`, `WF-VO-Rust` and
  `BE-RustRepresentation`; the old recognition/partiality rules are retired.
  Conformance updates are marked unverified; no checks were run for this
  documentation change.
- GPU TPC-H representation repair: manifest dictionary inputs declare published
  symbol codes, never `String` device columns. Explicit authored
  `dictionary_where` relations evaluate a pure predicate once per dictionary
  entry and publish immutable membership codes; the compiler does not split
  opaque Rust. Missing relational literals cannot insert dictionary entries.
  Decimal and Date retain their transparent scalar identity layouts and
  authored Rust methods; runtime `Scalar` dispatch supplies lane encoding,
  reconstruction for Rust reads, and layout widths without inspecting type
  names. Authored type annotations on computed bindings record result identity;
  unannotated bindings stay on the host. Nonnullable staged publications retain
  their recorded device layouts instead of downloading and re-uploading codes.
  Dictionary decoding belongs at final host answer publication, not relational
  joins or staging. Acceptance measurements are reported separately.
- Hoisted CPU sharded-router closures retain the recorded key tuple's declared
  type tokens and ownership. This supplies inference before the provider-choice
  match without interpreting types; inline routers retain contextual inference.
- Phase 1 done: fixtures on the core (`facade/mini-linq/tests/doop_heapalloc.rs`),
  Datalog (`features/datalog/tests/counters.rs`) and GPU
  (`experiments/gpu/datalog/facade/tests/heapalloc.rs`) routes.
- Historical phase 2 used field-path parameters under superseded contract 3.
  Contract 7 instead stores arbitrary query-closed Rust expressions; default
  order remains `Prefix(Connected)/Coarse`.
- Phase 3 done for ordering; Masks decided docs-side (contract 6): Masks is
  the bottom-up Yannakakis pass restricted to filter-carrying subtrees, not a
  full reducer; `passes.tex`, `STATUS.md` and `cq-reduce` now say so, and
  `Tree` remains the unconditional full reducer. Reducing unfiltered
  key/foreign-key edges is a separate cost question, not a correctness fix.
- Contract 5 done: `plan(Label, ..)` marks its incremental variant
  `authored`; Fork passes that order to the hole's recorded planning owner as
  `order(..)` over normalized clause positions. `Hole.plan` records the named
  phase index; custom queues declare it and conflicting arguments are rejected.
  Tests: `fork/tests/correspondence.rs` (advice and conflict rejection),
  `tests/counters.rs` (renamed planner executes authored cross-product order).
- Contract 4 measured, reverted, cause **not established**. Classifying
  head copies by `Analysis::opaque` instead of `Bind`/`Guard` syntax lets
  `lookup_first` rewrite every recursive DOOP hole. Measured: xalan fixpoint
  2.20 → 9.35 s, batik GPU out of memory; profile kernel total 1.21 → 7.11 s,
  with new time in `gather_encode`, `numeric_bounds`, radix sort,
  `keys_and_rows`, `sequence_decode_column`; dumps gain 12 stored
  `__lookup_forward*`, 16 more `__lookup_provider*`, 8 `store flatten`.
  Which plan nodes launch those kernels, the OOM cause, and whether the
  one-block emitter fallback (`SEQUENCE-PLAN.md` [6]) is involved were not
  checked; attribute kernels to Sequence definitions before concluding.
  Lookups themselves never store (they are pipelined `compact … lookup`).
  Two checker/emitter disagreements it exposed are fixed regardless:
  BE-Resource and the GPU emitter now use backward liveness that removes a
  map's written columns, and both carry a lookup key that stays live after
  its flatten as a parent instead of re-reading it from the provider row
  (`cq-lang-sequence/src/resources.rs`, `experiments/gpu/sequence/core/src/emit.rs`).
- GPU boundary rules added with the fixture (target representation only):
  a scalar `!=` selection against a row variable is row work and a device
  publication, never a host descriptor alias (`placement.rs`); a run-derived
  computed provider is built from its materialized rows and measures its own
  key bounds, because `DeviceLayout` supplies only primitive type ranges
  (`experiments/gpu/sequence/core/src/rows.rs`, `sequence-runtime`);
  catalog matching accepts wildcard native tokens and preserves concrete
  native keyed layouts (`cq-sequence-plan/src/rewrites/{catalog,lookup_first}.rs`).
- Open: `lookup_first` demotion (then revisit contract 4), and the `delta=k`
  SRDatalog plan translation that regressed batik.
