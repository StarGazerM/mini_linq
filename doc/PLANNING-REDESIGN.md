# Planning redesign: one planner, denoted decisions, mechanical lowering

Status: in progress. Driving case: SRDatalog DOOP `HeapAllocHelper`
(`features/datalog/doop/srd/src/generated_default.rs`), an acyclic four-atom
rule with a captured-constant guard that compiled to a written-order cross
product with no reductions (xalan: 2.0e9 pairs expanded, 3.27e6 kept).
Regression target: `facade/mini-linq/tests/doop_heapalloc.rs` and its
per-route siblings listed below.

Audit baseline: `7e187597` (2026-11-02). Dated plans, profiles and Progress
entries below are an append-only evidence history, not simultaneous descriptions
of the current tree. Numerical observations without a more specific artifact
are sourced to this document at that commit; they are not new measurements.
The direct-call cutover (`60622956`, 2026-11-01; `0c031b1d`, 2026-11-02)
removed autoref tiers, `build!` dispatch and `AutoDense`. Host read declarations
and per-reader Structures facts now select direct calls. Capture ownership is
plain value cloning per consuming move closure (`434b2d14`, 2026-11-02), not
`Arc` handles. `cq_order::compile_written` owns the written reference;
`cq_relational_lowering::compile` lowers it. Datalog negative provider/hash choices live
in Storage (`f7511375`, 2026-11-02), not a `hasher_round` rewrite.
P4-GPU slice D replaces the baseline's `DeviceRuns`/packed `DeviceLayout`
protocol with ordinary full-key `RadixLayout` sets backed by `DeviceRadixRun`.
The read-only `fresh` / separate `append_sorted` boundary is preserved;
the dated acceptance below records exact gates and matched paired measurements.

## Contract

1. **Join planning happens once, at CQ → VO.** Join order, reductions,
   lookup/expand phases and predicate placement are decided by `cq-order` /
   `cq-reduce` and recorded in VO. Elaboration, index policy, physical
   lowering, structure selection, Sequence lowering and every printer are
   refinements: they may choose representations (index kind, probe/merge,
   tile, store) but never reorder, delete or insert joins or move predicates.
2. **Connected ordering with Masks reduction by default.** An acyclic body
   uses connected ordering (no join of two operands sharing no column while
   a connected atom is ready). Masks reduces non-root edges whose child
   subtree carries a filter (`node != root && ranks[&child] < 3`); it is
   not a full Yannakakis reduction or a general output-bound guarantee.
   Every executable VO independently checks readiness (I1), earliest ready
   nonintroducing checks (I2), and connectivity (I3), including authored orders.
   I2 drains checks before any introduction, including a let; simultaneous
   filters may retain their relative order. Written and `plan(order(..))`
   constrain introducing atoms, not late checks, and reject illegal I1/I3
   orders. Written is exempt only from I4's heuristic tie-break obligation.
3. **Superseded by contract 7.** (Was: a closed Rust expression takes part
   in constraint recognition. Contract 7 forbids that recognition; remaining
   implementation violations are tracked below, not claimed eliminated.)
4. **Superseded by contract 7.** (Was: opacity is a recorded property. Every
   Rust expression is opaque and total; there is no other kind.)
5. **Datalog ownership is a staged cutover.** Datalog owns stratification,
   seed/recursive variants, delta identity and admission. In step 1 it retains
   `delta_first_order` and its recorded occurrence permutation; Storage still
   selects layouts from that Incremental prefix. CPU `WrittenOrder` and GPU
   `DeltaFirst` remain distinct structural inputs. These facts reach the CQ
   planner as `PlanOptions` `ordering(...)` inputs (not CQ grammar) and VO
   records them, rather than being inferred by a printer; I2 drains fully-bound
   checks without changing the introducing-atom order. Optional stratum shared
   demand is a recorded no-op in this step. Step 1b, after its own census,
   removes the permutation planner and makes Storage follow recorded VOs.
   Until that cutover, contract 1's single-owner goal is not a claim that
   Datalog's pre-VO ordering and layout selection have already been removed.
6. **Docs match code.** `R-Masks` is the filter-gated, non-root reduction
   policy, not the full Yannakakis pass; Plan-Le's composition matches the
   implementation.
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
     supports, derived domains and join equalities. Operands at a column
     declared with a code domain (`c: T in (D)`) are translated once on the
     host by A-Domain into captures `::cq_storage::domain::{exact,lower,upper}`
     of `D`; strings are always interned, so string equality/order is code
     equality/order within one domain (WF-Domain). `where` has no pattern
     operator: contains/LIKE are load-time `dictionary_where` code sets.
   - Atom literals remain occurrence restrictions.
   - Types exist only in the host language. The compiler never inspects a
     `syn::Type` for meaning: no integer/`Decimal`/`String`/`Option`/float
     lists, no wrapping rewrite, no Ord/Eq syntax checks (rustc enforces
     trait bounds). Sizes and encodings come from recorded representation
     facts (codecs, providers). Structures input columns record
     `codec_id: Option<CodecId>` beside the opaque codec type; `None` is
     identity. The declaration boundary interns omitted ids once by opaque
     token spelling: the same automatically interned spelling shares an id,
     while explicit different ids remain distinct even for the same spelling.
     Logical declarations likewise carry `cq_syntax::LogicalTypeId(pub u32)`;
     relation syntax is `column: Type [id] in [domain] (expr)` and provider
     columns use `[id] Logical => Storage codec [id] Codec` (ids optional).
     Declaration interning preserves explicit ids and shares omitted equal
     spellings once; subsequent codec lookup uses logical ids, not Rust types.
     Source syntax accepts `codec Type` or `codec [0] Type`. Later checking,
     GPU compatibility and catalog admission compare recorded ids, never
     codec types; the type is only retained for printing. Catalogs provide
     identity by default, not syntactic `Dense` authorization for local ids;
     custom codecs require explicit matching catalog declarations.
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
| `cq-order Written` | identity sequence, cross products kept | explicit reference retained; Connected/Coarse facade default done (contract 2) |
| `cq-reduce Masks` | reduce only filter-ranked subtrees, never the root | retained default policy (contract 2/6); P8 measured the full-reducer cost below: unconditional Tree loses overall; an evidence-only selective proposal remains unimplemented |
| `cq-relational-lowering` (was `cq-elaborate`) | predicate binder choice (branch join topology deleted) | done: predicate placement recorded in VO as `restrict R@i where P;` by `cq-order` (O-Restrict, order-independent H-Local shared with H-FilterRank); reducers rank by recorded restrictions; lowering (RL-*) applies only recorded decisions; a hoisted block's finite domain is the VO prefix binding its free columns; RA rewrites live in `cq-ra-rewrites` |
| CQ `normalize` | computed head becomes `Bind` | retain original Rust; placement uses free variables, not expression classification (contract 7) |
| `cq-physical-lowering` | probe first / eager / Let vs Pipe | representation choices only; point/range supply and join keys are consumed from matched, checked IX requirements |
| Sequence decomposition | preserves recorded relational construction | P3.3-lite removes automatic replanning; ordering belongs to VO |
| `cq_planners::order(le)` | Lookup = dropped-column subatom, phase not recorded | fold into the default planner with recorded phases |
| GPU emitter | tile defaults, virtual-range publication, retry protocol | representation; document |
| Datalog `fork` | authored incremental permutation reaches CQ as explicit order advice | done for authored order (contract 5); argument correspondence remains an open issue below |
| printers/cq-syntax (closed by InternedStrings) | string-literal branches (`StringLiteral`, `relational_string_eq`, string bound forcing a scan) | deleted; one typed form, operands are A-Domain captures |
| `algebra::disjoint_unions` (closed by InternedStrings) | literal-value union disjointness proof | option (b): deleted; only fallback construction proves disjointness (pending paired measurement) |
| `cq-ext-aggregate constant_group` (closed by InternedStrings) | `Expr::Lit` initializer shape | keyed on A-Capture-recorded top-level lets |
| GPU `capture.rs` (closed by InternedStrings) | `Expr::Lit` exemption from closed extraction | removed; literals are ordinary closed captures |
| Datalog `fork` (closed by InternedStrings) | literal key embedding in filtered occurrences | every negative-key filter is `where v == (e)` after the scan |

## Architecture (replanned 2026-10-28)

One rule organizes every contract: **each decision lives in exactly one IR
as a recorded, checked, printable fact; every later stage consumes it; laws
sit where a representation is introduced.**

| Layer | Owns (decisions) | Semantics / laws | Contracts | State |
|---|---|---|---|---|
| CQ surface | atoms, `where` relational predicates, opaque total Rust `if/let/for` | set denotation; `where` = selection by representation Eq/Ord; Rust placed by free variables | 7 | implemented; expanded-Rust audit in progress |
| CQ → VO → RA (planning) | join order, reductions, **Lookup/Expand order and Expand placement** (L&E §4), block placement | VO/RA refinement of the written order | 1, 2, 5 | order + Datalog advice done; Sequence replanning removed (P3.3-lite); GPU TPC-H and CPU TPC-H evidence done (P3.3e); per-round DOOP counts and CPU retained-prefix grading open |
| Structures (representation) | provider role (scan column vs index run), codec, summaries, grouped runs, copy vs read, join filters (exact / min-max / Bloom), positions | α abstraction laws: decode, enclosure, factorization, bounded-error directory, preserved ops | 8 (`doc/STORAGE.md`) | designed; type-blind provider dispatch partly implemented |
| Sequence + grading (execution shape) | tiles aligned to vectors/blocks, deferred ranges from stored offsets, launch/scan/fold/sync facts, adaptive choose from evidence | positions and sizes; BE-* rules | translator-decides-nothing, 7 (representation facts) | launch facts done; storage-aware shape open |
| Translators (pull, Rayon, CUDA, Metal) | nothing (target contract) | per-construct correctness | T-Pull/T-Tasks/T-Gpu | anchors registered in `DESIGN-ANCHORS.tsv`; recorded launch/realization cutover done, remaining recognition and runtime-policy violations open |
| Runtime (cq-storage, cq-load, device-columns, MCCL) | implements recorded representations | storage laws, tested | 8 | AdaptiveKeySet/ZoneRows off-design; RadixRun implemented and exported locally |
| Datalog maintenance | runs, tail/frozen blocks, merges, ratio, dictionary policy | STR-Run*, DL-DeviceRatio | 5, 8 §5 | runs exist (plain words); compressed/grouped open |

Evidence/advice (counts, keys, selectivities, ratios) never carries
semantics: it chooses among checked candidates.

### Declared dataset input and external planner records

`cq_evidence` owns the versioned JSON document and its strict checker
(`doc/planner-evidence.schema.json`). Numbers retain source, claim kind,
provenance, units and dataset hash. HLL observations are not deterministic
bounds; absent facts remain unknown. Dataset identity hashes the canonical
schema/input manifest, not a pathname. Exact bag counts are not silently
relabeled exact set counts.

CQ and DL-Source carry a separate optional `declared_evidence` attachment:

```text
declared_evidence "<checked evidence JSON>" {
    R = "dataset_relation" full [a = "dataset_column_a", b = "dataset_column_b"];
}
```

In CQ this follows the struct name and precedes declarations;
in Datalog it follows its separate key evidence and precedes `output`. Explicit
relation/column mappings survive normalization, Fork's layout/delta/scan
projection and parallel scan rebinding. `full`, `delta`, and `subset` record
the view, not an equality between its statistics and the full dataset.
Source/Fork/VO correspondence rejects lost or changed attachments.
Iteration scope is checked at its owner: EDBs cannot claim `recursive_final`,
and intrinsically recursive IDBs cannot claim `frozen`. Source permits an
acyclic IDB to be iterated by a later union of SCCs; Round then checks its
actual component. Components with steady work require `recursive_final` and
the exact zero-based component index in `recursive.stratum`; seed-only
components require `frozen`. The file cannot invent an iteration scope.
`cq_load::stats_manifest::load_build_input` reads an explicitly named JSON file
and registers its canonical path with Cargo's `rerun-if-changed`. The caller
supplies the returned neutral document to `DeclaredEvidence` with its explicit
compiler bindings; compiler syntax, IR parsers and runtime printers never read
an evidence file.

This checked document is the **only VO count/key input**. Without an explicit
order, Plan-Prefix reads equality row counts for full input bindings and
surviving key declarations through the recorded correspondence, applying the
existing O-EvidenceLe root/L&E rule and its unchanged filter/written ties.
Missing counts or keys select the defined default; no statistic is guessed
from syntax. An explicit order bypasses statistical selection. VO no longer
has inline `count`/`key` annotations or a `PlanOptions.evidence` field.
File key restatements do not establish Sequence uniqueness.
CPU TPC-H default/par load the checked-in neutral SF1 document by a crate-relative
path, without a dataset dependency; provenance records its source manifest path
and SHA-256. The SF1 ordering input must not be reused as exact SF10 bounds.
GPU imports its selected scale's manifest under `TPCH_GPU_DATA_ROOT`.
Sequence's independent evidence grammar is
unchanged in this step: moving its Count/Bounds needs its own source-binding
seam, while Selectivity/ForeignKey/Sorted have no equivalent document fields.

The optional `cq-cost` package exports a versioned compiler-record projection:

```text
cargo run -p cq-cost -- export --language datalog --source program.dl \
  --compiler-hash sha256:HEX --out plans.json
cargo run -p cq-cost -- export --language cq --source query.cq \
  --compiler-hash sha256:HEX --cq normal.cq --vo checked.vo --out plans.json
```

The library accepts a separate normalized CQ and checked VO, or Round plus
every checked hole's `(CQ, VO)` pair; the CLI can consume those records or
invoke the production compiler once. For consumed records, each `--cq`
supplies the actual normalized CQ context for the corresponding standalone
`--vo`; `--source` remains the authored input. Repeat `--cq` and `--vo` in
matching order for Datalog, following Round hole order. Their counts must
match; CQ export requires exactly one pair. The CLI does not regenerate
consumed CQ contexts from the authored source.
Source, compiler/configuration and input digests, original occurrences,
delta/seed/driver/permutation facts, bindings and complete IR are retained.
The semantic program digest excludes the dataset attachment, avoiding a cycle
with calibrated facts that cite that program; separate source/evidence/input
digests still bind its exact bytes. Record-only export accepts explicit CPU,
CPU-par and GPU target identities and checks Round target compatibility.
CLI compilation is CPU-only; other routes supply their checked records.
The checker verifies the projection against those records without rerunning
scheduling. A CPU VO boundary has no selected physical providers and says so;
it does not invent them from a target name. This export is input to the
external tool, not a new compiler default. Crossing-plan guarded advice and
its Datalog transport remain design-only, outside this step's static-advice
scope.

Static costing is an explicit offline command:

```text
cq-cost advise --plans plans.json --evidence evidence.json --target cpu \
  --out costs.json --advice orders.dl
```

The evidence must supply the selected target's declared rational `h`/`m` and
matching configuration identity; there are no guessed defaults. A deliberately
authored `h=m=1` reference policy is normalized row work, not hardware timing.
The tool enumerates the complete I1–I3-legal positive-prefix orders, drains
ready checks before every introduction, retains original occurrences/delta
positions, and records every candidate and per-step inequality/citation.
Bindings come from the checked export, never from lowercasing relation names.

Bounds use cardinalities, maximum degrees, declared keys and support caps:
`|P join R| <= min(U_P*M_R, U_R*M_P, min(D_P,D_R)*M_P*M_R, U_P*U_R)`.
Only supported terms participate. No product of independent selectivities or
HLL-derived bound is introduced. Prefix degrees retain earlier multiplicity;
a base key is not automatically a key of an expanded prefix. The row-work
model charges each processed scalar/guard/expansion frame, including generated
head-copy lets; these do not multiply rows. It is not a bound on arbitrary
opaque Rust instruction time.

For a steady variant, an affine certificate `a*|delta_r|+b` becomes
`a*F+b*N` under the recorded complete partition, or a conservative upper bound
under completed insert-only-set consumption. Seeds and recorded component-key
construction execute once. Persistent read-index maintenance is charged once
across holes and strata, with its recorded policy; negative-key helpers retain
their actual canonical provider rather than inventing a constant-key index.
The greedy planning rank and every reused demand are printed.

Incomplete/incomparable bounds retain the recorded incumbent. Crossings are
reported, but no runtime guard or pair of alternative plans is emitted.
Existing partial-read/nested work without finite model certificates remains
explicitly unknown rather than being replaced by an uncosted coarse plan.
The named `restricted-view-lifetime` gap also stays unknown: joined filtered
views can introduce per-invocation materialization that the VO/Storage export
does not record for each candidate. No such candidate gets a complete
whole-run certificate or a new order. A single-read component key builder
does have an explicit lifetime: its occurrence restrictions and key-set
construction are charged once, not once per steady round.
The output is a complete replacement for existing authored directives:
unchanged phase/intersection/admission advice is retained, and new whole orders
use `plan(Label, delta(R@k), order(...))` (without `delta` for seeds).
Core CQ uses `plan(order(...))`. Provenance comments escape newlines; they
cannot inject unchecked advice.
Changed variants hold the recorded admission owner fixed with an explicit
checked `known(admit)` or `known(at(...))` constraint. Existing authored hints
are preserved; automatic InPlan receives a checked ready boundary. The report
records that ownership was held fixed and does not select a potentially cheaper
different owner. Adoption rejects a changed constraint or a directive set
different from the costed one.
A declared acceptance-review policy suppresses a proven sub-1.05 whole-run
bound ratio. `noise_screen` records the rejected candidate, exact 21/20
threshold and policy provenance; shared demands follow the retained incumbent,
not a rejected plan. This screen is not a measured speedup or a proof that a
larger bound difference is useful.

The compiler-owned `RulePlan` grammar is shared by macros and the tool.
Before emission, selected Datalog orders go through Incremental **before**
Storage, then Fork and shared VO-Order; predicted read demands must exist in
the resulting Storage. This establishes logical advice legality, not execution
or a replay of a custom/GPU physical pipeline. The adopting target must still
check its own representation/launch decisions. Printers make no cost choice.

