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
3. **Recognition depends on meaning, not spelling.** A Rust expression that
   reads no query variable (a bare captured name, a field path such as
   `constants.c_x`) is a host parameter and takes part in constraint
   recognition exactly like a bare name.
4. **Opacity is a recorded property.** A clause that lowering creates for
   its own purposes (a head column copy `let __cq_k: T = *v`) is marked as a
   relational rename, never an opaque Rust barrier. Passes consult the
   recorded classification; they do not re-derive opacity from operator
   syntax (`Bind`/`Guard` variants).
5. **Datalog hands its order to CQ.** The incremental occurrence permutation
   that storage layouts follow is passed to each hole's CQ `plan` as explicit
   order advice, so layouts and the hole plan agree by construction.
6. **Docs match code.** `R-Masks` either reduces every inner edge on acyclic
   scopes or stops claiming to be the Yannakakis pass; Plan-Le's composition
   matches the implementation.

## Inventory of mislayered decisions (audit 2026-10-27)

| Layer | Decision made there today | Disposition |
|---|---|---|
| `cq-syntax guard::term` | field access ⇒ opaque; bare ident ⇒ parameter | fix (contract 3) |
| `cq-order Written` | identity sequence, cross products kept | keep as explicit reference; new default (contract 2) |
| `cq-reduce Masks` | reduce only filter-ranked subtrees, never the root | fix on acyclic scopes (contract 2/6) |
| `cq-elaborate` | predicate binder choice, branch join topology | record in VO or keep as pure refinement |
| CQ `normalize` | `*v` head ⇒ `let` ⇒ opaque `Bind` | classify as rename (contract 4) |
| `cq-physical-lowering` | probe first / eager / point-range / Let vs Pipe | representation only; allowed |
| `cq-sequence-plan lookup_first` | reorders and deletes joins after lowering; freezes on any Bind/Guard | demote to representation (retained leaves, providers); ordering comes from VO |
| `cq_planners::order(le)` | Lookup = dropped-column subatom, phase not recorded | fold into the default planner with recorded phases |
| GPU emitter | tile defaults, virtual-range publication, retry protocol | representation; document |
| Datalog `fork` | independent CQ plan after incremental permutation | pass order advice (contract 5) |

## Phases

1. Fixture tests per route (CQ core, planners feature, Datalog hole, GPU).
2. Recognition: closed host expressions as parameters; constraints at first
   binding under the written routes.
3. Default connected order and full acyclic reduction; measure TPC-H SF1
   default against `experiments/gpu/compare/performance_cpu_sf1.tsv`.
4. Datalog/GPU boundary: head copies, order advice, `lookup_first` demotion.
5. Docs: `passes.tex`, `STATUS.md`, `PLANNING-COVERAGE.md`.

Each phase lands only with the fixture green on every route it touches and
no SF1 default regression beyond noise.

## Progress

- Phase 1 done: fixtures on the core (`facade/mini-linq/tests/doop_heapalloc.rs`),
  Datalog (`features/datalog/tests/counters.rs`) and GPU
  (`experiments/gpu/datalog/facade/tests/heapalloc.rs`) routes.
- Phase 2 done: `constraint::Parameter` field paths (contract 3); default
  order `Prefix(Connected)/Coarse` places constraints at first binding.
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
