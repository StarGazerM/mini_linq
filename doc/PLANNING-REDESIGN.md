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
`cq_elaborate::compile` lowers it. Datalog negative provider/hash choices live
in Storage (`f7511375`, 2026-11-02), not a `hasher_round` rewrite.
`DeviceRuns` still implements Datalog state: `fresh` plus `append_sorted`
replaces fused `admit`; replacing this state with grouped radix sets remains
planned in this baseline.

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
   Predicate placement follows bound-variable dependencies. Written order
   remains the explicit reference (`Heuristic::Written`) and `plan(order(..))`.
3. **Superseded by contract 7.** (Was: a closed Rust expression takes part
   in constraint recognition. Contract 7 forbids that recognition; remaining
   implementation violations are tracked below, not claimed eliminated.)
4. **Superseded by contract 7.** (Was: opacity is a recorded property. Every
   Rust expression is opaque and total; there is no other kind.)
5. **Datalog hands its order to CQ.** The incremental occurrence permutation
   that storage layouts follow is passed to each hole's CQ `plan` as explicit
   order advice, so layouts and the hole plan agree by construction.
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
| `cq-order Written` | identity sequence, cross products kept | explicit reference retained; Connected/Coarse facade default done (contract 2) |
| `cq-reduce Masks` | reduce only filter-ranked subtrees, never the root | retained default policy (contract 2/6); full reduction is an optional P8 cost question |
| `cq-elaborate` | predicate binder choice (branch join topology deleted) | predicate placement recorded in VO as `restrict R@i where P;` by `cq-order` (O-Restrict, order-independent H-Local shared with H-FilterRank and the reducers); lowering applies only recorded restrictions; a hoisted block's finite domain is the VO prefix binding its free columns; RA rewrites live in `cq-ra-rewrites`. Open: rename to `cq-relational-lowering` (ElabFix 5) |
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
    below it become E-Check witnesses. It reads the committed version, like a
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
    existential E-Check levels; Round/Sequence = `admit` granularity (row,
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
    `DeviceRuns::fresh` against the canonical layout, publication appends to
    every layout, and fused `DeviceRuns::admit` is removed.
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
  Remaining: a faster smallest
  read (e.g. dense or cached candidate directories, measured) before any
  default use, Retain/Expand (retained L&E witnesses need the GPU target's
  witness support), encoded FOR key packing (two P4 DECODE V sites), GPU
  Datalog layouts (replacing plain-word `DeviceRuns`), and GPU canonical
  indices.
- **P5 Scan columns (CPU).** Measured rules only (§12.4): plain default;
  byte truncation for scans; bit-packing only for bandwidth-bound parallel
  scans; encoded execution on dictionary codes first. Block format, codec trait, summaries built with
  payload, columnar loader; prune → encoded predicate (preserved-op law) →
  positions → late materialization; STR-Blocks / BE-Blocks rules.
  S1 runtime implementation: `cq-storage::scan` Plain/Bytes columns retain
  per-block headers, aligned payload and optional same-input-pass extrema;
  Bytes uses at most 16-bit residuals, otherwise native-width plain storage.
  RT-ScanCodec/RT-ScanSummary/RT-Translate cover decoding, enclosure and encoded
  predicates. This adds no compiler selection; loader/ScanTable cutover and
  STR-Blocks/BE-Blocks remain later slices.
- **P6 GPU storage** (compression for transfer/capacity first; resident
  scan codecs only where measured). Device blocks with per-vector summaries, tile decode
  fused with predicates, narrow carriers, summary skip; GPU Datalog runs as
  grouped runs.
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
- **P7 Join filters and evidence** across targets (exact min/max and
  membership from runs; published key/row/group statistics for `choose`).
- **P8 Remaining items.** Key-partition routing projection, placement spine
  (only when a branching machine appears), Masks as full reducer (a cost
  question), delta=k SRDatalog plans.
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
3. **CQ → VO / elaboration owners — ElabFix in progress.** Branch spanning
   trees are deleted and predicate attachment targets are recorded in VO by
   `cq-order` (`passes/cq-order/src/restrict.rs`, O-Restrict) under the
   order-independent H-Local rule; lowering applies only recorded
   restrictions. A hoisted block's finite domain is the VO prefix that binds
   its free columns (WF-BlockPlacement), not a written-order join, and the
   checked RA rewrites moved to `cq-ra-rewrites`. Remaining: the
   `cq-relational-lowering` rename.
4. **GPU runtime owners — historical unrecorded runtime work.** The reviewed
   tree chose run geometry inside the runtime. Current `DeviceState` records
   merge tile, block and permutation threads, consumed by `RunParams`
   (`experiments/gpu/datalog/src/storage.rs`, `datalog-runtime/src/runs.rs`;
   `14d76950`, 2026-11-02). The original lane-attribution concern is not
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
  native keyed layouts (`cq-sequence-plan/src/catalog.rs`).
- P3.3-lite removes automatic Sequence replanning rather than demoting its
  ordering authority. Open: revisit contract 4 and the `delta=k` SRDatalog
  plan translation that regressed batik.
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