### Optional completed-run calibration

The optional `planner-calibration` feature on `cq-datalog` (also forwarded by
the prepared `srd-doop-mini` and `srd-doop-mini-par` engines) enables the
`cq_cost::calibration` API, reexported as `cq_datalog::calibration`.
`measure`/`try_measure` take checked compiler records, baseline evidence, the
explicit stats request, a run ID and one execution closure. They install an
isolated counter epoch, execute the unchanged program, and invoke the offline
cost tool only **after** a complete return. They never adopt advice at runtime.

The generated input hooks reuse the existing canonicalization traversal:
raw decoded rows feed `typed-row-stream-v1`, including order/duplicates;
only successful full-tuple admission updates exact frequencies. The canonical
cardinality is independently checked at input completion. No second tuple set
or data-file pass is introduced. Raw metadata bytes must be forwarded from
the same loader read; the prepared DOOP loader does this for `meta.json`.
Unsupported scalar codecs produce an explicit calibration failure, not a
coercion or a new query type bound. Without an active observer the codec is
not evaluated.

Each component records seed mass, every consumed delta, executed N, final
canonical rows and the unconsumed tail. Complete insert-only consumption is
checked by `sum(consumed deltas) + tail = final_rows`; a partition premise is
emitted only when the tail is zero. Derived columns without measured scalar/
degree statistics remain explicitly unordered/unknown. Dataset identity is
recomputed when the measured manifest adds derived schemas; all numbers are
bound to that identity and the completed program/configuration/run.

An optional checked continuation trace records the actual VO, mechanical
VO-to-RA origins, and subsequent checked IR snapshots through Physical and
Structures/Sequence. It is metadata, not another planner. Compiler-owned host
parameter substitutions are recorded by the partition printer and replayed by
its shared substitution routine; no source identity is guessed from generated
Rust. Metadata is erased when calibration is disabled.

Reports use existing **operator** counters at actual Physical definitions and
their explicit return, never storage `rows_enumerated`. The VO/hole/variant/
seed-or-round identity survives joined Rayon tasks. True row-counter aliases
are grouped once, their VO coverage deduplicated, and the covered bounds
summed with `fused` marked; shared timers do not fuse row counts. Distinct
physical invocation/worker instances remain distinct. Input/setup caps are
reported separately from joined stream-output caps, so a raw source scan is
not compared to an empty join's output bound. Concrete-build/timer auxiliaries
are identified separately rather than mislabeled as Physical definitions.
An unobserved operator is reported explicitly, not published as an exact zero;
an operator that actually observes an empty stream may report zero. Replayed
Cartesian RHS counters use the checked join-output envelope only for a
host-only, single-consumer RHS with the same VO coverage as its empty-key
probe. Other unsupported replay scopes remain Unknown rather than receiving
a misleading single-scan bound.

The count report shows final/N/seed/tail and delta series, then Physical-to-VO
coverage, bound role, planned bound, observed rows, measured-fact bound,
unknown/exceeded status, fusion and per-variant advice differences. It neither
fits hardware h/m nor reports workload timings. Failed/unknown bound
comparisons remain observational; they never become cardinality certificates.
Incomplete runs, stale decoded input/metadata, count overflow, mismatched
programs/scopes or bad provenance cannot publish calibrated facts.
The bound-quality review reports `bound_sum / observed_rows` for incumbents,
and observed per-step ratios only for an identical executed VO prefix/key/
version and an unambiguous output counter. Zero denominators remain undefined;
changed prefixes and fused multi-VO scopes remain unobserved. The numeric bands
are C (<1.05, suppressed), D ([1.05,2)), and B (>=2, winning-key evidence still
unvalidated). No A certification is inferred from aggregate counters. In
particular, output/input is an average, not a maximum fan-out certificate.
The optional-feature command below reviews saved artifacts without executing
the workload; `--vo-out` prints checked current/proposed VOs, without adopting
the proposed orders:

```text
cq-cost review --plans calibration.plans.json --evidence calibration.facts.json \
  --costs screened.costs.json --report calibration.report.json \
  --out bound-quality.review.json --vo-out checked-vos.json
```

Per-key maxima need complete lookup exhaustion, exact upstream-prefix and
relation-version/round identity, and a key-domain coverage certificate before
they can replace degree bounds. These are not supplied by the current aggregate
counters. The xalan result and the certifying-run entry conditions are recorded
in `/data/design-notes/planner-step2-design.md` §11.

```text
cargo run -p srd-doop-mini --features planner-calibration -- PREPARED_DIR \
  --calibrate plans.json evidence.json stats-input.json OUTPUT_DIR RUN_ID
# The parallel engine uses the same flags, plus --threads N:
cargo run -p srd-doop-mini-par --features planner-calibration -- PREPARED_DIR \
  --threads N --calibrate plans.json evidence.json stats-input.json OUTPUT_DIR RUN_ID
```

This route bypasses the harness's benchmark clocks, preserves answer export/
`BENCH_CHECK_WITNESS`, and refuses to overwrite artifacts. It emits
`calibration.evidence.json`, `calibration.facts.json`, `calibration.plans.json`,
`calibration.costs.json`, `calibration.orders.dl`, and JSON/text reports.
The full evidence contains Physical diagnostics; `planning_facts()` explicitly
removes only step observations, retaining all admitted facts and complete delta
series. The facts file is the compiler attachment, avoiding diagnostic-volume
duplication into every hole. Cost matching compares this checked projection,
while reports retain the full evidence-file hash. Numeric facts and provenance
must still match exactly. Exceeded comparisons produce a nonzero CLI result
after emitting truthful diagnostics, without changing the running plan.

Synchronous CPU and CPU-par components are supported. Recorded
`schedule.progress=relational` is rejected at setup: event-driven member
batches need a distinct per-driver invocation-count model, not an invented
shared N or observation barriers. Device calibration requires a device-counter
producer and is not enabled by this host feature. The separate future
restricted-view lifetime model must record actual candidate materialization/
build ownership before those unknown costs can select an order.



### GPU TPC-H authored stage frontend (DR-StageSource)

The 26 GPU programs are authored in
`experiments/gpu/tpch/queries/<query>.stages`. CPU workload files are independent
reference programs, not importer input. One checked, printable `stages::Program`
owns input producers, domain identities, captures, stage dependencies and
terminal composition. The frontend grammar is:

```text
program name {
  domain Labels = dictionary(items.label);
  input Items(key: i64, label: StringCode in Labels) = table(items);
  input Selected(code: StringCode in Labels)
    = dictionary(Labels) where { |text: &str| text.contains("green") };
  operand cutoff: Date = date { Date::load("1994-01-01").expect("ISO date") };
  stage s0 {
    inputs { Items = Items; Selected = Selected; }
    captures [];
    evidence {}
    cq {
      struct Query;
      relation Items(key: i64, label: StringCode);
      relation Selected(code: StringCode);
      relation answer(key: i64, label: StringCode);
      answer(key, label) :- Items(key, label), Selected(label);
    }
  }
  publish Rows = s0 owned;
  terminal { from s0 project [column(0), column(1)] dictionaries [1 = Labels]; }
}
```

`input R(...) = host uses [dependencies] rust { expression };` declares opaque
host preprocessing; `host value uses [...] rust { expression };` declares a
nonrelation host dependency. Dependencies are borrowed for the opaque producer.
`operand x: T = rust { expression };` and `date { expression }` preserve their
payloads and evaluate on the host, never at build time. `value { expression }`
in a terminal projection is a host-once constant extension. Terminal clauses
concatenate in declaration order; projection slots are explicit and cannot be
out of bounds or repeated. The examples' Rust types remain opaque.

Top-level column domains propagate to CQ's `in (domain)` declarations; shared
CQ normalization owns exact/lower/upper operand captures. The importer never
distinguishes a string literal from a block expression. Dictionary predicates
are explicit value-once producers, not calls recognized by a Rust function
name. Host producer code carriers convert to published `u32` through runtime
trait dispatch at the declared domain-column boundary.

The existing CQ→VO→RA→Physical→Structures→Sequence pipeline consumes each stage.
Input representation, nullable/grouped publication and dictionary origins
remain recorded facts; `plans.json` includes `stage_source`, per-stage
`sequence_input`, `sequence_planned`, trace and publication records.
Checkers inspect those facts, never rerun source extraction. The code no longer
contains the Rust stage/terminal/ownership-map recognizers, Data/load/EXPORTS
producer recovery, dictionary-call recognition, or compile-time `date` folding.

Acceptance is the combined shared-domain/frontend tree: all 26 queries exact
at GPU SF1 in advised and automatic modes, Sequence dumps identical or
individually classified, expanded-Rust evidence for intended differences,
frontend/domain regressions and formal checks. H19's explicit `where` DNF
belongs to the shared interned-string migration, not an identity claim for
rewriting opaque Rust in this frontend.

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
fresh evidence. The default gate only expands Rust and never runs timed
benchmarks, so a classified intended (g–l or defined) or runtime-changed target
first exits 2 with `verification: "required"`. Supplying evidence:

1. Run the gate with `--classes`; fix or review every hunk until `unexplained`
   is 0 (exit 1 otherwise — evidence never excuses an unexplained hunk, and
   `--verify-command` is not even invoked for such targets).
2. Either rerun with `--verify-command '<argv template>'` (shell-free;
   placeholders `{target} {tree} {key} {expansion} {baseline_tree}
   {baseline_key}`; must print `{"exact":true,"counters":{"passed":true,
   "values":{…}},"scope":"…","evidence":[paths relative to the repo root]}`),
   or run the exactness/counter checks yourself and rerun with
   `--verified-results results.json` containing
   `{"version":1,"results":[{"target":…,"key":<report key>,"outcome":{…}}]}`
   (evidence paths relative to `results.json`'s directory). The `key` must be
   the current report's `key`; any source/runtime/input change yields a new
   key and requires new evidence.
3. Outcomes must attest actual exactness and counter checks: `exact: true`,
   `counters.passed: true`, nonempty counter `values`, a nonempty `scope`, and
   existing evidence files. Timing alone is not evidence. Files are copied
   into the cache and hashed in `target/expand-gate/cache/<key>.json`.
   The target reports `passed` for a successful verifier command, or `cached`
   for an imported result (including its first run), and the gate exits 0
   only when every selected target passes. Subsequent runs reuse that evidence
   while the key and cached file hashes match.

For example, after collecting the current report keys and writing the above
import envelope from actual checks:

```sh
python3 tools/expanded_gate.py --baseline <rev> --targets all \
  --classes tools/expanded_gate_classes.json \
  --verified-results path/to/results.json
```

Keep the same baseline, target selection and data-manifest arguments used to
collect the keys. A `--verify-command` adapter should emit only its outcome JSON
on stdout (send diagnostics to stderr); it must perform those same checks, not
merely echo success. Neither evidence route changes hunk classification.

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
  subatom that binds nothing (RL-Check, `⋉`), an expand is the full subatom
  (RL-Join, `⋈`); Plan-Le/CK-Le check it. What the paper adds is operational
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
    - The retired automatic Sequence planner (`plan(machine)`, `lookup_first`)
      re-planned authored CQ orders. On DOOP, decomposition-only beat the
      default by 1.070x [1.064, 1.124] (xalan) and 1.234x [1.231, 1.238]
      (batik), all 40/40 exact.
    - GPU `probe smallest` was silently emitted as sequential range
      enumeration + membership; it is now rejected (T-Gpu) until P4 ranges.
    - DOOP AIPT_Store is 2.6% of the traced batik fixpoint: skew-heavy rules
      are not where fixpoint time goes; per-hole time from the counters must
      choose targets.
    Diagnosis (2026-10-30): answers are exact in every variant; the
    semantics (VO/RA meaning, L&E as subatom phases) hold. The losses are
    implementation: the retired `lookup_first` was an exact-semijoin adaptation of
    RPT (doc/formal/passes.tex, order(transfer)), not RPT's cheap Bloom
    pipeline. It materializes stored transfers, probes again, replicates
    adaptive alternatives and runs on cyclic queries and per-iteration
    Datalog deltas, outside RPT's acyclic guarantee. RPT's robustness is
    insensitivity to join order at low transfer cost, so a transfer that
    makes plans slower than no transfer (DOOP 1.07x/1.23x, H05) indicates
    a wrong realization. Fixed by P3.3 (delete re-planning) and P7
    (filters built during builds).
  - **P3.3-lite** Remove `lookup_first` and automatic Sequence replanning,
    including adaptive optional-filter advice/provenance and their formal rules.
    GPU automatic routes use decomposition plus existing grading; authored
    Sequence recipes, `choose` and key partitions remain.
  - **P3.3** Default planner emits L&E phases from evidence; grading learns
    R's schedule shapes (retained small prefixes, segmented fan-out) from
    recorded evidence; GPU match-set carrier follows P4. **Status
    (2026-10-31):** slice A (O-EvidenceLe/CK-EvidenceLe: recorded counts and
    TPC-H keys in the VO; a key-functional root replaces the written root only
    when it dominates it — count ≥, filter rank ≥, and no neighbour other
    than the written root filtered as strongly), slice B (GR-RetainPrefix,
    GR-SegmentFanout, `retain(name, rows)` advice) and slice C (ADM-InPlan,
    CPU Datalog, advice-only: the structural default measured DOOP xalan
    `mini` 2.36x, batik `mini-par` 2.95x) are implemented. Only the GPU TPC-H automatic route records
    CQ evidence. Paired GPU vs `c1b75718`: H04 automatic SF1 0.661
    [0.652, 0.676], SF10 0.500 [0.497, 0.503]; d85 SF1 identical code
    (1.007 [0.996, 1.022]; without the neighbour clause 1.363 [1.358, 1.367]);
    advised H02 SF10 0.733 [0.720, 0.745] and H11 SF10 0.422 [0.417, 0.427]
    from retained prefixes; H05 step-R order SF1 0.121, SF10 0.070.
  - **P3.3e CPU evidence — done (2026-11-01).** CPU TPC-H `default`/`par`
    record dbgen SF1 counts and TPC-H primary keys for canonical base
    relations (R-TpchEvidence, driver in `tpch_runtime.rs`; counts are
    recorded once because workers compile before export and the planner
    only compares counts, whose order is scale-invariant). Only `retail_h04`
    changes plan (expanded gate); `default` 0.565 [0.543, 0.598], `par`
    0.229 [0.213, 0.248] vs `c395fe83`. Datalog records per-read fan-out
    (DL-Fanout) from optional source keys (WF-DL-Evidence); FK-Known puts an
    unadvised hole of a planned queue in plan iff a later read is `multiply`.
    DOOP sources record keys checked functional on xalan/batik; no DOOP hole
    is in plan by default (CPU engines expand byte-identical), so DOOP was
    not re-timed. Not done (open): per-round DOOP fact counts and CPU
    GR-RetainPrefix grading from recorded counts.
  - **Follow-up from structural planner step 1:** GR-Retain reuse is defeated
    when a membership check interposes between the lookup and its reuse
    (h02/h20 `le_runs` after I2); retention should look past non-introducing
    steps. This step records the observed emitted-code consequence and real
    exactness/counter evidence, not a retention rewrite or a performance claim.
- **P3b Profile the engine gaps** (`doc/STORAGE.md` §12.4): GPU h06
  reduce/dedup at 16–74× a hand kernel; CPU h01 at ~140× a String-grouping
  microbenchmark. Profiles (perf, nsys/ncu) decide whether these come before
  the storage slices. **Profiled 2026-10-31** (`experiments/gpu/compare/p3b-profile/`,
  M0 binaries): GPU h06's scalar reduce ran on one 256-thread CTA (51% of
  kernel time, 0.9% of peak bandwidth), fixed by P3e's recorded grid fold;
  CPU h01 spends 61% in comparison sorting of wide DISTINCT valuations (keys
  are already StringCode), addressed in P4 compiler selection below; GPU DOOP
  gaps go to P6b.
- **P3c Engine-wide storage rules** (`doc/STORAGE.md` §13): intern strings
  at input for the CPU catalog as on GPU; retire V decode sites through their
  owning representation steps, measured per changed target. **Checkpoint
  2026-10-31:** `doc/DECODE-AUDIT.md` assigns all 96 V sites: P4 owns 5
  encoded-comparison/index sites, P5 owns 17 columnar-load sites, and P6 owns
  74 packed-carrier/selective-read sites, with cross-step dependencies recorded.
  None is a decode call in the CPU CQ/Physical/pull printers; string interning
  alone demonstrably retires none. Structures records per-column preservation
  or authorized opaque-Rust/final-host reconstruction, refined into
  Physical/Sequence and consumed mechanically by printers. R/P decoding stays
  only with the corresponding recorded read/publication authorization; no
  unused CPU answer-boundary scaffolding is added. The uncommitted sort-canon
  work (a recorded sort strategy, whole-stream collect/sort/unique, push callbacks waiting
  at the barrier) is superseded by P3d: it treated duplicate state as a buffer
  rather than an index, so it lost laziness and reimplemented index work.
- **P3d Distinct and admission are anti-joins against an ordinary index**
  (decided 2026-10-30; replaces CQ `seen`/sort canon and Datalog `seen`/`all`).
  - **Historical problem at the decision date.** CQ canon (`__cq_seen` hash set, then whole-stream sort) and
    Datalog admission (CPU `seen` beside `all`,
    `features/datalog/print/src/lib.rs:703-718`; GPU `DeviceRuns::admit` fused
    with maintenance, `experiments/gpu/datalog-runtime/src/runs.rs:163-177`)
    each kept duplicate state outside the index machinery. The result: shadow
    copies, contention, lost laziness, and index work (selection, maintenance,
    providers, lazy merge) redone ad hoc in each printer.
  - **One semantics.** No layer's semantics contains sorting or a seen set,
    only RA. Distinct over a stream produced in batches `B_1 … B_n` is an
    incremental anti-join against the output relation O:
    `δ(⊎_k B_k) = ⊎_k (δ(B_k) ▷ O_{k-1})`, `O_k = O_{k-1} ⊎ (δ(B_k) ▷ O_{k-1})`
    (L5, independent of granularity). A batch of one row needs no inner δ,
    which gives streaming. One batch is the whole stream. A Datalog round is
    the same operator with `O = R` and one batch per round:
    `Δ_{r+1} = δ(⊎_{h: head R} π_x̄ Q_h(A_r, Δ_r)) ▷_x̄ R@A_r`,
    `A_{r+1} = A_r ⊎ Δ_{r+1}`. CQ canon and Datalog admission are therefore one
    construct, `admit`, over an ordinary relation and its ordinary indices.
  - **Laws:** L1 `(⊎Q_h)▷A = ⊎(Q_h▷A)`; L2 `δ(X)▷A = δ(X▷A)`;
    L3 `δ(X⊎Y) = δ(δX⊎δY)` (partial/per-task distinct); L4 in a rule,
    `¬O(x̄)` is an ordinary negated subatom at the VO level where the last
    head variable binds (predicate transfer to the deepest join layer). Levels
    below it become RL-Check witnesses. It reads the committed version, like a
    delta occurrence: not recursion through negation, and
    `A_r ∪ (Q▷A_r) = A_r ∪ Q`. L5 as above.
  - **Canonical is an ordinary index, chosen by index selection.** O's
    indices form one set derived by the index-selection pass (CQ Structures
    built providers; Datalog Storage layouts) from every read of O, including
    the anti-join. Each access pattern is completed to a full column
    permutation, so every index holds complete rows and can answer full-row
    membership. The pass records which member of that set is
    **canonical**: a reference to one ordinary index, not a separate store,
    provider kind or type. Policy (SRDatalog
    `ir/hir/index.py:select_indices/canonical_index`): prefer a complete index
    whose full version is already read by joins, so no dead index is
    maintained; else the first complete index; else the identity order
    (Ascent's fallback: its `relations_full_indices` is an ordinary
    `IrRelation` in the same set, shared with any body read in that order,
    `ascent_hir.rs:230-252`). Advice may pin the choice; the checker verifies
    the record names a complete member of the set. The canonical index alone
    has three duties: admission builds the batch in its order and anti-joins
    against its full version (`fresh`); the other indices derive their deltas
    from the canonical delta by permutation (SRDatalog
    `RebuildIndexFromIndex`); and it is always maintained in full. Every other
    index keeps a full version only if some join reads it as full (SRDatalog
    `full_needed`). Every index is maintained by its provider's merge ⊕_L, with
    law reads(⊕(V,D)) = reads(V) ⊎ D; reads are stable between publications;
    lazy merge is allowed (Ascent's `RelIndexMerge` + `freeze`). How a provider
    deduplicates a batch (hash, sorted-run merge, radix) is its own business;
    sorting is never a semantic step or a strategy name. No `seen` or `all`
    store exists: a full scan reads any complete index.
  - **Ownership** (multi-stage, each boundary checked, never by rerunning a
    producer): Incremental/CQ = obligation; RA planning = placement of the
    anti-join per rule (in-plan at level k, or at `admit`) and the
    existential RL-Check levels; Round/Sequence = `admit` granularity (row,
    `batch(n)`, whole/round) and the residual δ, checked so every producer of
    O discharges ▷O exactly once; index selection = O's indices like any
    relation's; publication = ⊕ per index. Printers print recorded facts only.
  - **Advice** (pinned choices, planner fills the rest):
    `plan(L, …, known(at(level k)))`, `plan(L, known(admit))`,
    `admit(O, granularity(batch(n)), partial(per_task))`; index advice as for
    any read.
  - **GPU printers print recorded providers** (P3d C1, CUDA audit
    `agent://CudaProviderAudit`). GPU binding/grading records the built-provider
    catalog ABI and construction type, complete native lookup permutations,
    download adapters, directory policy, key codec and publication storage.
    Independent checkers validate these facts; printers consume them.
    Keyed grouping records its segmented accumulator in `Structure::Group.provider`.
    Upload adapters and immutable bounds provenance are also recorded. C1 is
    CQ-only; C2 has now implemented Datalog provider identity on top of
    Track B's ordinary canonical-layout records, without parallel GPU-only
    ownership fields. RC (P3d) completed the device cutover: device Storage records
    every layout's state and initializers, NEW (`fresh_run`), key sets, candidate
    containers, arenas and scratch policy; admission is read-only
    canonical `fresh` against the ordinary layout state, publication appends to
    every layout, and the former fused admission API is removed.
  - **GPU CQ printers print only recorded facts** (P3e, audit
    `agent://GpuPrinterAudit`). GPU-Launch also records initial capacity,
    stripe-batch reservation and per-level strips (resolved tile defaults and
    fold scopes). GPU-Realization records, after publication storage, device
    source access (scan/positions/provider interval, setup/build), one step per
    Sequence definition (terminal-group alias, terminal group, dispatch, host
    union, upload), host transfers (complete rows), provider descriptor bounds
    including measured run keys, auxiliary element-kernel geometry and closed
    captures; an independent checker validates them, and a selection failure is
    recorded and raised only when that candidate is printed. The terminal group
    records its input realization and completion; keyless groups use fused
    occupancy-grid partials plus one combine (GPU-GroupGridFold, the one
    intended work change). TPC-H grouped answers record slots and key bounds
    sources (DR-GroupedPublication). GPU Datalog printing is not covered.
  - **Removal scope.** CQ `CanonStrategy`/`dedup::sort` canon and
    `__cq_seen` are replaced by Track A's ordinary canonical index; CPU
    Datalog `seen`/`all` (hash, `ShardedSeen`, `RunSeen` seeds) and separate
    DL-InsertNew/DL-ShardedAdmission stores are removed, as is GPU fused
    admission. This does **not** remove every callback/materialization
    barrier: Track A's sorted rows collect before yielding, and the two
    row-streaming dispatch tests remain ignored. Expansion counters,
    exact oracle/TPC-H/DOOP and paired timings cover the implemented
    cutovers; first-answer-after-first-batch is not a completed acceptance
    claim for row-granular CQ streaming.
  - **Track B (CPU Datalog) as implemented.** Storage `Providers.canonical`
    indexes one full layout of the relation keyed by every column
    (DL-Canonical): the first such read layout, else an appended identity
    layout. Every layout keyed by all columns is a payload-free key set
    (`HashSet<K, Fx>`, protocol `set`); other layouts stay grouped maps.
    Admission (DL-InsertNew) is the anti-join `¬C.contains(k)` with the
    committed canonical layout plus insert-if-absent into the round-local NEW
    version `Relation.next_canonical`, using the provider's own operations;
    publication maintains the canonical layout as an ordinary `full_layouts`
    member (no `all_extend`); EDB loading is insert-if-absent into the
    canonical layout, extending the other layouts with admitted rows. Outputs
    move (`set`/`grouped_map`) or clone (`bulk`) the canonical layout. The
    parallel extension shards each IDB canonical layout by default
    (`ShardedLayout<K, (), S>`, or `RunLayout<K, ()>` under `layouts(runs)`)
    and admits per shard (`ShardedLayout::admit`: chunks hashed and routed in
    parallel, one task per shard checks the committed shard and inserts into
    NEW, which takes the committed hasher so one hash serves both; sorted runs:
    concat, sort, dedup) (DL-ShardedAdmission). The first realization (per-chunk
    anti-join, then NEW `extend_from` per chunk and a serial distinct-key
    listing) cost batik `mini-par-keyed` at 15 threads 1.18× M0 (admission
    3.3 s vs M0 2.0 s); per-shard admission takes 1.0 s and measures batik
    `mini-par`/`mini-par-keyed` 0.89×/0.90× and xalan 0.95×/0.93× M0 (t15,
    5 interleaved pairs, CIs below 1.0). `seen`, `all`,
    the `Canonical` wrapper, `ShardedSeen`/`RunSeen`, `seen(runs)`,
    `RuleHole.insertion` and DL-RunAdmission are gone; Datalog uses no
    `maintain::{Fresh, Publish}` trait.
  - **Parallel tracks** (decided 2026-10-30). M0 (`e189e296` baseline plus the
    contract commit: ADM-*/IX-* in `incremental.tex`; its admission/publication
    provider traits were later deleted by track A's canonical-reference
    cutover) is serial on main.
    Each track then works in its own worktree
    `/data/mini_linq-wt/<track>` on branch `codex/p3d-<track>`, rebases on
    main after each milestone and never merges sideways. Agents never commit;
    the main session merges with full gates (expanded gate with reviewed
    classes, workspace tests, regressions, formal check, exactness, paired
    timing; GPU timing one track at a time).

    | Track | Scope | Directories | Model |
    |---|---|---|---|
    | A CQ admit | As built: canon is the one-batch build of O's canonical index, an ordinary index named by a canonical reference (`rows`/`[cols]`/`host`) chosen in Structures index selection and typed by a `Distinct` provider; `__cq_seen` and `CanonStrategy` removed. Row-granularity admission and first-answer streaming are not implemented (see below) | `languages/cq-lang-{structures,sequence}`, `passes/cq-{physical-lowering,structure-selection,sequence-lowering,sequence-plan,physical-realization}`, `features/rewrites`, facade tests | Claude |
    | B Datalog CPU admit | Storage index selection over all reads incl. the anti-join, completed permutations, recorded canonical (SRDatalog policy); Round `admit` = fresh on canonical (δ within round + ▷R) then publish to every layout; remove `seen`/`all`/`ShardedSeen`/`RunSeen` seeds | `features/datalog/*`, `features/datalog-parallel/*` | Claude |
    | C GPU conformance | Printers print recorded providers (CUDA audit list); then GPU Datalog admission split into fresh/publish over Round records from B | `experiments/gpu/{sequence/core,sequence-runtime,datalog,datalog-runtime}` | GPT (task default) |
    | D Index providers | Unified `OrderKey`; narrow grouped offsets, merge-path and lazy ratio-bounded `RadixRun::merge` (IX-RunMerge, a multiset union). The admission/publication maintenance traits and their set/map/run/trie impls were deleted by track A's cutover | `runtime/cq-storage` providers, runs, key codecs | GPT (task default) |

    **Track A as built (canonical reference).** By user decision the
    canonical index is an ordinary index chosen by index selection
    (SRDatalog `select_indices`/`canonical_index`), with no new IR node,
    runtime type or printer construct. Structures records `r canon = rows`
    (the rows record, default `::cq_storage::dedup::Sort`, the sorted array;
    PL-Distinct), `r canon = [cols]` (an existence-only complete Built index,
    the result's only storage) or `r canon = host` (a direct load-verified
    host-rename carrier), and `r head = Type` for the answer
    (STR-Canon, SEL-Canon, PH-Canon). Single-consumer rows records move by
    value into their consumer (STR-RowsByValue); Sequence lowering omits the
    Store of such a canon `let` (`Analysis::rows_by_value`), so Tasks plans
    it exactly like a canon pipe (routing, per-part canon, partial-group
    folds over tiles), and a planner Store over it moves by value into its
    single driver instead of being re-collected. Sequence carries
    `Obligation::Canon { canonical }` and catalogs offer `Distinct`
    providers instead of canon tokens. Row-granularity admission would need
    a recorded admission granularity, which was not added, so the streaming
    dispatch tests stay ignored.

    Merge order: M1 = C's provider printing, then A; M2 = B (rebased on A)
    and D's providers; M3 = C's Datalog admission (needs B's Round records);
    then P3.3 planner work (touches `cq-sequence-plan`, after A). Main and
    milestone reviews run on Claude.
- **P4 Index runs.** Runtime radix index role now uses one order-preserving
  `OrderKey` law (replacing `DenseOrdinal`/`RadixKey`; codec policies delegate),
  narrow offsets and stable merge-path, with lazy ratio-bounded publication.
  Membership/count/range/key reads do not read item columns; lookup cursors
  visit all published runs without compaction. `RadixRun<K,V,C=Vec<V>>`
  offers explicit `ItemColumns<Row=V>` layouts: tuple-of-Vec SoA columns,
  selective group-range field reads, and late row materialization only for
  demanded rows or contiguous legacy item reads. Compiler selection records C.
  Standalone AdaptiveKeySet remains for existing non-Clone interval/hash
  callers; radix maintenance introduces no adaptive staging or `Arc`. Datalog runs
  become grouped runs merged by merge-path. The dictionary policy is decided
  here from measurements (`doc/STORAGE.md` §10): code assignment is a
  publication, aligned with round/load boundaries.
  Memory placement is a per-target fact (`doc/STORAGE.md` §11): shared byte
  layout and laws; CUDA and Metal differ in allocation, copy reasons and cost.
  **Compiler selection (CPU, done 2026-10-31).** (a) A one-level payload
  build records `items rows | items columns` (STR-ItemLayout); the checked
  analysis prints it at RadixRun's declared layout slot as `::std::vec::Vec<_>` /
  `::cq_storage::TupleColumns<_>` (the instance spelling never states it).
  `radix_runs` chooses it from recorded read needs (SEL-ItemLayout): the
  carried-column demand analysis, now `cq_lang_structures::demand` (moved
  verbatim from cq-print), gives each probe/merge reader's columns; `columns`
  iff the payload has 2–12 fields and some reader carries part of it. Every
  TPC-H radix reader carries the full payload, so all record `rows`
  (spelling-only hunks). (b) `cq_storage::dedup::Packed` is an ordinary
  Distinct provider: a sorted run keyed by the packed `OrderKey` of every
  column (PL-PackedCanon; comparator fallback beyond 128 bits). It is
  selected only by authored, checked advice (`canon(packed)` →
  `cq_rewrites::packed_canon`, EXT-PackedCanon); h01 opts in. Making Packed
  the canonical pass's own choice needs a recorded column-representation
  (OrderKey) fact or evidence-driven grading (P3.3) and is deferred.
  `OrderKey` now covers `cq_load::Decimal<S>` and `Date`. Measured (SF1, 5
  interleaved pairs of 5-run medians vs 1031adf6, bootstrap 95% CI of the
  geometric-mean ratio): h01 default 1981 → 939 ms, 0.470× [0.457, 0.480];
  h01 par 4.97 → 4.87 ms, 0.991× [0.970, 1.015]. Page faults: h01 default
  process minor faults 718K → 906K (about +31K per execution, the 24-byte
  key words of 5.9M rows), peak RSS 714 → 853 MB; par unchanged (74.0K).
- **P4-GPU Device RadixRun.** Runtime done (`DeviceRadixRun<W, O, C>` in
  gpu-datalog-runtime: whole packed order-key words, offset carrier as a type
  fact, explicit item layouts, caller-supplied ratio/radix bits/merge tile,
  exact radix-prefix directory, stable merge-path, key reads without items;
  law tests against `cq_storage::RadixRun`). **Slice A runtime done**
  (`RadixLayout<K, V, W, O, C>`: checked key packing and value-lane domains,
  per-publication cached snapshot with identity and resident count and no
  host sync, republish reuse, narrow item columns, run-local match cursors
  with retained witnesses, execution-pool-qualified reads under
  fork/join readiness; GPU-RadixLayout/Cursor/LaneRead law tests).
  **Slice B done for GPU CQ built indices and non-witness smallest**: the
  `radix_layout` token (reads exact/range/membership/candidates, abi radix,
  registered after existing tokens) with authored per-instance advice
  (GPU-RadixAdvice), the `ProviderAbi::Radix` binding record
  (GPU-RadixProvider), resident-source builds (GPU-RadixSourceBuild),
  once-encoded whole-word probes (GPU-RadixEncodedProbe), exact union
  candidate counts (GPU-RadixCandidates) and the PH-Smallest realization
  record (GPU-Smallest; other side shapes are rejected, never First);
  the authored `triangle-smallest` configuration runs `probe smallest` over
  two radix sides, exact on dblp and on dblp with full-domain signed i64 IDs.
  **Radix smallest is not a default anywhere**: after computing the choice
  once per outer row, it remains 1.54× slower than the default
  `triangle-compiled` `probe first` Range plan on DBLP (paired, both locks).
  The remaining kernel is a latency-bound chain of u128 binary searches.
  **Cached-prefix experiment rejected (2026-11-03):** a compact scalar
  prefix/suffix directory did not improve resident DBLP total:
  cached/old-smallest **1.004 [0.996, 1.022]**, kernel **0.978** but build
  **1.060**; cached/Range-first **1.387**. Five rotating interleaved
  triplets used the GPU lock, recorded load/faults, and checked all 2,224,385
  tuples on every resident call. These GPU-only samples establish no
  end-to-end speedup; no CPU-exclusive/linker-layout-confirmed small-effect
  claim is made. The unused policy, runtime cache and tests were removed;
  neither triangle plan changed. Raw samples:
  [`p4gpu-cached-smallest.results.json`](../experiments/gpu/compare/p4gpu-cached-smallest.results.json).
  Remaining: a faster smallest
  read (e.g. dense or cached candidate directories, measured) before any
  default use, Retain/Expand (retained L&E witnesses need the GPU target's
  witness support), and encoded FOR key packing (two P4 DECODE V sites).
  **Slice C implementation:** canonical references use the constrained
  full-row/unit-item `radix_set` token and ordinary resident `RadixLayout`.
  Complete-key uniqueness shares stable ordering with occurrence builds;
  resident count and sorted-prefix adoption avoid a synchronizing snapshot.
  `SetBuild` records checked construction parameters including block width;
  the consumed FOR codec owns domain/width provenance (GPU-RadixSet).
  Per-thread/pool scratch grows and is reused separately from result owners.
  The old packed/wide canonical implementations are removed. The h06 keyless
  fold is unchanged.
  **Slice D implementation:** Datalog EDB/full/delta/negative layouts use
  `radix_relation`, `NativeAbi::RadixRelation`, and ordinary
  `RadixLayout<(all physical row columns), (), W, u64, ()>` views over
  `DeviceRadixRun<W, u64, ()>`. Logical prefix lookups use encoded key
  intervals and decode only demanded complete-key fields. Per-layout records
  include `offset: u64`, `construction: full_key_set`, `read: key_prefix`,
  legal configurable `radix_bits` (default zero), ratio and launch geometry.
  Independent legality predicates check these facts, schemas, initializers
  and source erasure without replaying realization. Canonical `fresh` returns
  a read-only `SortedBatch`; every secondary permutation consumes that same
  admitted batch, restoring order without another unique pass. Publication
  uses `append_sorted` and `republish(&state)`; initial maintained views share
  their backing state through `publish`, and one-shot negative views use
  `from_sorted(pool, packing, params, batch)`. Delta replacement uses
  `with_sorted(batch, params)` to reuse snapshot/kernel/field owners;
  outputs use `to_rows`.
  Delta moves, independently unique frozen negative projections, lane
  schedules and per-hole reservation are preserved. Old public state/view
  aliases are removed. Existing DL-DevicePacking/DL-DeviceRuns,
  CK-DL-DeviceStorage and GPU-Runs laws cover the cutover.
  **Acceptance, 2026-11-02:** GPU suites and fixed-fixture full relations pass;
  xalan/batik each match all 43 duplicate-free tuple sets against an independent
  CPU reference, and Cassandra all 26 tuple sets. Root, quick, formal and API
  gates pass; all 46 CPU expansions are identical to `b7e83c6f`.
  Five interleaved exact-answer pairs give current/baseline fixpoint ratios:
  xalan **1.01059 [0.99901,1.02241]** (neutral/slightly slower),
  batik **0.98145 [0.97875,0.98556]**, Cassandra
  **0.49983 [0.48944,0.51058]**. Cassandra's reported series repeats the
  comparison with matched `oxide run` materialization.
  Nsight shows unchanged sort/unique-mark counts and added unit-offset work;
  Cassandra's module load/unload calls fall 158→6, not its GPU sort cost.
  Raw evidence, profile scope and per-command gate times:
  `experiments/gpu/compare/p4-gpu-d.results.json`.
  **Slice E implementation:** `DeviceState.active_bits` records complete
  declared codec widths (the conservative full-word alternative remains legal).
  Explicit `new_set` consumes the bit window; native scalar sets sort keys
  directly, and known-unique scalar output adopts the sorted buffer without
  another copy. Wide sorting retains physical stride independently of active
  width. `DeviceRelation.admitted_orders` records canonical sorted/unique
  adoption or an injective full-heading permutation; each ordered batch is
  constructed once and shared by its full/delta references. Output field maps
  are recorded too. Full-key descriptors carry implicit unit offsets and no
  item-source map; ordinary occurrences retain materialized grouping offsets.
  Checkers validate provenance and complete bit/field coverage independently,
  including legal nonpreferred full-width/redundant-sort choices.
  Provider identity, ratio and directory settings are unchanged: no slower
  alternative provider or nonzero directory is promoted by these sort changes.
  The `gpu-datalog-runtime/radix-counters` Cargo feature separates
  canonical/layout/occurrence planned radix passes and modeled bytes, merges,
  group readbacks and metadata materialization; disabled builds use no-op stubs.
  These counters and Nsight captures run separately from paired timing.
  **Measured retention, 2026-11-02, versus `1b1540bc`:** five interleaved
  exact-answer pairs per dataset give current/baseline fixpoint ratios
  xalan **0.96599 [0.95611,0.98067]**, batik
  **0.95177 [0.95017,0.95347]**, Cassandra
  **0.96163 [0.93628,0.98586]**. Same-source linker-shuffle controls (five
  pairs each) give **1.00607 [0.99821,1.01438]**,
  **1.00501 [1.00249,1.00727]**, **1.00701 [0.99843,1.01614]** respectively.
  Every optimization CI is below one and disjoint below its control CI;
  retain the combined change, without attributing its wall-time gain to a
  subtracted profile bucket or promoting another provider/directory default.
  Fixpoint planned eight-bit radix passes fall **18,952→16,032**,
  **30,632→25,660**, **15,760→13,284**; modeled radix bytes fall
  **109.290→107.737 GB**, **108.785→104.723 GB**, **2.743→2.468 GB**.
  Sort counts and merges are unchanged in this corpus; the new certificates
  make sharing explicit but do not claim fewer sorts here. Fixpoint unit-offset
  materializations **4,183/6,999/3,062→0**; group readbacks stay zero.
  Separate diagnostic-process pool peaks fall **7.927→6.526 GB**,
  **8.377→5.240 GB**, **0.358→0.189 GB** (native CUDA scratch excluded).
  GPU suites/Cassandra 26/26, xalan/batik 43/43 duplicate-free exports on every
  primary/control sample, root, quick (15), formal and API (29 unchanged)
  pass; all **46 CPU expansions remain identical**. TPC-H advised+automatic
  is **104/104 exact** at SF1/SF10. Its separate H05 advised SF10 ceiling
  failure is **pre-existing**: pristine baseline **951.291 ms**, current
  **952.262 ms**, versus **10.35 ms**. Both commands correctly exit nonzero;
  the baseline diagnostic is not a paired H05 performance comparison.
  Source/binary/data identities, Nsight attribution, counters, sampled VRAM,
  page faults and per-command gate times:
  `experiments/gpu/compare/p4-gpu-e.results.json`.
  **H05 regression follow-up:** first-parent bisection isolates the native
  placement change in `b2bb698c`: declared column-backed index sources move
  from host descriptor builds to unpartitioned device publications. That
  revision cannot compile H05 because it emits duplicate provider parameters;
  `d3b62f04` repairs ownership and is the first runnable slow revision
  (947.118 ms). Its planned Sequence is identical to its parent's.
  Nsight identifies single-block Lineitem and Orders identity-copy
  publications, not deduplication, as the dominant added work.
  Placement now retains descriptor-backed column builds at host, while
  row-oriented native rebuilds retain descendant grades. Fixed advised H05
  SF10 measures **9.978 ms** in the full free-device inventory, below the
  unchanged **10.35 ms** ceiling; all **104/104** advised/automatic SF1/SF10
  cases are exact. GPU Sequence/core/TPC-H, root, quick, formal and API gates
  pass. `tools/gpu_gate.sh` now enforces that ceiling explicitly. Bisection,
  recorded plans/realizations, profiling caveats and gate evidence:
  `experiments/gpu/compare/h05-regression.results.json`.
  **Current state (2026-11-03, through `dbe4c2cb`):** slices D/E and the H05
  descriptor-placement repair are merged. Directory/provider defaults remain
  conservative; this is not completion of every GPU storage optimization.
  H03's remaining publication-write layout slowdown is separate work, not a
  reason to reinterpret the historical D/E or H05 results.
- **P5 Scan columns (CPU).** Measured rules only (§12.4): plain default;
  byte truncation for scans; bit-packing only for bandwidth-bound parallel
  scans; encoded execution on dictionary codes first. Block format, codec trait, summaries built with
  payload, columnar loader; prune → encoded predicate (preserved-op law) →
  positions → late materialization; STR-Blocks / BE-Blocks rules.
  S1 runtime implementation: `cq-storage::scan` Plain/Bytes columns retain
  per-block headers, aligned payload and optional same-input-pass extrema;
  Bytes uses at most 16-bit residuals, otherwise native-width plain storage.
  RT-ScanCodec/RT-ScanSummary/RT-Translate cover decoding, enclosure and encoded
  predicates. S2 adds direct columnar `ScanTable` loaders without compiler
  selection. The approved S3 checkpoint first authors blocked scans only on
  `tpch_q06`, comparing row hosts, blocked Plain and blocked Bytes with
  summaries off in pull and Tasks. Default selection remains unchanged.
  Its initial legal boundary is a unary scan/predicate/projection/Rust-read
  spine and grouping of owned computed fields. Blocked sources at join/index
  boundaries or grouping directly on encoded base fields must fail the
  Structures checker, not a printer. **Open:** join/index integration and
  encoded-base grouping/output-domain propagation, before authoring those
  kernels after a winning Q06 checkpoint; broader default promotion requires
  paired end-to-end evidence and a same-source layout control.
  Restricted source renames (atom literals) also await a source-predicate
  record and reject in the initial checker. Input rows are sets by the IO
  boundary contract, including encoded carriers; no compiler certification
  flag or query-time verification is required. Unsupported boundaries never
  silently fall back to decoded rows.
  Aggregate continuations derive their value demand from the checked outer
  key/measure specification. That requirement is transported to Structures;
  ignored witness fields publish injective value codes, not decoded values
  falsely labelled final answers. This retains full value-based deduplication:
  it is not a RowId/key-premise shortcut and does not assert
  `consumer = canonical`. Continuations that read encoded base values await
  the same grouping/ownership cutover and reject at this checkpoint.
  The lowercase-alias gap in the workload's canonical-name primary-key
  evidence mapping is separate and unchanged.
  SF1 load-order census at 1024 rows finds no disqualified static-predicate
  blocks in the 29 CPU kernels; H01 has six fully qualified lineitem blocks
  and D72/D85's `ps_availqty > 0` qualifies all 782 partsupp blocks.
  This is data-property evidence, not a scan speedup or a summary default.
  **Q06 checkpoint: measured negative for default/serial.** Five interleaved
  process pairs per comparison, five exact warmups and 21 exact samples per
  process, SF1 versus `4dd291be`: row host 23.063 ms, blocked Bytes 25.427 ms;
  paired time ratio **1.112 [1.097, 1.130]**. Blocked Plain is
  **1.746 [1.728, 1.771]** versus row host. At 24 Tasks workers, Bytes wins:
  10.320 -> 7.560 ms, **0.766 [0.719, 0.843]**; Plain is inconclusive,
  **1.049 [0.928, 1.151]**. Same-source linker-shuffle Bytes controls give
  **0.960 [0.936, 0.986]** serial and **1.016 [0.901, 1.158]** parallel.
  Every one of 2,520 timed samples and 600 warmups is exact; all process major
  faults are zero. Serial process minor faults fall 73,391 -> 46,493 and peak
  RSS 296,156 -> 184,764 KiB, but smaller storage does not overcome the serial
  execution cost. **Stop the staged rollout:** keep the authored mechanism
  and tests, retain existing defaults, and do not author the other six kernels.
  All 46 expanded targets are verified; 45 emissions are identical and Q06's
  four intended hunks are classified. Untimed checks cover 62 CPU executions
  (all 29 default/par plus four blocked Q06 routes) and 17 library kernels.
  Core/package, quick, compile-TPC-H, formal and reviewed API gates pass in
  this pre-consolidation worktree; merge-time default-member validation remains
  Main's consolidated-workspace gate. Complete samples, controls, source/binary
  identities, faults, counter caveats and per-command gate times:
  `experiments/storage-bench/results/p5-s3-q06.json`.
  **Current state (2026-11-03):** retain Q06's authored parallel Bytes route
  (`blocks-bytes-par`) and serial/parallel Plain/Bytes opt-ins. The existing
  `par` configuration still uses the row host; promoting Bytes into that route
  is not merged. Join/index integration and H12/H14/H19 authorship remain
  follow-on work, subject to exactness, paired measurements and layout controls.
- **P6 GPU storage** (compression for transfer/capacity first; resident
  scan codecs only where measured). Slice A adds an opt-in shared S1
  Plain/Bytes format and recorded read contracts, not automatic selection.
  Explicit domain/layout/family/carrier and immutable snapshot identity
  accompany optional encode-time vector extrema; metadata validation is
  independent of encoding. Initialized-byte uploads retain event-ordered
  owners. Structures owns per-consumer read authorization; runtime and
  printers do not select it or inspect Rust/type spellings.
  FastLanes remains unselectable as a shared-format family; existing
  FastLanes callers and default target selection remain unchanged.
  Cooperative scan selection, intermediate/publication caller cutover and
  grouped-run integration remain later slices. Slice A retires none of
  the 74 P6 V sites and makes no performance claim; default expanded targets
  must remain identical. Formal registration is not a gate-result claim.
  **Current state (2026-11-10):** E1 is measured (STORAGE §§12.5–12.6). FOR
  base columns beat plain by 1.16–1.18× and size-selected FastLanes DELTA by
  1.35× on the GPU TPC-H suite, including the H05 catalog path, so FOR stays
  the default. Host-packed upload and the persisted packed columns
  (`--base-cache`) are loader options, not defaults. Slice C has been redone
  GPU-first (`codex/p6c-both`; design
  `/data/design-notes/p6c-for-design.md`). The S1 route on `codex/p6b-slice`
  is dropped: its host S1 encoding made process wall 1.3–1.6× slower.
  Slice T realizes resident keyless-fold scans as checked FOR-native tiles
  (`SourceAccess::Tiles`): code-domain predicates and survivor-only reads.
  It is automatic for h06, h11 and h15 only. h06 resident is 0.771× (SF1)
  and 0.809× (SF10). Slice L moves the TPC-H cold-load dedup, bounds and
  encoding onto the device, streaming native-width pinned staging: h06
  process wall is 0.661× (SF1) and 0.113× (SF10). On the 26-query SF1 suite
  the geomean is 0.991 [0.989, 0.993]. Grouped-base admission (h01/h15 keys),
  joins/probes and intermediate publication remain later slices.
- **P6b GPU Datalog parallelism — historical initial profile** (2026-10-31,
  `experiments/gpu/compare/p3b-profile/01-doop-report.md`): fixpoint xalan 1.76 s /
  batik 4.97 s vs SRDatalog ~1.0–1.5 s / 1.8–3.0 s; one stream, zero kernel
  overlap, GPU idle 40% / 22%; joins 0.70 / 3.40 s (batik `VPT_LoadArray` alone
  1.52 s, skew), host/launch gaps 0.70 / 1.08 s, sort/unique ~0.3 s.
  **Re-profiled 2026-10-31 on aad2670d** (nsys + ncu, `/data/p6b-work`): batik join
  kernels spent 2.6 s in launches of 1–46 blocks at ~6% SM fill (a block exhausts
  its claimed root tile; grid = root tiles), `VPT_Cast` always ran one block
  (`Partition::One`), and 5734 of 19218 dispatch attempts were full replays because
  the `reserve_previous` hint lived in the head relation's shared arena.
  Done (each a recorded, checked, printed fact; 43/43 export compare identical,
  counts identical in every paired run):
  (1) `roots spread` Sequence statement (EXT-SeqRoots, rewrite
  `cq_sequence_rewrites::roots(spread)` in the DOOP GPU pipelines) recording
  `Partition::Spread` for fold-free dispatches (GPU-RootSpread): root tiles sized
  at launch from the occupancy query; batik 0.562× / xalan 0.858× of main.
  (2) GPU-ReservePrevious per Round hole (arena hint selected by the recorded hole)
  and keys-only attempts use the leased slot's capacity: 0.854× / 0.904× of (1).
  (3) Releasing pool storage once per program instead of per component measured
  0.996× / 0.989× (CIs include 1) and was reverted.
  (4) GPU-RootShare (`Execution.share`): spread single-chain dispatches share their
  first expansion's child batches through per-tile cursors; owners list hot tiles
  and idle blocks replay and help them (VPT_Cast leaves the top kernels): batik
  0.948× [0.856, 0.981], xalan 0.966× [0.894, 1.022] of (2).
  Total vs main (5 interleaved pairs): xalan 1.449 → 1.056 s (0.729×), batik
  4.335 → 1.977 s (0.456×). `roots spread` is scoped to `srd-doop-gpu`: on doop-gpu
  Cassandra (host round-trip bound, joins 0.45 s of an 8 s profiled fixpoint) it
  measured 1.032× [1.021, 1.045] of main; without it the remaining changes measure
  0.992× [0.966, 1.013]. Revisit after the Cassandra host-transfer track. Making
  `Spread` a default from size evidence is a later decision.
  (5) Op reduction (P6c): GPU-ResidentMeta (`Execution.meta`, arena-resident
  count/failure/cursor words re-zeroed by the last block; no per-attempt
  allocation, memset or partials buffer) and `DeviceLayout::republish`
  (shared field descriptors, unchanged runs republish the same layout):
  xalan 0.983× [0.763, 0.985], batik 0.983× [0.982, 0.990] of (4); memsets
  17.8K→6.2K and H2D 16.0K→6.2K per xalan fixpoint.
  (6) DL-WaveLanes: `Schedule.streams`, `StratumSchedule.lanes` (WF-DL-Lanes)
  by the explicit rewrite `::cq_gpu_datalog::lanes(streams = K)`; `Pool::lane`
  streams share the allocator (`Pool::shares_memory`), persistent lane workers
  with per-(lane, head) component-scoped arenas, event fork/join before
  admission. Batches stay on their lane pool: each lane waits the next fork,
  which follows the main stream's consumption, so no rebinding is needed.
  Counts identical at K=2/4 and 43/43 relations byte-identical at K=4; peak
  pool memory unchanged (xalan 6224 MiB, batik 4998 MiB at K=1/2/4). Locked
  pairs vs (5): K=2 xalan 1.009× [0.972, 1.072], batik 0.979× [0.976, 1.026];
  K=4 xalan 0.865× [0.852, 0.892] (1.046 → 0.905 s), batik 0.811×
  [0.800, 0.816] (1.947 → 1.580 s). `srd-doop-gpu` records `lanes(streams = 4)`.
  These are the latest recorded 2026-10-31 `srd-doop-gpu` P6b medians here,
  not the initial profile above. The retained SRD fullclock reference is
  xalan 1.01–1.02 s and batik 1.82–2.10 s (the same dated
  `p3b-profile/01-doop-report.md`, backed by
  `experiments/gpu/compare/srd_query_xalan_batik.results.json`).
  The historical SRD clock/configuration boundary is different; this is not
  a fresh paired speedup experiment.
  The ~5.4 µs host setup per hole (occupancy query, module lookup) is the next
  per-hole cost.
  doop-gpu (relational heads, c1b75718) records `roots(spread)` again:
  0.953× [0.923, 0.960] (7 pairs) and, rebased, 0.962× [0.825, 1.049] (5
  locked pairs) of no spread; and `lanes(streams = 4)`: Cassandra fixpoint
  1.668 → 1.205 s, 0.723× [0.702, 0.765] of one stream (5 locked pairs);
  26/26 outputs exact.
  Sort/unique (~0.23 s) stays with P4-GPU.
  **Host setup follow-up, 2026-11-02 (`f796fcd3`):** five locked current-main
  runs gave xalan **0.912135 s** (bootstrap median CI [0.903112, 0.914550])
  and batik **1.599563 s** ([1.596005, 1.608999]), all 43 relation counts exact.
  Against a fresh build of the historical K=4 merge `a1e3423b`, five
  interleaved batik pairs gave 1.602919 → 1.605993 s, paired ratio
  **1.00169 [0.99906, 1.00272]**: no meaningful main regression. The current
  versus Oct 31 batik difference was not reproduced in the paired comparison;
  no bisect was warranted under the agreed 1.03 lower-CI threshold.
  Separate locked nsys captures (all CUDA APIs) attributed xalan/batik
  GPU-idle wall time in ms: setup APIs **83.94/109.36**, sync/readback
  **185.59/183.29**, launches **79.28/118.35**, other CUDA **108.25/163.80**,
  outside CUDA **165.62/236.52**; GPU busy **556.12/1195.94**. Overlapping
  lane APIs are partitioned in that priority order, not summed as wall time.
  Occupancy ran **13,656/18,535** times, function lookup **36,930/60,290**,
  function attributes **67,184/103,458**, device attributes **425,520/648,767**.
  Default trace readbacks were **14,708/19,507**, launches **39,133/62,120**.
  A non-callable-only prepared-launch candidate was rejected (xalan
  1.0011 [0.9783, 1.0855], batik 1.0106 [0.9948, 1.0368]); a diagnostic
  trace showed unchanged setup counts because DOOP holes use callable
  specialization. No benefit is claimed for that discarded design.
  Evidence and preserved binaries: `/tmp/p6b-host/` (`main.json`,
  `historical-batik.json`, `main-all-*-attribution.json`, `program-launch.json`).
  **Retained 2026-11-02: GPU-ProgramLaunch**, rebased against main `20101707`
  after the DOOP codec repair. `Realization.launches` owns preparation:
  one-block or occupancy-derived program envelope. The subsequent DriftWatch6
  cutover gives each fixed kernel specialization one optional slot, resolves
  symbols only on cache misses, and prints definitions/calls from a checked
  final ABI recording live scalar positions, witnesses and preparation entry.
  Context and grid-envelope checks replace repeated driver validation.
  Module/entry slots belong to the query scope and worker context. This cutover
  adds no new measurement evidence. Grid caps, sharing and all count/failure
  readbacks are unchanged. Five interleaved pairs, both locks, phase-only
  fixpoint, no counters/profiler, exact counts in every sample:

  | Workload | Main median (s) | Candidate median (s) | Paired ratio, bootstrap 95% CI |
  |---|---:|---:|---:|
  | srd-doop-gpu xalan | 0.912370 | 0.904628 | 0.99133 [0.98154, 1.00028] |
  | srd-doop-gpu batik | 1.609418 | 1.588257 | 0.98519 [0.97691, 0.98829] |
  | doop-gpu Cassandra | 1.331751 | 1.311806 | 0.98218 [0.95805, 1.02116] |

  Batik is a demonstrated improvement; xalan and Cassandra are statistically
  neutral, not claimed speedups. Retention follows the RootShare criterion:
  a significant batik gain with neutral other workloads. A separate xalan
  mechanism capture reduced setup-attributed GPU idle **83.94 → 48.04 ms**,
  occupancy queries **13,656 → 3,123**, device attributes
  **425,520 → 246,459**, function attributes **67,184 → 46,118**.
  Launches (**39,133**) and readbacks (**14,708**) were unchanged.
  Function lookup calls were unchanged; auxiliary runtime setup is outside
  this query-kernel preparation slice.
  Profile windows use same-thread synchronize endpoints enclosing every
  generated query kernel and agreeing with the phase duration within 0.4 µs;
  summed host API durations are not additive wall time. Evidence:
  `/tmp/p6b-host/final-srd.json`, `final-cassandra.json`,
  `specialized-all-xalan-attribution.json`. Final pair gate times:
  **246.32 s** SRD and **30.27 s** Cassandra (**276.60 s** locked wave).
  Complete sorted tuple exports matched **43/43** relations on both xalan and
  batik, with duplicate-free counts checked against each run; both main and
  candidate Cassandra matched the CPU's complete **26/26** output sets
  (`/tmp/p6b-host/exact-final.json`, `*-cassandra-exact.log`; export/exactness
  gate **539.20 s**). Required GPU Datalog crates plus Sequence-core and the
  launch-cache runtime tests passed under the GPU lock (**353.80 s**, including
  lock wait), as did the SRD exact fixture (**70.83 s**). Root workspace
  (**64.06 s**), quick **15/15** (**84.17 s**), formal inventory
  (**0.28 s**) and public API **29/29** (**38.08 s**, after the intentional
  launch-IR snapshot update) passed. These are gate wall times, not fixpoint timings.
- **P7 Join filters and evidence** across targets (exact min/max and
  membership from runs; published key/row/group statistics for `choose`).
  **P7.1:** typed provider statistic/publication records and host scalar guard
  readiness contracts are implemented (`STR-BuildStats`, `BE-StatsRead`).
  Exact/estimated units and structured immutable build/layout/domain provenance
  remain distinct. Authored evidence carries per-fact source and never supplies
  a uniqueness premise. Input set semantics establishes only full-heading
  uniqueness. No automatic statistics, filter activation policy, runtime
  population, device transfer or committed-round publication is selected by
  this slice.
  **P7.2 merged (2026-11-03, `514ef6af`; advice-only):** CPU run/hash builds
  publish cheap optional occurrence/key/group statistics (`RT-BuildStats`);
  run views borrow ordered keys for exact point/interval tests (`RT-RunFilter`).
  Provider fact `run_filter` records source capability; explicit advice selects
  mutually exclusive `borrowed_filter` or `attached_filter` realizations.
  Borrowing requires a scalar complete stored layout; attached builders use
  `BuildSidecar` through `BuildWithSidecar`. Neither is selected automatically.
  P7.2's built-in optional-statistic activation and unit-sidecar publication
  are not selected by default. Existing main projected filters retain ordinary
  Build, exact Keys::key_count activation and conditional projected construction.
  Borrowed/attached filters and optional-statistic guards require authored advice.
  **Historical construction-repair investigation (now merged):** the content rebase onto `4dd291be` reproduced
  H09 default/radix regressions of 1.12260/1.11745; linker-shuffled P7.2 remained
  slower. Identical counters and CPU perf/assembly locate the cause in optional
  initializer dispatch inside the no-demand HashIndex closure: it changes the
  hot loop's register allocation. A diagnostic isolating dispatch restored the
  baseline loop and was neutral within its five-pair confidence intervals.
  `STR-Construction` now records `Ordinary` versus `Statistics(BuildDemand)`
  on the Structures provider instance, with explicit `statistics_build` support
  and independently checked publication witnesses. Printers consume that
  record, not the presence of a MaxGroup accessor. With no statistic consumer,
  construction selects ordinary `Build::build`; requested row/max work uses `BuildWithStatistics`, with lazy
  initializer selection outside the ordinary `OnceCell` closure. Cheap lengths
  remain available; ordinary runtime Build no longer accumulates statistics.
  **P7.3 cost-model evidence, not an accepted default:** after the H09 repair,
  five interleaved pairs per changed default/radix kernel (SF1, complete exact
  answers each sample, bootstrap 95% intervals, zero major faults) still showed
  H05/H21/d72 losses with P7.2 publication/activation on. Consequently the merged
  advice-only implementation has no P7.2 default activation; its default/radix
  expansions match main `907b9b0d` in the recorded acceptance below.
  The construction/statistic/filter runtime and records remain authored-only.
  Measurements below describe the rejected activation-enabled candidate.

  | Kernel/configuration | P7.2/main ratio [95% CI] | Same-source main shuffle ratio [95% CI] |
  |---|---|---|
  | retail_d19/default | 0.99815 [0.98350, 1.01633] | 0.99782 [0.98422, 1.00630] |
  | retail_d19/radix | 1.02360 [1.00526, 1.03997] | 1.03528 [1.01012, 1.05870] |
  | retail_d27/default | 1.01967 [0.98736, 1.05443] | 1.04087 [1.01716, 1.06349] |
  | retail_d27/radix | 1.02159 [1.01428, 1.02906] | 1.00244 [0.97777, 1.02666] |
  | retail_d72_inner/default | 1.02808 [1.01368, 1.04304] | 1.01667 [1.00704, 1.02688] |
  | retail_d72_inner/radix | 1.03425 [1.01830, 1.05325] | 1.00000 [0.98900, 1.01129] |
  | retail_d85/default | 1.01524 [0.98481, 1.04566] | 0.98497 [0.95532, 1.01896] |
  | retail_d85/radix | 1.01190 [0.99306, 1.03074] | 1.02820 [0.99117, 1.06524] |
  | retail_h03/default | 0.99133 [0.98508, 0.99769] | 1.00043 [0.95487, 1.05127] |
  | retail_h03/radix | 0.99873 [0.95572, 1.03320] | 0.97729 [0.95907, 0.99787] |
  | retail_h05/default | 1.07615 [1.03944, 1.11287] | 0.99716 [0.97321, 1.01847] |
  | retail_h05/radix | 1.00163 [0.99155, 1.01172] | 1.00590 [0.98029, 1.03152] |
  | retail_h07/default | 1.00227 [0.94393, 1.06294] | 1.01840 [0.94447, 1.08904] |
  | retail_h07/radix | 1.02797 [0.99917, 1.06101] | 0.95865 [0.90951, 0.99081] |
  | retail_h07_block/default | 1.03213 [0.95455, 1.10229] | 1.01807 [0.93746, 1.08214] |
  | retail_h07_block/radix | 0.97314 [0.92929, 1.01698] | 1.00011 [0.97986, 1.02046] |
  | retail_h08/default | 0.99708 [0.96937, 1.02413] | 0.99728 [0.94012, 1.03916] |
  | retail_h08/radix | 0.98945 [0.92413, 1.06299] | 0.99473 [0.97958, 1.00611] |
  | retail_h21_positive/default | 1.03748 [1.02133, 1.05363] | 0.97741 [0.96072, 0.99333] |
  | retail_h21_positive/radix | 0.99672 [0.98799, 1.00589] | 1.00806 [0.98650, 1.03194] |

  Exact raw pairs/page faults: `target/p7-repair/paired-final-raw.json`;
  controls use pinned LLD `--shuffle-sections=*=7`. No small speedup is claimed.
  **P7.3 recorded filter policy (veto-only rule, no automatic activation):**
  every `JoinFilter` carries one mandatory Structures `policy` record:
  `abandon 35 of B keys key_count` (main's measured incumbent), `keys ACCESSOR`
  only under authored borrowed/attached advice, or `always` without a positional
  base (SEL-FilterPolicy, checked by STR-FilterPolicy). This replaces the optional
  `abandon` field whose absence implied "always". The approved cost rule may only veto
  the incumbent from recorded evidence (P3.3e counts, filter ranks, P7.1
  publications), with margin 25. Unknown evidence keeps the incumbent at compile
  time, and an unavailable statistic is identity at runtime. **Census**
  (`/data/design-notes/p7-3-design.md`) uses observed compiler output: 297 filter
  records across all routes, 24 on the default route in 10 kernels, plus DuckDB
  SF1 cardinalities.
  - **No departure anywhere:** 0 of 29 TPC-H kernels in any route and 0 DOOP rules.
    Datalog selection has no positional hosts, so there are no candidates.
  - The rule never selects P7.2's optional guard, statistics construction or
    sidecars, so H05/H21 default and d72 radix keep main's code.
  - A literal compile-time "unknown ⇒ no filter" reading would have deleted two
    useful filters: H21 `Lineitem.order_key` (6.0M tests, 5.9% pass) and d85
    `PartSupp.part_key` (800k tests, 4.4% pass).

  P7.4 device publication and P7.5 committed-round identities remain separate;
  this is not completion of all P7.
  **Advice-only acceptance:** expanded gate versus `907b9b0d` passes 46/46
  targets with zero hunks and complete execution evidence. SF0.1/SF1 each pass
  58 default/radix exact-answer and repeated-counter cases; 17 library oracles,
  behavioral/package/quick/formal gates and 29 public-API snapshots pass.
  No GPU execution or automatic-activation performance gain is claimed.
- **P8 Current state (2026-11-03).** Key-partition routing projection is closed
  at the measured stop below; KP-1/KP-2 positional support merged in `925d5f73`.
  Masks versus Tree is closed with Masks retained; the selective proposal is
  not implemented. `delta=k` SRDatalog plans are in progress. Placement spine
  remains conditional on a branching machine.
  **delta=k coarse-order revisit (2026-11-03): census and CPU experiment.**
  `k` is the zero-based original body occurrence, not an IDB ordinal or
  reordered position. Default / field-flow source queries have 82 / 85 rules
  (88 / 91 single-head expansions); 20 / 22 rules contain at least two
  positive IDB atoms, but only 14 / 15 contain at least two **changing**
  same-SCC positions. There are 37 / 38 changing positions in those rules
  (47 / 48 after head expansion), and 9 / 7 authored delta entries
  (12 / 10 expanded), versus zero imports in the committed default.
  Eight of nine / six of seven imported greedy atom orders already equal
  the GPU delta-first occurrence certificate; the exceptions are
  `VPT_LoadField delta=2` / `FieldFlow_rule delta=2`, imported `210` versus
  driver `201`. Equal certificates do not establish equal executable plans.
  SRDatalog's variable order binds individual variables; the importer greedily
  binds whole atoms and is **not a faithful WCOJ plan import**.
  The checked CPU census independently admits every resulting VO: for both
  mini and mini-par, pinning the current written orders (B) leaves all 65 / 67
  recursive-hole VOs byte-identical to default A; imported delta-only C
  changes all 9 / 7 authored entries (12 / 10 split-head holes).
  Therefore CPU timings compare A/C, not redundant A/B; C excludes all six
  base entries and does not import execution pragmas. This is a complete-set
  comparison, not per-position attribution. DL-Incremental already records
  delta identity and occurrence advice; CQ/VO remains executable-plan owner.
  Contract **4 is superseded by 7**: preserve opaque Rust and contracts 1, 5,
  and 7; do not restore expression classification or Sequence replanning.
  The historical imported-order batik GPU observation was 9.1→21.3 s.
  It is distinct from the reverted opacity-classification experiment
  (xalan 2.20→9.35 s, batik OOM); neither establishes per-node causality.
  Full census, checked VO correspondence, source provenance, and reproduction
  protocol: `/data/design-notes/delta-k-design.md`; raw artifacts:
  `/data/mini_linq-wt/delta-k/target/delta-k-census/`.
  **Measurement stopped on user direction:** retained CPU samples are
  exploratory only, not a default/planning decision; no ratios are promoted
  here. Imported occurrence orders were measured before reviewing their
  interaction with evidence roots, L&E phases, Masks, join filters, blocked
  scans, provider selection and admission/canon. A legal checked permutation
  alone does not justify that experiment. Remaining exports had been moved
  to `/dev/shm`; interrupted samples are discarded. No GPU benchmarks ran.
  The new **Plan review** in the design note compares A/C recorded plans and
  expanded Rust before any further measurement. Delta-specific planning must
  use the same evidence and downstream capabilities as the rest of cq-order,
  not an isolated imported greedy permutation. Defaults remain unchanged.
  The completed review finds **four source entries / eight emitted C cases**
  delaying an already-bound array-kind membership until after RSI expansion.
  P3.3e's DL-Fanout reaches every variant but only informs admission; CQ
  `Planning` transports neither that evidence nor per-round counts. A therefore
  uses ordinary Prefix/Coarse, not evidence-driven root selection. Masks is
  enabled but these holes have no qualifying Restrict; no optional L&E split,
  P7 filter or P5 blocked-scan decision was active. Complete CPU expansions,
  Structures, serial Sequence (parallel prints Structures directly), and the
  selected GPU delta-2 realization review support the joint Datalog planner
  handoff. Do not resume isolated-order timing; delta=k belongs in cq-order's
  per-delta planning after the evidence/readiness integration.
  **Key-partition routing projection: no target in the measured CPU suite**
  (2026-11-03, 57216a5c). The preimplementation census covered 28 Tasks/par
  kernels at SF1 with six workers; all complete answers matched DuckDB.
  Across 42 checked stage plans and 129 distinct routing sites, 66 routed
  covers exchanged 34,487,575 rows / **781,900,952 carrier bytes**.
  Existing Structures carrier demand already retains precisely the required
  downstream/canonical/routing columns: independently seeding demand without
  Built/filtered/native-layout requirements found **zero droppable columns,
  zero benefiting rows and zero saved bytes (0%)**. Retain the existing
  implementation; no paired speedup or implementation-gate claim is made.
  `retail_h07_block` was explicitly excluded pending its separately owned
  Positions-capability repair and must be rechecked after that repair merges.
  Per-kernel widths/counts and eligibility rationale:
  `/data/design-notes/p8-keypart-design.md`; raw checked plans, exact answers,
  exchange counters and per-command gate ledger:
  `/data/mini_linq-wt/p8-keypart/target/p8-census/`.
  Census Gate time **583.02 s**, with **1,666.13 s** separate lock wait.
- **P8 Masks as full reducer — measured; default unchanged (2026-11-03).**
  The comparison starts at `57216a5c` and preserves the authored default
  P3.3e/Prefix ordering, advice, downstream passes and sequential machine.
  The experimental planner change is `Masks` → `Tree`, in an isolated source
  copy; it does not relax positional advice, change predicates, assume an FK,
  or substitute a spanning-tree fallback for an unsupported full reducer.
  Nondefault `par` consumers were omitted where their compilation prevented
  building the requested default worker, including the baseline H07-block
  worker. This is **default-route evidence, not a Tasks/par acceptance result**.

  All **29 Masks kernels** and all **19 supported Tree kernels** matched the
  cached DuckDB-CLI SF1 tuple-set oracle on every warm and measured execution.
  There are five interleaved pairs per supported kernel, alternating execution
  order, with one warm execution before each timed execution; the ten
  unsupported Tree kernels retain five exact Masks samples. Loading,
  serialization and exact comparison are outside the query timer.
  Ratios below are geometric means of the five paired **Tree/Masks** ratios,
  not ratios of the displayed time medians. Intervals are percentile bootstrap
  95% CIs (10,000 paired resamples, seed 2119); the aggregate resamples pairs
  within each supported kernel. No result is inferred for excluded kernels.

  The supported-kernel geomean is **1.03541 [1.02493, 1.04551]**: unconditional
  Tree is slower overall, despite substantial H09 and H12 improvements.
  A five-pair **same-source, separately built source-layout control** on H01
  is **1.00020 [0.98309, 1.01702]**, all exact. H18's small apparent improvement
  overlaps that control band and is not claimed as a controlled small win.
  Minor-fault columns are medians during the timed query; all 240 primary
  timed executions recorded **zero major faults**.

  | Kernel | Masks ms | Tree ms | Tree/Masks [95% CI] | Minor faults Masks/Tree |
  |---|---:|---:|---|---:|
  | `retail_d19` | 15.730 | — | positional advice | 470/— |
  | `retail_d27` | 16.768 | — | positional advice | 0/— |
  | `retail_d72_inner` | 158.509 | — | cyclic | 7976/— |
  | `retail_d85` | 20.720 | — | positional advice | 412/— |
  | `retail_h01` | 949.116 | 958.780 | 1.05451 [0.99383, 1.12224] | 138671/138671 |
  | `retail_h02` | 55.728 | 69.739 | 1.23878 [1.21188, 1.26245] | 784/6386 |
  | `retail_h03` | 36.467 | — | positional advice | 2629/— |
  | `retail_h04` | 23.610 | 38.563 | 1.62039 [1.57918, 1.66268] | 467/1208 |
  | `retail_h04_not_exists` | 162.166 | 161.436 | 0.99131 [0.96862, 1.01485] | 25842/25842 |
  | `retail_h05` | 37.536 | — | cyclic | 1117/— |
  | `retail_h06` | 34.103 | 34.483 | 1.00537 [0.96954, 1.04253] | 0/0 |
  | `retail_h07` | 37.265 | — | positional advice | 1769/— |
  | `retail_h07_block` | 40.530 | — | cyclic | 2235/— |
  | `retail_h08` | 32.403 | — | positional advice | 1300/— |
  | `retail_h09` | 1070.091 | 657.670 | 0.60296 [0.57759, 0.62608] | 46846/16506 |
  | `retail_h10` | 241.590 | 246.488 | 1.01951 [1.00646, 1.03266] | 18410/24604 |
  | `retail_h11` | 15.496 | 21.937 | 1.42052 [1.35052, 1.49201] | 659/2026 |
  | `retail_h12` | 110.081 | 39.014 | 0.35447 [0.35141, 0.35756] | 14267/0 |
  | `retail_h13` | 192.648 | 232.703 | 1.20939 [1.20427, 1.21572] | 18903/19477 |
  | `retail_h14` | 47.457 | 50.377 | 1.01274 [0.93052, 1.08000] | 3174/2470 |
  | `retail_h15` | 62.685 | 59.939 | 0.96589 [0.92522, 1.02834] | 2634/2634 |
  | `retail_h16` | 51.884 | 51.177 | 1.08338 [0.95729, 1.19083] | 5867/6006 |
  | `retail_h18` | 1042.303 | 1024.869 | 0.98180 [0.97875, 0.98485] | 82486/63750 |
  | `retail_h19` | 16.815 | 28.439 | 1.70967 [1.69395, 1.72554] | 0/1450 |
  | `retail_h20` | 320.862 | 312.690 | 0.98499 [0.96747, 1.00345] | 21299/21900 |
  | `retail_h21_positive` | 83.398 | — | positional advice | 1773/— |
  | `retail_h22` | 29.884 | 31.644 | 1.05746 [1.04216, 1.07400] | 4607/5183 |
  | `tpch_q06` | 23.368 | 23.142 | 0.99271 [0.98823, 1.00021] | 0/0 |
  | `tpch_q19` | 16.893 | 22.763 | 1.33344 [1.28083, 1.37425] | 0/996 |

  The three cyclic exclusions are the existing Tree domain restriction.
  The seven positional exclusions are **unsupported combinations of advice
  authored for the Masks plan with a different reduced source**, not reported
  compiler bugs. Neither category is silently counted as an exact Tree run.

  **Planners corpus.** All 30 authored scenarios were compiled through the same
  default ordering and downstream sequential stages. Five interleaved batches
  of 1,000 native executions per variant check every result against the
  independently authored exact answer, outside timing. All 30 Masks and 24
  acyclic Tree cases are exact; the other six reject Tree. The supported
  geomean is **1.26056 [1.25447, 1.26566]**. Across the timed batches there were
  two minor faults and zero major faults. These are small authored fixtures,
  not a substitute for the SF1 measurements. The small `cartesian` estimate
  concerns two zero-key existence semijoins, not key/FK pruning; no controlled
  small-improvement claim is made for it. `projection` and `scan` have unchanged
  valuations.

  | Corpus scenario | Masks ns | Tree ns | Tree/Masks [95% CI] |
  |---|---:|---:|---|
  | `barbell` | 684.0 | — | cyclic |
  | `bushy` | 433.0 | 596.4 | 1.30971 [1.19312, 1.38013] |
  | `cartesian` | 196.2 | 190.6 | 0.97326 [0.95632, 0.99050] |
  | `cartesian_first` | 368.6 | 434.4 | 1.17405 [1.16613, 1.18186] |
  | `chain_projection` | 342.5 | 476.4 | 1.39024 [1.37022, 1.40928] |
  | `claw` | 359.3 | 447.4 | 1.25067 [1.23255, 1.27456] |
  | `composite` | 218.4 | 276.9 | 1.27230 [1.26335, 1.28222] |
  | `cycle4` | 406.7 | — | cyclic |
  | `existential` | 150.0 | 152.5 | 1.00793 [0.98042, 1.03091] |
  | `fact_star` | 404.5 | 623.1 | 1.54812 [1.52968, 1.57586] |
  | `fine_triangle` | 250.8 | — | cyclic |
  | `intersection` | 140.0 | 214.2 | 1.53711 [1.52049, 1.55390] |
  | `loomis_whitney_4` | 459.4 | — | cyclic |
  | `oriented_chain` | 402.0 | 498.1 | 1.23592 [1.22223, 1.24759] |
  | `projection` | 103.0 | 101.2 | 0.99150 [0.97632, 1.01026] |
  | `return_trip` | 178.8 | 260.3 | 1.45725 [1.45120, 1.46441] |
  | `scan` | 99.2 | 99.2 | 0.99966 [0.99077, 1.00710] |
  | `self_chain` | 308.8 | 364.6 | 1.17579 [1.16406, 1.18761] |
  | `spade` | 479.4 | — | cyclic |
  | `ternary_guard` | 527.8 | 1112.4 | 2.10602 [2.10270, 2.10935] |
  | `triangle` | 246.5 | — | cyclic |
  | `unary_guard` | 153.3 | 214.7 | 1.40420 [1.39474, 1.41343] |
  | `ya_composite` | 526.1 | 593.6 | 1.13607 [1.12518, 1.14995] |
  | `ya_composite_reduced` | 529.9 | 586.6 | 1.11175 [1.10140, 1.12487] |
  | `ya_deep` | 464.0 | 612.7 | 1.31414 [1.28404, 1.34033] |
  | `ya_deep_reduced` | 470.8 | 605.2 | 1.28736 [1.28390, 1.29133] |
  | `ya_delivery` | 523.8 | 574.2 | 1.12983 [1.10903, 1.15313] |
  | `ya_delivery_reduced` | 490.5 | 578.5 | 1.16784 [1.15414, 1.18140] |
  | `ya_disconnected` | 105.0 | 148.1 | 1.41661 [1.39824, 1.43260] |
  | `ya_disconnected_reduced` | 104.2 | 137.5 | 1.31630 [1.27767, 1.34828] |

  **Where the large wins come from.** Separate counter-enabled workers, not
  the timed binaries, matched the same exact oracle. Counts below describe
  observed intermediate rows, not assumed keys or referential integrity.

  - **H09:** all seven occurrences have recorded local filter rank 3; Masks
    emits no reductions. Tree's `Part ← GreenNames` removes **189,336** rows
    (200,000 → 10,664). Its subsequent `Lineitem ← Part` removes **5,681,811**
    rows (6,001,215 → 319,404) **before the payload index keyed by `part_key`**:
    that index receives 319,404 rows instead of Masks' 6,001,215.
    `Supplier ← Lineitem` removes 135 (10,000 → 9,865),
    `PartSupp ← Lineitem` removes 757,376 (800,000 → 42,624), and
    `Orders ← Lineitem` removes 1,212,510 (1,500,000 → 287,490), shrinking their
    final providers too. The other seven Tree edges remove **zero** rows:
    `Lineitem ← Orders` (6,001,215), `Lineitem ← PartSupp` (6,001,215),
    `Nation ← Supplier` (25), `Supplier ← Nation` (9,865),
    `Lineitem ← Supplier` (319,404), `Part ← Lineitem` (10,664), and
    `GreenNames ← Part` (10,664). These are all unfiltered in the recorded
    rank sense; `GreenNames`' actual selectivity is not a recorded count or
    filter rank. Total copied rows fall **6,011,215 → 989,405**, while index
    builds rise **6 → 18** and enumerated rows **9,160,712 → 9,990,996**.
  - **H12:** the `Matches` scope has filtered outer `Lineitem` (rank 1, count
    unknown) and unfiltered inner `Orders` (rank 3, recorded count 1,500,000).
    Local predicates leave 30,988 lines. `Orders ← Lineitem` removes
    **1,470,901** orders (1,500,000 → 29,099) **before the order-key payload
    index**; Masks builds that index from all 1,500,000 rows. The reverse
    `Lineitem ← Orders` removes **zero** of the 30,988 lines. Thus the large
    benefit is propagation from the filtered outer into the build, not
    evidence that the reverse unfiltered support is useful. In later
    unfiltered scopes, `HighPriorities ← MatchingLines` keeps 2 rows,
    `MatchingLines ← HighPriorities` takes 30,988 → 12,402 before the
    high-priority group, and `HighCounts ← Totals` / `Totals ← HighCounts`
    each keep 2 rows. Total copied rows fall **1,500,002 → 60,089**; builds
    rise **4 → 7**, enumerated rows **7,594,191 → 7,625,181**.
  - **Contrasts:** H04 already builds only a 57,218-key membership set and
    copies zero payload rows. Tree reduces 3,793,296 filtered lines to
    144,869 and 57,218 orders to 52,523, but adds **197,392 copied rows** and
    raises builds **1 → 3**, losing 62%. H19 reduces filtered Part
    7,296 → 3,416 and filtered Lineitem 128,371 → 4,694, yet materializing
    those 128,371 lines raises copied rows **7,296 → 131,787**, builds
    **1 → 2**, and loses 71%. Shrinking an intermediate alone is not a cost
    justification. H18 reduces Orders 1,500,000 → 57 and Customer
    150,000 → 57; its two reverse edges remove nothing. Copied rows fall
    1,650,000 → 228, but its whole-query small timing estimate is not a
    controlled win.

  **Docs-only selective proposal and census.** A simple empirical predicate
  over facts already recorded does separate the two large winners in this
  census. Consider an acyclic scope for which the measured Masks valuation
  contains no reductions, then select Tree if either:

  1. **A:** every positive occurrence has rank 3, the outer has a declared
     count, and some nonouter occurrence has a declared count larger than the
     outer's; or
  2. **B:** there are exactly two positive occurrences, outer rank is at most
     1, inner rank is 3, and the inner has a declared count of at least
     **1,000,000** rows.

  Otherwise retain Masks. Unknown counts stay unknown, never zero or an
  inferred TPC-H cardinality. No relation names, Rust syntax, type inspection,
  inclusion dependencies or FK assumptions occur in this predicate.
  A selects **H09 / Query** (outer Part 200,000, inner Lineitem 6,001,215,
  all ranks 3); B selects **H12 / Matches** (inner Orders 1,500,000,
  outer rank 1). It selects **zero of the other 27 TPC-H kernels**, including
  every measured loser and every excluded combination, and **zero of the 30
  corpus scenarios**, whose inputs supply no count evidence. H04 fails the
  rank conditions, H19 has no applicable recorded count, H14's inner count is
  200,000, and H18 has three positive occurrences, not two.

  This is a **proposal, not an implemented or validated default**. The
  one-million cutoff is an empirical SF1 discriminator, not an established
  scale-independent cost boundary. Counts/ranks do not record filter
  selectivity or whether lowering will build payload versus membership;
  A does not prove that an unfiltered smaller relation removes any rows.
  The measured H12 whole-kernel Tree binary also reduces later stages,
  whereas this scope-local proposal selects only `Matches`; its exact
  mixed-policy performance has not been measured. A future producer would
  have to record its reduce/skip selection and evidence in VO; legality
  checkers would validate those facts and semijoin obligations, not replay
  the producer, and printers would only consume the recorded decisions.
  **Masks stays default.** No per-edge implementation is introduced here.
  Proving an unfiltered key/FK edge non-reducing would require a declared
  inclusion-dependency fact: that language addition is outside this slice.
  Existing `R-Masks`, `CK-Masks` and `Plan-Prefix` in `passes.tex`/`rules.tsv`
  remain unchanged and continue to describe the code.

  **Gates and retained evidence.** At the assigned snapshot/layout, root,
  planners and rewrites workspace tests passed; quick was **15/15**, formal
  inventory passed, API was **29 unchanged**, and all **46 CPU expansions
  versus `57216a5c` were identical**, zero unexplained hunks. These gates
  describe the retained compiler, not a claim that unsupported Tree
  combinations passed. Gate time / lock wait (seconds): root **219.511 /
  <0.001**; planners **441.578 / 431.351**; rewrites **88.350 / 38.317**;
  quick **264.226 / 380.536**; expanded, seven bounded commands, **915.546 /
  1111.089**; formal **0.566 / <0.001**; API **70.773 / <0.001**.
  Primary SF1 workers took **237.374 / 18188.408** across 240 commands;
  control **27.088 / 1237.741**, counters **12.167 / 75.965**, corpus timing
  **0.164 / 423.620**. Including experiment builds, unsuccessful preliminary
  worker builds, corpus harness corrections and census commands, the
  per-command ledger sums to **3779.689 seconds of gate time** and
  **26131.028 seconds of lock waits**; these overlapping-command sums are
  not elapsed session time. Shared build locks and exclusive timing leases
  prevent build/measurement overlap.
  Raw local evidence is in the `p8-masks` worktree's `target/p8/`:
  `summary.json`, `samples/`, `control/`, `corpus.json`, `counts/`, `plans/`,
  `census.json`, `proposal-census.json`, `expanded/*/report.json`, and
  `gates.jsonl` (exact commands, statuses, gate time and wait per command).
- **Open issues (2026-10-31).** (1) `doop-gpu` (26-output DOOP) fails at base
  67d76822 with 220 P-Decompose unmet catalog obligations (DeviceLayout
  membership at host level). (2) **Resolved by closed capture (see
  Progress).** Counters test
  `empty_negative_keys_test_existence_without_building_indexes` failed since
  `0124315f`: `cq-order` `ready()` placed a closed binding first, so a positive
  empty-key probe built an index. A query-closed `let`/`if` is now a host
  capture, never a heading column, and the test passes. (3) Row-granular streaming of CQ
  answers has no realization without a new construct; the two facade
  streaming tests stay ignored. (4) `compiler.tex` does not build
  (`backend_ir.tex` `\inferrule*` error). (5) Datalog outputs are sets in
  layout order, no longer insertion order. (6) ADM-InPlan is implemented for
  CPU Datalog (P3.3 slice C: FK-Known, DL-AdmitOnce, `known(admit)` /
  `known(at(level k))` advice). **CPU default decided by recorded evidence
  (P3.3e):** an unadvised hole of a planned queue is in plan iff a later read
  is recorded `multiply` (DL-Fanout over source keys); DOOP records keys and
  no hole is in plan there (identical code). Still open: GPU Datalog records
  `admit` for every hole and rejects in-plan advice until a follow-up after
  P6c. (7) The LianJie repo
  split is undecided and out of scope here. (8) CPU TPC-H h10 1.116x and h19
  1.168x of M0 (h14 1.028x) with identical instructions per query: glibc
  heap layout. M0's loader left long-lived allocations that kept the heap top
  from being trimmed, so each repeated run reused the freed index tables; the
  P3c interning loader leaves a leaner heap, glibc trims it after every run,
  and the next run re-faults the hash table and arena (h10 351 → 14.5K faults
  per query). M0 itself swings 350–20.5K (h10) and 5.9K–16.7K (h12) faults
  with the data-path string length. Recorded as a repeated-run allocator
  effect; benchmarks should report page faults beside timings. Index
  footprint reductions (P4 compiler selection) are the real lever.
  (9) **Closed (2026-10-31):** `cq-ext-recursion` heads are authored plain
  relational variables from positive atoms (`EXT-ActiveDomain`), not recognized
  `*x`/`.clone()` Rust. Computed heads still fail the active-domain check;
  free variables alone cannot prove finiteness (`*x + 1`). Context records
  per-rule/head borrowed/owned columns from the defining binder
  (`EXT-AnswerOwnership`), with checked uniform Clone realizations in owned CQ
  hole heads before planning/canonicalization; the renderer decides nothing.
  (10) **Closed (2026-10-31):** invariant helpers project positive-input
  variables in column order (`DL-InvariantProjection`). The `owned_column`
  primitive-type classifier is deleted. Existing `DL-AnswerOwnership` records
  the copies for owned storage, including `String` and aliases; no Rust/type
  recognition or new Datalog printer decision is involved.

### Review open issues by owner (2026-11-01)

This is the historical review of `17a14b93`, not a current open-issue inventory.
Paths and lines refer to that tree; later cutovers below supersede resolved items.

1. **Checker owners — checker reruns producer for plan decisions.**
   Staging compares recorded additions with `decisions(source, ...)`
   (`passes/cq-staging/src/lib.rs:177-181`); CK-Transfer recompiles reductions
   and transfers them (`features/planners/plan/src/lib.rs:425-435`);
   CK-EvidenceLe calls `dominating`, which calls the ranked read planner
   (`passes/cq-order/src/evidence.rs:285,199,130-167`); GPU Datalog
   `check_device` compares with `realize(restore(...))`
   (`experiments/gpu/datalog/src/storage.rs:347`). Replace producer replay
   with checks of recorded facts in the owning tracks. The formal text
   describes today's replay rather than claiming independence.
2. **Runtime/provider owners — autoref tiers (resolved after this review).**
   The reviewed tree selected membership/build capabilities through reference
   tiers and AutoDense. The DirectCalls cutover removed both; current
   `runtime/cq-storage/src/dispatch.rs` exposes direct functions, and selection
   records native or fallback reads. See `STORAGE.md` §13.3.
3. **CQ → VO / relational lowering owners — ElabFix done.** Branch spanning
   trees are deleted and predicate attachment targets are recorded in VO by
   `cq-order` (`passes/cq-order/src/restrict.rs`, O-Restrict) under the
   order-independent H-Local rule; `cq-relational-lowering` (RL-* rules)
   applies only recorded restrictions. A hoisted block's finite domain is the
   VO prefix that binds its free columns (WF-BlockPlacement), not a
   written-order join, and the checked RA rewrites live in `cq-ra-rewrites`.
4. **GPU runtime owners — historical unrecorded runtime work.** The reviewed
   tree chose run geometry inside the runtime. `DeviceState` now records
   offset/construction/read and directory facts as well as merge tile, block
   and permutation threads, consumed by explicit `RadixRunParams`
   (`experiments/gpu/datalog/src/storage.rs`, `datalog-runtime/src/radix_run.rs`).
   The earlier geometry-only change was `14d76950` (2026-11-02).
   The original lane-attribution concern is not
   independently closed by that geometry change.
5. **Datalog fork/round owners — plan-argument correspondence.** Authored
   order forwarding is implemented, but fork synthesizes `no_reduce`,
   `order`, `smallest` and `known` argument tokens
   (`features/datalog/passes/fork/src/lib.rs:643-671,779-795`). The pipeline
   and planning ordinal are stored in each hole (`:797-805`); the missing
   item is independently checked structured correspondence for those
   arguments, not absence of the emitted token stream. Fork's top-level
   checker delegates to the Round contract (`:95-108`).
6. **Test owners — wording/implementation pinning.** Provider tests compare
   token-string type spellings
   (`features/rewrites/tests/sorted_indexes.rs:21-36`), so representation
   refactors can churn tests without changing semantics. Audit such
   assertions separately from intentional public API, expanded-Rust and
   Round fixture goldens; retain semantic assertions where spelling is not
   itself the contract.
7. **Resolved: declaration identity owners — logical-type ids (contract 7).**
   `cq_syntax::LogicalTypeId(pub u32)` records logical declaration identity.
   Relation and provider declaration boundaries intern omitted ids once by
   opaque spelling, preserve explicit ids, and keep explicitly different ids
   distinct even for equal spellings. RA/Physical rows carry sparse
   `logical_type_ids`; Physical computed outputs use their result declaration's
   id, while computed opaque types without a declaration remain unknown.
   Sequence `StructuresProof` / `codec_obligations`, GPU `Codecs::get` /
   `record`, and GPU Datalog `Symbols` consume those ids for codec lookup.
   Source/provider logical-id mismatches and conflicting codec ids for one
   logical id reject; Rust types are printing payloads after interning.
   Datalog advice uses `symbols([logical_id] Logical => [codec_id] Codec)`,
   with optional ids resolved at the declaration boundary. Runtime encoding
   laws remain premises; this resolution records no fresh gate evidence.

## Progress

### Integration ledger: 2026-11-03 (`57216a5c..dbe4c2cb`)

Every first-parent commit in the requested range is listed below. These are
integration dates; benchmark dates and original measurements remain historical.
The older progress entries following the ledger retain their original context.

| Date | Commit | Merged change / evidence boundary |
|---|---|---|
| 2026-11-03 | `9e8a89cd` | Shared build/test timing lock and exclusive measurement gate documented; refined by `c5b1bd71` below. |
| 2026-11-03 | `61582bc0` | Rejection tests use failure, structured facts and rule IDs rather than diagnostic prose; no tests deleted. |
| 2026-11-03 | `cda68a77` | P8 routing-projection census closes at a measured stop: 66 exchanges, 782 MB carriers, zero droppable SF1 columns. |
| 2026-11-03 | `fa7c9372` | Declared Positions capability repairs `retail_h07_block` parallel compilation; positional-selection regressions and workload methodology updated. |
| 2026-11-03 | `4dd291be` | External fixture consumes recorded provider token/type; missing SRD lockfile supplied for locked vendoring (later consolidated). |
| 2026-11-03 | `925d5f73` | KP-1/KP-2 preserve native positional Selection on Tasks and supply sharded flat Build as a singleton cover. |
| 2026-11-03 | `545ff640` | Local GPU execution bundles and disposable RunPod sessions; pinned HF volume seeding and 60-minute deletion watchdog. |
| 2026-11-03 | `4bf6555e` | Bundle Python benchmark drivers; `--gate-bundles` and `BUNDLE_DIRS` enable cross-bundle interleaving. |
| 2026-11-03 | `907b9b0d` | Consolidate 28 workspaces into root, GPU and Mac; retain packaging isolation, explicit regression packages and non-instrumented root defaults. |
| 2026-11-03 | `a2936f47` | P8 Masks/Tree comparison closes: Tree/Masks 1.035 [1.025, 1.046]; Masks stays default, selective-count proposal remains design. |
| 2026-11-03 | `2ae9c34a` | Benchmark-only `--no-tests` bundles and `--no-same-owner` extraction for the RunPod volume. |
| 2026-11-03 | `cbab9ab7` | Bundle `click` with HF CLI; `typer-slim` alone omitted that dependency. |
| 2026-11-03 | `6331301d` | Historical CPU paired report: SF1 default 0.908 [0.902, 0.917], par 0.820 [0.800, 0.840] versus `c1b75718`; pod benchmark driver added. |
| 2026-11-03 | `71887d6b` | Pull pod results through tar over SSH, avoiding recursive scp's rejected `/.` source. |
| 2026-11-03 | `8a38bdf1` | P5 S2/S3 checked blocked CPU scans, Q06 authored opt-ins; retain parallel Bytes evidence without serial/default promotion. |
| 2026-11-03 | `d2bad61f` | Bundle prebuilt DuckDB CLI and export `DUCKDB_CLI` for pod exactness oracles. |
| 2026-11-03 | `a591b050` | Put bundle binaries on `PATH`; record benchmark ceiling failures while still checking exactness. |
| 2026-11-03 | `ab40cbaa` | Historical RunPod TPC-H report: SF1 1.085, SF10 1.023 (four SF10 pairs). Terminal-download boundary confounds these ratios; not current regression claims. |
| 2026-11-03 | `61af703a` | DriftFix6: ordering readiness, async empty publications, lazy owner lifetimes, recorded active bits/logical IDs, checked kernel ABI and cache-miss launch resolution, feature-gated radix counters; GPU gate invocation repaired. |
| 2026-11-03 | `f6bf1d04` | Pod DOOP GPU xalan/batik 0.239/0.724, exact but low confidence: same-binary spread up to 10x. No live SRD toolchain or prepared SRD GPU corpus on the pod. |
| 2026-11-03 | `b35d9da4` | Prune earlier pod session directories; remove own bundles after successful result retrieval to protect the 40 GB volume. |
| 2026-11-03 | `ce52ccee` | DriftFix7 restores specific rejections/behavioral tests, labels provisional/unconfirmed benchmarks, gates workspace-split coverage and documents root default members. |
| 2026-11-03 | `c5b1bd71` | Shared lock per command, never around multi-command runners. GPU timing holds only GPU lock, with rotating interleaving/loadavg; near-neutral claims require CPU-exclusive confirmation. |
| 2026-11-03 | `514ef6af` | P7.2 merged advice-only: recorded Ordinary/Statistics(BuildDemand), statistic builds and borrowed/attached filters. P7.3 automatic activation remains design. |
| 2026-11-03 | `dbe4c2cb` | Final GPU TPC-H downloads reported separately as `download_ms`; corrected SF1 0.948 [0.946, 0.949] versus `c1b75718`, exploratory three-pair evidence. H03 1.141 publication-write layout remains under investigation; no corrected SF10 claim. |

Evidence for benchmark rows: [paired report](../experiments/gpu/compare/bench-2026-11-02.md),
[boundary samples](../experiments/gpu/compare/gpu-tpch-boundary.results.json),
and the P5/P8 step entries above. Current work is P4-GPU's remaining layout
work, P5's retained authored Q06 parallel Bytes route and subsequent integration,
P6 slice C, P7.3 design and P8 `delta=k`; these are not additional merged results.

### Earlier progress and historical measurements

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
  17 built-radix), in addition to 29 default checks. Current formal inventory
  (checked 2026-11-01 at `17a14b93`): 386 rules, 413 conformance records,
  30 anchors and 520 decode-site markers. This inventory check validates
  registered locations, not compiler conformance.

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
- Phase 3 done for ordering; Masks remains the bottom-up Yannakakis pass
  restricted to filter-carrying inner subtrees, not a full reducer;
  `passes.tex`, `STATUS.md` and `cq-reduce` say so, and `Tree` remains the
  unconditional full reducer on acyclic scopes. P8 above records the measured
  default-route cost answer and the unimplemented evidence-only proposal;
  this remains a cost question, not a correctness fix.
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
  native keyed layouts (`cq-sequence-plan/src/catalog.rs`).
- P3.3-lite removes automatic Sequence replanning rather than demoting its
  ordering authority. Contract 4 is superseded by contract 7; expression
  classification and Sequence join replanning must not return. Open: remeasure
  the `delta=k` SRDatalog coarse-plan translation that regressed batik GPU,
  under the current CQ/VO ordering authority (see P8).
- M0 baseline (P3.3-lite + transitional P3c sort canon), 2026-10-30.
  Expanded gate vs `0072f5fb`: every target differs, 518 hunks, all in the
  canon realization (`__cq_seen` HashSet → `dedup::sort`); P3.3-lite adds no
  CPU hunks. Root workspace tests pass; two first-answer streaming tests are
  `#[ignore]`d with owner P3d track A. Regressions 119/119. CPU TPC-H SF1:
  all 29 configurations exact vs DuckDB. Timing is **unpaired** (3 runs vs
  `cpu_final_sf1_paired.tsv`): h13 0.55, q06 0.66, h15 1.60, h20 1.33,
  h01 1.15; paired timing is owed by track A, which replaces this
  realization. GPU H04 SF1 automatic 1.213 [1.169, 1.225] is accepted: the
  deleted `lookup_first` hid it; the preserved order materializes filtered
  Lineitem (dispatches 1 → 2). **Correction (P3.3, 2026-10-31):** recovering
  it was a CQ → VO ordering decision (O-EvidenceLe streams Lineitem and reads
  Orders through its key, one dispatch), not grading: H04 SF1 automatic is now 0.661
  of `c1b75718`, without a revived transfer. `cq_storage::dedup::sort` defers work to the first
  `next()`, so an undemanded result evaluates nothing.
- Closed capture (A-Capture, A-CaptureRef, WF-Capture, IF-Capture), on
  `6bf2ca20`, 2026-10-31: CQ normalization records each query-closed
  `let`/`if`, closed head operand and closed aggregate argument once as a host
  capture; union branches keep their binders as `let p = capture c` /
  `if capture c`. RA and Physical carry and check the list; printers bind it
  once before any loop. Open issue (2) is resolved: the Datalog counters suite
  passes 9/9, including
  `empty_negative_keys_test_existence_without_building_indexes`. Regressions
  119/119; the 11 `features/datalog-parallel` targets needed their
  `Cargo.lock` updated for the new `cq-datalog-test-passes` → `cq-lang-cq`
  edge. CPU TPC-H SF1 `default,par`: 58/58 configurations exact against
  DuckDB. CPU DOOP xalan `--check`: `mini` and `mini-keyed` match Ascent on
  all 26 outputs. `doop-gpu` cassandra: 26/26 outputs exact against
  `doop-mini`. GPU TPC-H SF1 advised and automatic: 52/52 exact. Expanded gate
  against `6bf2ca20`: 42 targets identical; `correlated_exists`, `retail_h12`,
  `tpch_q06` and `tpch_q19` differ in 159 hunks (77 distinct), all reviewed
  in `tools/expanded_gate_classes_closed_capture.json`, which contains the p4
  rules plus 54 `closed-capture-binding` rules (intended: the capture binding
  removes the empty-key build and its `seek`/`lookup(&())` driver) and 23
  `closed-capture-renumber` rules (refactor: generated ordinals shift). 0
  unexplained. With imported exactness and counter evidence
  (`--verified-results`) for the four differing targets, the gate exits 0.
  Paired timing: 5 interleaved AB/BA process pairs, median of
  3 runs each; ratios are new/base with a bootstrap 95% CI. Page faults are
  whole-process minor faults.

  | Target | Ratio [95% CI] | `index_builds` base → new | Minor faults base → new |
  |---|---|---|---|
  | `tpch_q06` | 0.865 [0.844, 0.880] | 1 → 0 | 78.2K → 75.5K |
  | `tpch_q19` | 0.758 [0.724, 0.759] | 2 → 1 | 88.6K → 86.1K |
  | `retail_h12` | 0.971 [0.940, 1.003] | 7 → 4 | 101.9K → 102.0K |
  | `correlated_exists` (synthetic, A=4000, S=400K) | 1.009 [0.945, 1.021] | 4000 → 0 | 2460 → 2459 |

  `rows_tested` is unchanged on every target. The `correlated_exists` time is
  dominated by its per-outer-value scan of S (1.74G rows tested). No CI lies
  entirely above 1.0.
- C7Provider C1 (contract 7, provider identity), 2026-11-01. Providers are
  recorded `@token as Type` instances declared once in the `providers` table
  (STR-Provider, STR-Routed, PL-Catalog); base declarations live in
  `cq_lang_structures::provider::base`. Closed audit rows
  (`/data/design-notes/c7-audit.md`): A-42 positional supplementary filter,
  A-43 publication routed, A-44 key_partition `Sharded`, A-45 catalog
  matcher, A-46 catalog rewrites/placement, A-47 `supports[keys]`,
  A-48/49/50 cq-print `Vec`/`HashSet` spellings, A-129 rewrites indexes,
  A-130 rewrites structures, A-131 sequence-tasks levels, A-132 workloads
  `HashIndex` suffix. C2 closed A-126..128 (below). G1/G2 implementation
  closes A-138..140 below; its gates are not included in these C1 results.
  Gates: expanded gate vs `c1b75718` passes 46/46 with identical expansions
  (29 runtime-changed targets verified by TPC-H SF1 29/29 DuckDB-exact,
  default and par, counters unchanged); DOOP xalan `--check` ok; regressions
  119/119; root tests 1,207, sequence-tasks 63, rewrites 197, workloads
  2,308 passed.
- C7Provider C2 (contract 7, Datalog Storage/Round provider identity),
  2026-11-01. Storage layouts, delta/next buffers and Round key sets record
  `@token as Type` instances; layouts and key sets record `hasher { fx |
  keyed | none }`. Storage `Program.tokens` declares each token once
  (spelling function, protocol, admission, hashed, task hasher); the
  contract checks token protocol, hasher/`hashed` agreement and each
  spelling against its token's spelling of the recorded key/payload schema
  (WF-DL-Storage, DL-LayoutProtocol). Base tokens `hash_set`, `hash_map`,
  `rows_vec` (dl-storage); `sharded_layout`, `run_layout` (partition);
  `device_layout`, `device_view` (GPU device storage, verbatim, compile-only).
  Closed audit rows: A-126 hasher rewrite (hashed tokens respelled from
  schema), A-127 partition (token identity conversion, CK-DL-ShardStorage;
  non-bulk layouts admitted by declared protocol), A-128 parallel print
  (task hasher and admission from declared token facts,
  DL-ShardedAdmission). Gates: expanded gate vs `c395fe83` 46/46 identical;
  DOOP CPU engines (`doop-mini`, `-keyed`, `-par`, `-par-keyed`, oracle
  engines) expand identically before/after; DOOP xalan and batik
  `--check` all six engines ok; GPU `cq-gpu-datalog-core` tests and
  `srd-doop-gpu` exactness pass.
- P3.3e CPU evidence and Datalog fan-out (R-TpchEvidence, DL-Fanout,
  WF-DL-Evidence, FK-Known), 2026-11-01. CPU TPC-H `default`/`par` record
  dbgen SF1 counts and TPC-H primary keys; only `retail_h04` changes plan.
  Paired timing (5 locked AB/BA pairs vs `c395fe83`, median of 3, bootstrap
  CI, SF1 exact):

  | Target | Ratio [95% CI] | Minor faults base → new | Counters base → new |
  |---|---|---|---|
  | `retail_h04` default | 0.565 [0.543, 0.598] | 67.9K → 38.5K | lookups 57,218 → 3,793,296; distinct insertions 3,793,296 → 57,218 |
  | `retail_h04` par | 0.229 [0.213, 0.248] | 151.6K → 40.8K | index builds 64 → 1; hashes 3.90M → 145K; same lookup/insert swap |

  Every other TPC-H target: identical counters at SF1 `default,par`.
  DOOP: keys recorded in the mini and SRD sources; `doop-mini`,
  `doop-mini-keyed`, `doop-mini-par`, `doop-mini-par-keyed`, `srd-doop-mini`
  and `srd-doop-mini-par` expand byte-identical to `c395fe83` (no hole in plan
  by default), so no DOOP timing was needed. Gates: expanded gate vs
  `c395fe83` 46 targets, 17 identical, 28 differing only in the inert
  `evidence_query` re-export (`p33e-evidence-export`, refactor), `retail_h04`
  7 intended `default`/`par` hunks (`p33e-cpu-evidence`) in
  `tools/expanded_gate_classes_p33e.json`; 0 unexplained, exit 0 with
  imported exactness/counter evidence. CPU TPC-H SF1 `default,par` 58/58
  exact on both trees; DOOP xalan `--check` ok for `mini`, `mini-keyed`,
  `mini-par`, `mini-par-keyed`, `ascent-par` (26 relations); regressions
  119/119; root 1,217, datalog 304, datalog-parallel 107, workloads 2,272,
  planners 35 passed. `passes/cq-order` is unchanged, so GPU automatic plans
  are unchanged.
- C7Provider G1/G2 (contract 7, GPU native providers), 2026-11-01:
  implementation closure of A-138..140. Provider declarations record native
  ABI, contiguous-range eligibility and resident-column bounds; Host instances
  record ordered logical/storage input columns, key count and optional codecs.
  GPU binding consumes these facts and carries input schemas into Built
  instances instead of recovering ABI, layout or encoding from Rust types.
  Sequence publication reads declared contiguous-range eligibility directly,
  without a target type-predicate callback. Structures checks schema cardinality,
  required native Host schemas and declaration consistency. Round-trip,
  mutation and opaque-alias coverage passes. Builds: advised/automatic
  `cq-gpu-tpch`, `srd-doop-gpu`, `doop-gpu`. GPU suites: Sequence 52,
  Sequence core 40, Datalog core 22, Datalog facade 3 (one timing probe
  ignored), SRD exactness 1 checking all 43 relations. Structures/Sequence
  language suites: 166. GPU TPC-H SF1 advised and automatic each 26/26 exact;
  Cassandra all 26 complete tuple sets match CPU. GPU executions held the GPU
  lock; no timing improvement is claimed.
  Identity vs `c395fe83`: 81 advised and 161 combined advised/automatic
  Sequence dumps and kernel modules match after erasing only added provider
  facts and IR-derived module hashes. All 138 advised / 304 combined PTX entry
  instruction bodies, ten helper bodies and shared declarations match after
  compiler-generated symbol renaming, function-order and comment normalization;
  raw PTX is not byte-identical. Evidence is in `target/c7-evidence/`.
  Expanded Rust matches on all 46 targets (the interrupted `aggregate_stats`
  snapshot passed separately). The full gate still requires CPU verification
  for 29 conservative runtime-fingerprint changes: adding an empty instance
  schema to `cq-sequence-tasks` catalog constructors changes the crate hash,
  although every emitted expansion is identical. This verification was not
  run in this slice; do not report the full gate as passed.
- InternedStrings (strings are always interned) and P5 S0, implemented on
  top of closed capture: `ColumnDecl ::= c: T ["in" "(" Expr ")"]` records a
  column code domain; normalization rule A-Domain translates closed operands
  at domain columns into host-once `cq_storage::domain::{exact,lower,upper}`
  captures (`<=`/`>` via `upper`, ranges via `lower`/`upper`, literal
  membership as an `Or` of equalities) and WF-Domain rejects cross-domain
  joins/comparisons. `cq_storage::{CodeDomain, OrderedCodeDomain}` replace
  `StringValue`/`relational_string_eq`/`relational_str_cmp`/`StringLiteral`;
  `StringDictionary` implements both, `Dictionaries::domain` replaces
  `Dictionaries::literal`. Printer string branches, the literal-value union
  disjointness proof (option (b)), the `Expr::Lit` tests in
  `constant_group` and GPU closed extraction, and fork literal key embedding
  are deleted. P5 S0 done: 17 TPC-H sources (retail d19, d27, d85, h02, h03,
  h05, h07, h07_block, h08, h10, h11, h12, h16, h19, h20, h21_positive;
  tpch q19) write literals in atoms/`where` over declared
  `dictionaries.domain("col")` domains (h19's opaque `if` is a relational
  `where`), and typed_* workloads intern text with registered publication
  decodes. Formal rules, conformance, STORAGE §13.1 and anchors updated
  (`doc/formal/check.py` passes). Point reads: `Select::constant_keys` took
  only literal equalities, so the A-Domain captures had lost point-read
  eligibility (closed operands never had it). It now returns `Term`s for
  every column = constant conjunct; `host_point` reads a key with a closed
  operand through `dispatch::BorrowedPointRead` (owns the borrowed key once
  per read via `OwnKey`, else scans). The facade dispatch tests are back to
  1 read / 0 scans on a code-keyed `HashMap`, plus a host-parameter test.
  Gates against main `a1e3423b`: root 1214, every `features/*` workspace
  (datalog 292, datalog-parallel 105, own-dialect 239, rewrites 197,
  workloads 2308, …) and `experiments/gpu` sequence 90 tests pass;
  regressions 119/119; CPU TPC-H SF1 `default,par` 58/58 exact against
  DuckDB; DOOP xalan `--check` mini and mini-keyed 26/26. Expanded gate: 28
  targets identical modulo runtime and 18 targets differ (the 17 S0
  targets and `typed_line_filter`), 826 hunks in 222 distinct hunks. All are
  reviewed in `tools/expanded_gate_classes_interned_strings.json`:
  `interned-domain-capture` 158, `interned-renumber` 42, `interned-h19-where`
  12, `interned-capture-collect` 8 and `interned-typed` 2, so 0 are
  unexplained. No union target changed under option (b), and no target
  needed a map point read. Verification for the runtime-changed targets
  (`--verified-results`) is not imported yet. Paired timing (locked, 5
  AB/BA pairs, median of 3, bootstrap 95% CI) gives new/base ratios. Only
  h19 changes counters: `hash_computations` 594597 → 149999, `lookups`
  214377 → 128371, `rows_tested` 7.07M → 7.30M. The other targets'
  counters and all answer rows are identical: d19 1.004 [0.977, 1.021],
  d27 1.000 [0.974, 1.014], d85 0.983 [0.974, 0.987], h02 1.026
  [0.982, 1.070], h03 0.930 [0.872, 1.020], h05 0.967 [0.937, 0.989],
  h07 0.946 [0.919, 1.009], h07_block 1.053 [0.970, 1.135] (rerun 1.085
  [0.981, 1.114]), h08 0.993 [0.970, 1.016], h10 1.005 [0.967, 1.033],
  h11 1.010 [0.972, 1.030], h12 1.008 [0.982, 1.016], h16 1.073
  [0.748, 1.189] (rerun 0.985 [0.932, 1.165]), h19 0.315 [0.276, 0.341],
  h20 1.010 [0.972, 1.048], h21_positive 0.983 [0.974, 1.026], q19 0.961
  [0.819, 1.007]. No CI lies entirely above 1.0, so option (b) stands.
  The typed_* workloads are tiny fixtures with no timed harness and were
  not timed. GPU TPC-H paired timing is still owed on the combined tree
  with ImporterDSL.
  Historical follow-up (superseded by plain ownership below): IF-Capture
  stopped collecting a lazy answer because of a capture. Each `let` capture
  was bound as `Arc::new` of the host value, and consuming move closures cloned
  the handle (`cq_print::own_captures`). This replaced the
  `interned-capture-collect` class and also removes the collection from the
  closed-capture targets `tpch_q06` and `correlated_exists`. h19 still
  collects its one grouped row, because its new plan builds a part_key
  index. Rerun: workspaces as above; regressions 119/119 (two architecture
  checks rerun with the system Python 3.10 after a `tomllib` import
  failure); TPC-H SF1 `default,par` 58/58. Locked pairs: h02 0.998
  [0.980, 1.019], h11 0.988 [0.909, 1.061], h12 0.999 [0.959, 1.060], h16
  1.024 [0.996, 1.100], h19 0.318 [0.278, 0.342]. Expanded gate against
  `a1e3423b` with
  `--classes tools/expanded_gate_classes_interned_strings.json
  --verified-results` exits 0: 20 intended and 26 refactor-only targets,
  0 unexplained. TPC-H evidence is the default+par oracle files, plus
  paired counts where they exist. Library evidence is the e2e
  extended/typed regression logs.
- InternedStrings + GPU TPC-H stage DSL on `17a14b93` (C1, C2, P3.3e, G1),
  2026-11-01. IF-Capture handles: an `Arc` for every capture made CPU
  `retail_h07` 1.058 [1.047, 1.137] slower than `c395fe83` with identical
  counters (+0.15% instructions, +3.7% cycles, extra realloc copies); the
  same tree without captures ran 1.002. That checkpoint used type-dependent
  capture handles. Drift review removed that selection: captures use plain
  ownership and each move closure clones the values it reads at creation.
  This applies at every nesting level; Rust checks Clone. Copy values need
  no allocation; other values pay their own clone cost per closure instance.
  A-Domain now carries a typed record, never a recognized Rust call.
  Gates on the earlier rebased tree: root 1230 and
  all feature workspaces (4633 tests, 0 failed); GPU sequence 92/92;
  regressions 119/119; CPU TPC-H SF1 `default,par` 58/58 exact; DOOP xalan
  `--check` mini and mini-keyed ok; GPU TPC-H SF1 52/52 exact (advised +
  automatic) and 26/26 advised-only; `doop-gpu` cassandra 26/26;
  `srd-doop-gpu` exactness ok. Expanded gate against `17a14b93` with the
  re-keyed `tools/expanded_gate_classes_interned_strings.json` (170 kept,
  67 new reviewed rules): 0 unexplained, 20 intended and 26 refactor-only.
  GPU TPC-H timing now includes the final answer download for every query
  (the stage frontend times each stage's `to_host`; the removed standalone
  path did not). Against a `17a14b93` base built with the same boundary, all
  52 SF1 targets are within noise except h09 advised (1.004 [1.003, 1.005],
  rerun 1.016 [1.006, 1.028]): its kernels, launch and whole timed host
  region are token-identical to the base (only the kernel module name
  differs); the +14–21 µs lies in `build` (4.58 ms), so it is host binary
  layout. Locked CPU pairs (new/base): h07 0.961
  [0.805, 1.104], h19 0.261 [0.250, 0.301], q06 0.960 [0.936, 0.984], q19
  0.965 [0.935, 0.984], h12 0.981 [0.973, 0.989], d27 0.970 [0.960, 0.992],
  others' CIs include 1.0. `correlated_exists` (synthetic) 1.102 [1.093,
  1.108] is loop alignment: 99.4% of its time is
  `cq_storage::exists::Exists::from_iter`, whose hot loop has the same
  instructions in both binaries but crosses a 64-byte line only in the new
  one; built on both sides with `-C llvm-args=-align-loops=64` the ratio is
  0.999 (8 interleaved runs). The capture there is the subquery's unit
  answer, bound once per outer row, never in the loop.
  Rebased on `c0f79500` (GpuCorrect, ElabFix slices 1–2; A-Alias kept next to
  A-Domain): root 1232 and feature workspaces (4635 tests, 0 failed), GPU
  sequence 100/100, regressions 119/119, CPU TPC-H SF1 `default,par` 58/58,
  DOOP xalan `--check` ok, GPU TPC-H SF1 52/52 + 26/26, cassandra 26/26,
  `srd-doop-gpu` exactness ok; expanded gate against `c0f79500` exits 0 with
  the same keys as the timed tree (all 46 cached, 0 unexplained). All 52 GPU
  SF1 modules are identical to the timed tree (modulo kernel module names),
  so no pair was re-timed.
- Fresh paired timing, `44bf82bc` vs `c1b75718` (the last full timing
  anchor; later entries time subsets vs `c395fe83`/`17a14b93`/`20101707`),
  2026-11-02/03. Summary
  [`bench-2026-11-02.md`](../experiments/gpu/compare/bench-2026-11-02.md), raw
  samples and CIs in `bench-2026-11-02.results.json`. Five interleaved
  fresh-process pairs (GPU TPC-H SF10: four); TPC-H answers fully checked every
  sample, DOOP relation counts checked after separate full-tuple validation;
  paired bootstrap 95% CI of median ratios (cur/ref). These are historical
  checks, not the current full-first/sampled-later protocol. CPU TPC-H SF1
  (28 of 29 kernels, DuckDB-exact):
  `default` geomean 0.908 [0.902, 0.917], best h19 0.288, h04 0.535, q19
  0.707; `par` 0.820 [0.800, 0.840], best h19 0.199, h04 0.231, q06 0.235.
  CIs above 1: `default` h06 1.026 [1.017, 1.043], h20 1.024 [1.015, 1.035]
  fit the existing same-source bidirectional layout-control envelope:
  h06 [0.9401, 1.1137], h20 reverse [0.9794, 1.0451] (evidence in the linked
  summary). Neither is a confirmed regression; the control does not establish
  the historical cause. `retail_h07_block` fails to compile on `44bf82bc`
  (E0599 `cq_storage::sharded::Index::from_positions`, emitted by
  `cq-print/src/positional.rs:393` for `par`). CPU DOOP xalan (Ascent-exact):
  `mini` 0.975 [0.969, 0.982], `mini-par` 0.913 [0.891, 0.969]. Local GPU
  timing was blocked by a foreign vLLM tenant, so GPU TPC-H advised ran on
  RunPod (RTX 6000 Ada, `pod_bench_2026_11_02.py`, session
  `20261103T164334.987048Z`, every query exact). SF1 geomean was 1.085
  [1.033, 1.120] (5 pairs) and SF10 1.023 [1.000, 1.054] (4 pairs, budget).
  All SF10 effects are provisional/exploratory, not confirmed regressions
  or improvements: the protocol requires at least five interleaved pairs.
  CIs above 1 at SF1 cover 13 queries, led by h09 2.524, h21_positive 2.065,
  h10 1.553 and h03 1.264. At SF10 they cover 11, led by h09 2.340,
  h21_positive 2.088, h10 1.609 and h03 1.221. SF1 h08 1.048 and d72_inner
  1.025 lack same-source layout controls and remain unconfirmed apparent
  slowdowns. Subsequent source inspection found the boundary mismatch at
  `6a4f52bf`: current charges terminal host download to build/total but
  reference excludes it. Historical samples cannot separate that component,
  so those ratios are not confirmed execution regressions. An earlier pod
  gives SF1 1.169 [1.116, 1.217] with the same outliers.
  `doop-gpu` xalan/batik (second pod, 5 pairs, 26/26 exact) gives fixpoint
  xalan 0.239 [0.150, 0.599] and batik 0.724 [0.482, 0.836], with no CI above 1.
  These are low confidence: fresh-process spread was up to 10x on the same
  binary, cause not established. Live SRDatalog and `srd-doop-gpu` are not part of the
  pod run (no SRD toolchain or prepared corpus there), so the Oct 26 SRD
  fullclock ranges remain the only, historical, SRD reference.

- GPU TPC-H boundary repair and follow-up (2026-11-03): final host downloads
  now populate `download_ms`, outside build/total, with checked terminal-use
  facts and execution-populated boundary metadata. Three exact rotating local
  full-SF1 pairs, repeated with exclusive CPU timing locks for small effects,
  give corrected 26-query geomean **0.9475 [0.9461, 0.9489]** against `c1b75718`.
  H09 0.9740, H21 0.9935, H10 1.0021; H03 remains 1.1408. These are exploratory
  three-pair results, not five-pair acceptance claims. The H03 adjacent-commit
  diff (`17a14b93`→`c82e9abf`) removes dead zero stores but retains a five-i64
  publication stride for two live Lineitem fields; its final lookup kernel is
  native-instruction-identical. Profiling localizes growth to the publication
  write pass, not full-width membership. Compact checked publication layouts
  are a separate change. Evidence:
  `experiments/gpu/compare/gpu-tpch-boundary.results.json`; timing: 480.244 s
  rotating + 214.277 s exclusive, lock/launcher 0.082 + 96.389 s. Historical
  biased ratios above are retained as evidence, not corrected by subtraction.
- GPU compact flat publications (2026-11-03): each flat publication records its
  own resident fields and dense stride instead of the query-wide carrier; the
  checker verifies coverage, uniqueness, stride and every union/provider
  consumer against that record. Generated GPU TPC-H code vs `dbe4c2cb` differs
  only in recorded stride/offset literals (stores, reads, allocation extents,
  provider descriptors, union staging, host downloads), 20 removed dead
  carrier-slot copies, the kernel module hash and line wrapping. The record is
  printed in each plans.json stage (`gpu_publications`, e.g. H03 Lineitem
  `flat_rows ["order_key", "ship_date"] width 2`). H03 SF1,
  local RTX 6000 Ada, all exact: exclusive CPU 5 pairs 1.355 → 0.893 ms,
  0.660 [0.659, 0.661]; GPU-lock-only 10 rotating pairs 0.710 [0.701, 0.721]
  (load 11.6–29.8). One full 26-query SF1 pair (GPU lock only, loaded host)
  is exploratory: geomean 0.985, H03 0.707; others within the ±7% spread also
  seen on queries whose generated code is unchanged. Evidence:
  `experiments/gpu/compare/gpu-tpch-compact-publication.results.json`.
