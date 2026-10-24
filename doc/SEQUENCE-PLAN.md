# Sequence back end: implementation contracts

Design: `doc/formal/backend_ir.tex` (Section "Sequence"). Sequence is the
shipped default CPU target. This file records the crates, their public
functions and contracts, with their dependencies and acceptance criteria.
Nothing here names a data structure or an algorithm; implementations live in
runtime crates behind traits.

Set `CQ_SEQUENCE_PLAN_TIMING=1` while compiling query macros to report each
rule-based Sequence planning invocation in seconds. GPU TPC-H build reports
also record `generation_seconds` per query, scale, and mode.

`cq_lang_sequence::contract::Checker` owns proof reuse for one planning
invocation. Its `check`, `check_rewrite_in`, and `check_grading` methods retain
the same obligations as the one-shot entry points. Planner replay uses
`Advice::apply_in` and the rewrites' `_in` entry points with that shared context;
it never bypasses a checker. Caches use complete support-graph and query keys,
not estimated facts or hash-only equality.

## Dependency order and ownership

```
cq-lang-sequence  (language, checks, machine model, grading check)      [1]
   ├── cq-sequence-lowering   Structures -> Sequence (default)            [2]
   ├── cq-sequence-rewrites   Sequence -> Sequence, advice-driven         [3]  (features/sequence-rewrites)
   ├── cq-sequence-pull       translation: sequential CPU                 [4]  (passes/)
   ├── cq-sequence-tasks      translation: Rayon                          [5]  (features/)
   └── cq-gpu-sequence        translation: CUDA (experiments/gpu)         [6]
cq-storage / cq-gpu-runtime: provider and codec traits                    [7]
facade: pipeline phases `sequence`, `decompose`, `grade`, print defaults   [8]
conformance: one program, every target, equal supp∘α; H05 fixture         [9]
```

[1] blocks everything; [2]–[7] are independent of each other given [1]'s
API; [8] after [2]+[4]; [9] after any two targets. Shared crates ([1]–[3],
[8]) must not depend on target crates ([4]–[6]) or runtimes — enforced by
`tools/check_dependencies.py`.

## [1] `languages/cq-lang-sequence`

Complete program: `Structures` + `sequence { Term* }` + optional
`graded <machine> { Grade* }`. Parses, prints, round-trips, checks alone.

```text
Program  ::= Structures "sequence" "{" Def* [ Output ] "}" [ "graded" Ident "{" Grade* "}" ]
Def      ::= Ident "=" Term ";"
Term     ::= "rows" Ident                          // retained Physical result or host
           | "map" Ident Term                       // Physical result name = the total segment applied
           | "flatten" Term
           | "compact" Ident Term                   // Physical select result name
           | "tile" LitInt [ "items" LitInt ] Term
           | "lookup" Ident "[" Ident,* "]" Term    // provider result, key columns
           | "fold" Monoid Term
           | "group" Ident Term                     // Physical grouped fold over flat distinct valuations
           | "canon" Term
           | "store" [ "capacity" LitInt ] Term
           | "choose" "[" (Guard "=>" Term),+ "]"
           | Ident                                  // a Def name
Monoid   ::= "count" | "or" | "sum" Ident
Guard    ::= LenExpr ("<" | ">=") LitInt | "else"
LenExpr  ::= "len" Ident | LenExpr "+" LenExpr | LitInt
Grade    ::= Ident "at" Level ";"                  // Def name -> level of the machine
Level    ::= Ident ("." Ident)*                    // path in the machine tree, e.g. device.block.lane
Output   ::= "output" ("count" | "reserve" | "bounded") ";"
```

Public API:

```rust
pub struct Program { pub structures: cq_lang_structures::Program, pub sequence: Sequence, pub graded: Option<Graded>, pub evidence: Evidence }
pub enum Term { Rows(Ident), Map(Ident, Box<Term>), Flatten(Box<Term>), Compact(Ident, Box<Term>),
                Tile{n: usize, items: Option<usize>, inner: Box<Term>}, Lookup{provider: Ident, keys: Vec<Ident>, inner: Box<Term>},
                Fold(Monoid, Box<Term>), Group(Ident, Box<Term>), Canon(Box<Term>), Store{capacity: Option<usize>, inner: Box<Term>},
                Choose(Vec<(Guard, Term)>), Ref(Ident) }

/// Machine model: a tree of levels with labels. Data, not code; each target exports one.
pub struct Machine { pub name: Ident, pub items: usize, pub root: LevelNode }
pub struct LevelNode { pub name: Ident, pub collaborative: bool, pub dynamic: bool, pub width: Option<usize>, pub children: Vec<LevelNode> }

/// Target catalogs are ordered syntax/capability data, with no target dependency here.
pub struct Catalog { pub providers: Vec<ProviderToken>, pub codecs: Vec<CodecToken>, pub canons: Vec<CanonToken> }
pub struct ProviderToken { pub ty: syn::Type, pub capabilities: Vec<Capability>, pub levels: Vec<Vec<String>>, pub elements: Option<Vec<syn::Type>> }
pub struct CodecToken { pub ty: syn::Type, pub ordered: bool, pub levels: Vec<Vec<String>> }
pub struct CanonToken { pub ty: syn::Type, pub levels: Vec<Vec<String>>, pub max_packed_bits: Option<u32> }

/// Lengths as symbolic expressions (size types).
pub enum Len { Input(String), Scan(String), Lit(usize), Add(Box<Len>, Box<Len>), CeilDiv(Box<Len>, usize), Unknown(/*capacity*/ Option<usize>) }

pub struct Analysis {
    pub structures: cq_lang_structures::Analysis,
    pub lengths: BTreeMap<String, Len>,                 // every Def
    pub shapes: BTreeMap<String, Vec<Len>>,             // outer partitions through innermost row lengths
    pub views: BTreeSet<String>,                        // Defs that materialize nothing
    pub stores: BTreeMap<String, Len>,                  // stored Defs and their length/capacity
    pub capabilities: BTreeMap<(String, Vec<String>), Capability>, // provider reads: Exact | Range | Membership | Ordered
    pub levels: BTreeMap<String, Vec<String>>,          // grade of every Def (empty when ungraded)
    pub dispatches: Vec<Dispatch>,                      // derived (BE-Boundary): outer store, members, inner stores
    pub collectives: Vec<(String, Monoid, Vec<String>)>, // fold/scan Defs and their level
    pub output: Option<(String, OutputProtocol)>,       // the final store and how it is filled
    pub obligations: Vec<Obligation>,                  // provider/codec/canon traits and dispatch failure paths
    pub evidence: Evidence,                           // estimates and author-asserted key premises
}
pub mod contract { pub fn check(p: &Program) -> syn::Result<Analysis>; }   // BE-Alpha…BE-Choose structurally
pub fn check_grading(p: &Program, m: &Machine) -> syn::Result<Analysis>;   // BE-Scope…BE-Abort against m
pub fn check_rewrite(s: &Program, t: &Program) -> syn::Result<()>;         // checked equal support; join motion preserves pinned Rust domains
impl cq_stage::Language for Program { … }
```

`Machine.items` is the preferred per-lane count (the planner also considers one);
`LevelNode.width` records a fixed distributing width, or `None` for dynamic
levels. The GPU machine has 256-wide blocks and four items; the native sequential
and task machines use one item with dynamic widths. Each target exports
`machine()` and `catalog()`. Catalog levels are explicit absolute paths beginning
with `host`; an empty list admits no levels. Provider type patterns allow `_`
type/const arguments and `'_` lifetimes while retaining the author's actual type.
Native catalogs describe the storage providers, identity/dense codecs and set
canonicalizers; the GPU catalog describes resident `RangeProvider`,
`DenseDictionary`/identity codes and publication-level `PackedCanon`/`WideCanon`.

Checks (each a named `// Formal rule BE-…:` marker):
- BE-Alpha: every Term is well-formed over Structures (named results exist, `map` names a total segment, `lookup` key ⊆ heading, `compact` names a select).
- BE-Bind: a `lookup` under a sequential `flatten` needs `Exact`; `Range` is recorded only where the flatten's offsets are materialized (directly under `store`, domain of a `tile`, lengths folded at a level with members); semi/anti need `Membership`; merge needs `Ordered`.
- BE-Offsets / BE-Length: lengths computed for every Def; a `store` at a separated level whose Len is `Unknown` must carry a capacity; host (and ungraded) stores need none.
- BE-Collective: applies to `fold`/`scan`/`flatten` graded at a level with members; at a leaf level they are sequential loops.
- BE-Group: `group result input` names a Physical Group and consumes exactly its complete, flat, distinct valuation heading. Grouping executes at host, never per tile or at a separated pool/task. Global grouping has length one, including empty input; keyed grouping has data-dependent length and no empty groups.
- `union [d, ...]` is a view with α = ⊎ and summed Len; opaque Guard/Bind are `map` Defs that must be graded at host (BE-Partial).
- BE-Code: codec premises are deferred to traits (recorded as obligations in Analysis, like Leibniz).
- BE-Choose: guards exhaustive; all alternatives have equal `supp∘α` by construction (the same result or independently checked equivalent join/semijoin support).
- Grading: BE-Scope (reads inside extent, inner level), BE-Collective (fold/scan at collaborative level, whole level), BE-Separate (siblings exchange only via a store at an ancestor), BE-Boundary (derive `dispatches`), BE-Partial (no opaque Rust below host), BE-Abort (every dispatch has a failure channel).

Acceptance: round-trip of every form; one rejection test per rule; `rules.tsv` rows switch to `implementation`; `tools/public-api/cq-lang-sequence.txt`.

Box [1] lives in `languages/cq-lang-sequence`. `Sequence` holds named
definitions and an optional output protocol; `bounded` uses the final store's
capacity, and an omitted protocol means `count`. The grammar records capacities
explicitly rather than hiding allocation policy in a printer. A lookup has
unknown length; flatten, compact and canon produce symbolic scan lengths.
Tiles preserve the inner position count. Equal-support choose arms may have
different lengths; their joined length is unknown and requires a capacity when
stored. Checks without a machine conservatively require capacities for all
unknown-length stores.

Constructors lift through outer tile partitions: map/compact/lookup/fold/canon/
store act on innermost rows, and flatten joins the two innermost levels.
`group` instead requires a flat input: outer partitions must be flattened before
the shared valuation domain is folded. Its hash table materializes the groups,
while a named `store` still determines publication and cross-level ownership.
`Analysis.shapes` records per-level lengths, with `CeilDiv(L,n)` tile counts
and bounded inner lengths; `lengths` retains flattened cardinality.
A separated store publishes concatenated outer parts produced at its unique
child level; a depth-zero producer is one part. `Dispatch.producer_level`,
`parts` and `concatenation` record that execution level, outer tile width and
output protocol. Collaborative stores retain per-part nesting. Alpha forgets
all partition levels.
`Analysis.tilings` records constructors in preorder as `Tiling::Distribute`
or `Tiling::Strip`: a separated publication's outer tile distributes to its
child, while subsequent tiles in that definition strip-mine the current
level. `items` is work per member, not a new execution level.

Providers remain opaque type tokens: analysis records their exact/range/
membership requirements and codec premises as obligations for target traits.
No provider type spelling identifies a data structure. Machine paths normalize
to host-inclusive paths; host is the sequential collective case. Dispatch
failure channels are paths back to host, with their runtime flag behavior
recorded as obligations. The structural rewrite checker compares construction
normal forms under support. A changed Structures prefix additionally requires
an evidence-independent join/semijoin equivalence certificate with unchanged
opaque Rust domains; it does not prove arbitrary Rust or runtime trait laws.
The flat definition list defines lexical extents: a store is visible from its
definition onward, at its level and descendants. Cross-level views require
an explicit ancestor store. A distributed publication tile needs a unique
child; an ambiguous or absent child is rejected. Other tiles strip-mine
without descending, so block scans inside them retain whole-block
participation. Nonhost collectives require a collaborative grade and
uniform guards. Each separated store must be named by a definition so that dispatch
domains and publications have unambiguous names. Host and collaborative
stores may remain nested. Opaque Physical dependencies require a host
Sequence store before their values enter descendant execution.

`BE-Code` stays abstract: there is no static rejection test for a runtime
trait law. Its analysis-obligation tests replace such a test; all thirteen
statically checked backend rules have rule-specific rejection coverage.


## [2] `passes/cq-sequence-lowering`

```rust
pub fn compile(s: &cq_lang_structures::Program) -> syn::Result<cq_lang_sequence::Program>;
pub fn check(s: &cq_lang_structures::Program, t: &cq_lang_sequence::Program) -> syn::Result<()>;
```
Default lowering = "one part, nothing stored beyond Physical modes": every
`let` result → `store`, every `pipe` chain → `map`/`flatten`/`compact`
views, probes → `flatten(lookup …)` (BE-Bind), canon → `canon`, grouped
aggregates → `group result input` (BE-Group), final result → `store` with
`Unknown` capacity, ungraded. No tiles, no choose.
`check` re-derives the term from Structures and compares. Acceptance: the
whole workload corpus lowers; `check` passes; [4] over this output
reproduces today's answers (goldens + fuzz).

## [3] `features/sequence-rewrites` (proc macros + library, like `features/rewrites`)

Each is `Sequence -> Sequence`, takes advice tokens, has one law, and is
checked by `check_rewrite`:

| macro | argument | effect | law |
|---|---|---|---|
| `store_level(d)` | a `flatten` Def | wrap it in `store` (kept as offsets) | BE-Offsets |
| `tile(d, n [, items m])` | a Def | `tile n [items m]` around it | BE-Partition (views are bijections) |
| `choose(d, len x < k => a, else => b)` | guards + two Defs | `choose` | BE-Choose |
| `output(protocol)` | `count` / `reserve` / `bounded(n)` | set the final store's protocol | output paragraph |
| `codec(R.c => Type)` | host column, codec type token | record codec obligation | BE-Code |
| `canon(d => Type)` | canon provider type token | record | BE-Canon |

Acceptance per macro: applies, round-trips, `check_rewrite` passes, and a
rejection case (e.g. `tile` over a `store` at an unknown length).

## [4] `passes/cq-sequence-pull` (default sequential translation)

```rust
pub fn translate(p: &cq_lang_sequence::Program, a: &Analysis) -> syn::Result<syn::Expr>;
```
One function per Term constructor; `tile` groups contiguous rows and `choose`
uses the checked fallback arm; lifted constructors preserve outer groups.
`store` → `Vec`; output flatten → push. Reads only `Analysis`.
`group` uses the shared Physical host hash-fold adapter, preserving declared
accumulator types and consuming every canonical valuation once for every aggregate.
Acceptance: all workload goldens and `xtask fuzz` equal through
`sequence → pull` as through today's printer; no function in the crate
walks the whole program except the fold.

## [5] `features/sequence-tasks` (Rayon translation)

Same signature. Machine `tasks { host > pool(separated, dynamic) > task }`.
`tile n` at `pool` → `par_chunks(n)`; `store` at `pool` → per-task `Vec`
merged at join; a directly pool-graded `fold` is rejected, while publication
evaluates its producer at task; `canon` → BE-Canon prefilter + global.
`group` reuses the host interpretation after publication; grading rejects it at
both pool and task levels rather than reducing each chunk independently.
Acceptance: conformance with [4] on the corpus; the parallel
Datalog extension's sharded admission expressible as `tile` + `canon`.

## [6] `experiments/gpu/sequence` (CUDA translation; replaces `experiments/gpu/printer` at cutover)

Same signature. Machine `gpu { host > device(separated, dynamic) > block(collaborative) > lane }` (no warp level).
Templates: dispatch (one kernel per `Dispatch`, grid = length of its outer
stored space, dynamic claiming), `tile` at block → block of lanes, `items`
→ per-lane loop, `flatten` at block → scan in shared memory + owner search,
`store` at device → buffer sized by Len or capacity with overflow flag,
output protocols, `canon` → runtime provider. Reusable execution primitives
live in `experiments/gpu/sequence-runtime` (`cq-gpu-sequence-runtime`) and
reuse `cq-gpu-runtime` pools, streams and `DeviceColumns`. Acceptance: the
H05 term in the design section translates to two query kernels and RCCL
build/canonicalization calls; exact at SF1/SF10; target within 20% of 9.7 ms
at SF10, with measured results reported separately.

## [7] runtime traits (`cq-storage`, `cq-gpu-runtime`)

```rust
pub trait RangeCursor<K> { fn range(&self, k: &K) -> (usize, usize); }   // contiguous bucket
pub trait Membership<K>  { fn contains(&self, k: &K) -> bool; }
pub trait Codec<T> { type Code: Copy + Eq; fn enc(&T) -> Option<Self::Code>; fn dec(Self::Code) -> T; }  // injective; Ord impl ⇒ monotone
pub trait Canon<Row> { fn canon(rows) -> Self; }
```
Storage-law tests per implementation (α commutes), as today's `cq-storage`.

The GPU implementations are `RangeBuilder`/`RangeProvider` (stable RCCL
radix ordering, dense-code lower-bound offsets and gathered packed payload),
`DenseDictionary` (gpu-compress domain adapter), `SelectedRows` (stable
flagged selection for scan-only sources), and `canon::PackedCanon` (packed
keys sorted and uniqued). `canon::packed_columns` preserves duplicates when
no canon is recorded. Device scan/owner/membership helpers share gpu-tiles
and device-columns; selected sources do not invent an unused index key.

## [8] facade

`cq_stage::Configuration` includes phases `sequence`, `decompose = [...]`,
`grade`; defaults are `sequence = passes::sequence; grade = passes::sequential;
print = passes::translate_pull`. `compile!`/`mini_linq!` retain their entry
syntax. `translate_pull` supports profile counters. The direct Structures
pull macro and `cq_physical_realization::{compile, compile_profile}` are
removed. Shared `RowEmitter` adapters serve `cq-sequence-pull`; `sources_with`
prepares sources for explicit push printers such as `own_push`.
An explicit Structures terminal with no authored sequence, grade, or
decompose phases bypasses the default Sequence phases. Custom Sequence
terminal paths require explicit sequence and grade.

## [9] conformance

Implemented: `cq-testkit`'s `sequence-conformance` feature exposes
`assert_conformant(program, targets)` for `Sequential` and `Tasks`. Each
target checks its machine grading, translates, and compiles/runs against the
same inputs through the existing rustc driver. Answer rows compare as multisets,
or sets at an explicit canonical output boundary; ordering is not observable.
The first divergence names both targets and the differing row multiplicities.
The default driver uses deterministic binary-domain databases; authored corpus
inputs and raw duplicate-preserving `Case::inputs` use the same execution path.

The workload corpus expands each configured Sequence program into Sequential
and Tasks functions in the test binary through `conformance_targets!` and its
`sequence_targets` terminal. `translate_with_catalog` runs the same shared
planner, grading, translation and canonical-output check during macro expansion
that the generated-source spot-test driver uses at runtime. Caller-owned
providers extend each target's catalog; the corpus registers its own
`SortedIndex` rather than introducing a workload dependency in a target.
The test invokes both functions in-process on the same scenario and loader
bindings and compares their answer rows, without spawning rustc or a generated
executable. Planning and translation errors fail the build; execution and answer
failures are collected with configuration/scenario labels and differing row
multiplicities before the runner fails. Failures are never counted as agreement
or skipped. There is no private store-wrapping or grading preparation.
The per-program `assert_conformant*` APIs remain available for generated-source
spot tests. Capability inference is shared across machines: host row stores need
exact buckets, while segmented flattening and distributed execution require
ranges in their actual read context.

The `sequence_conformance_*` integration tests mirror the executable corpus's
13 partitions: classic, retail, and eleven full-kernel planner families. They
reuse the existing configuration lists and share registration, comparison, and
reporting through `tests/support/sequence_conformance.rs`, with one summary
test per file. Separate crates let Cargo compile the partitions in parallel
instead of serializing all 3,436 configurations in one rustc invocation; the
10,272 scenario comparisons and answer comparator are unchanged. Run them from
`features/workloads` with
`cargo test -p cq-workload-corpus --test 'sequence_conformance*' -- --nocapture`
to see each partition's configuration and agreement counts.
After `cargo clean -p cq-workload-corpus` (dependencies retained), the split
build took 4m58s versus the supplied 28-minute monolithic baseline; the timed
test command took 316.69s including all thirteen passing tests. Classic reported
274 configurations / 786 agreements, retail 314 / 942, each of the seven
coarse/hypergraph/LE kernel files 224 / 672, and each of the four fine kernel
files 320 / 960: 3,436 / 10,272 total with no failures.
`cargo clippy -p cq-workload-corpus --tests --no-deps` then completed in 221.94s;
it reported existing unused-macro warnings in `support/le_configurations.rs`
and needless-range-loop warnings in `matrix.rs`, with none in the new files.

Box [9] acceptance before the planner cutover exercised 3,436 configurations and 10,272 applicable scenarios:
10,272 agreed, zero grading skips, zero execution failures. The workload
workspace passed 3,528 tests; the root passed 753, Tasks 12, and Sequence
rewrites 17. CUDA passed 10 tests, including the SF1 H05 comparison without
skipping. Formal inventory (228 rules) and all 26 API snapshots passed.

The GPU workspace's executable `h05_conformance` test uses the checked-in H05
CUDA fixture and the same Physical program through default CPU structure
selection, Sequence lowering, and Sequential translation. Both consume the
same encoded `/data/tpch-gpu/sf1` inputs and use the shared answer comparator.
This is Physical-level conformance: device-only Structures are not presented
as a host-compatible Sequence program. Missing SF1 data produces an explicit
skip. Run it with `cargo oxide test --arch sm_89 -- --release -p cq-gpu-sequence`
from `experiments/gpu`.

## Cutover

CPU callers use the default Sequence target instead of the removed direct
Structures pull path. GPU callers select `machine_grade` and
`cq_gpu_sequence::translate` with explicit Sequence lowering; the old GPU printer and blocked-run path
are removed.

## [10] `passes/cq-sequence-plan` (decomposition planner; P-Decompose)

The planner is an advice generator over Sequence, exposed as
`cq_sequence_plan::plan(program: &Program, machine: &Machine, catalog: &Catalog)
-> syn::Result<Planned>`, where `Planned { program, trace: Vec<Advice> }`.
`Program` and the target-data-only `Catalog` belong to `cq-lang-sequence`;
target crates export their catalogs without introducing target dependencies
into shared planning. Each `Advice` replays through `.apply(&Program)`, so
folding the returned trace over the input reproduces the returned program.
The trace uses the same checked `store`/`store_level`/`tile`/`segment`/
`choose`/`output`/`grade` rewrites available to authors through [3].
The planner decides nothing an author could not write, preserves authored
choices, and leaves the emitter decision-free.

Laws:
- PL-Closure: `t' = r_n ∘ … ∘ r_1 (t)` with every `r_i` a [3] rewrite and
  `check_rewrite(r_{i-1}…, r_i…)` passing; `check_grading(t', M)` passes.
  Soundness is therefore inherited from BE-* and EXT-Seq*; the planner adds
  no law about values.
- PL-Precedence: authored rewrites are kept; the planner fills only missing
  grades, tiles and output protocols, and validates recorded obligations.
- PL-Det: a pure function of `(t, M, K)`; candidates are enumerated in a
  fixed order and ties broken by that order.
- PL-Cost: the work/span model compares legal segmented and plain schedules
  at the first feasible machine geometry. Proven lower span wins, then fewer
  stores and dispatches; equality or incomparability keeps the earlier candidate.
  Cost is an argument, never a meaning.

The checked `cq_sequence_plan::cost(&Program, &Machine)` API exposes the same
symbolic model for authored and planned programs, with `.leq`,
`.partial_cmp` and `Display` for comparisons and reports. Work counts
per-constructor evaluated positions; aliases, tiles and stores add no
evaluation work, with stores counted separately in the tie-break tier.
Span follows dependency chains and takes the maximum across branches.
An actual graded `Distribute` introduces symbolic
`maxpart(length; driver; rows)`; distribution follows machine width, not
`tile_width * items`. Unknown bucket sizes remain symbolic.
Comparisons use bounds and coefficient dominance, not measured or guessed
cardinalities; `.partial_cmp` can leave symbolic costs incomparable.
Symbolic comparisons retain the first candidate on equality or
incomparability; `.leq` does not treat incomparability as success.

Decision procedure (bounded enumeration, no search):
1. Grading: `rows` at host; a store that feeds a `lookup`, a `canon`, or is
   the final publication goes to the deepest level of `M` that may hold it
   (BE-Scope, BE-Separate); everything between inherits its producer's
   level.
2. Tiling: the first fanout in each `flatten` chain below host is tiled at
   the distributing level's width (`LevelNode::width`, a machine property;
   `None` for dynamic levels), initially with `M.items`. Try nested
   inner-flatten segmentation before plain tiling at that geometry and compare
   legal survivors by PL-Cost. Only if neither fits, halve items to one, then
   halve the machine-derived strip width to one. Dispatch-splitting publications
   at named flattens and choices are a last resort, preserving authored choices.
   Where separate primitives cannot express the change, the checked
   `segment(d, n, items m)` rewrite pairs an inner-flatten store with an
   outer tile (BE-Offsets and BE-Partition), shared by the library and macro.
3. Output: preserve every recorded protocol, including `count` inserted by
   lowering. Only when output is absent, fill `count` when the final length
   is a function of input lengths (no `compact`/`canon` on the final chain),
   else `reserve`; `bounded(n)` only when a capacity is authored.
4. Obligations: validate each reported codec/canon/provider obligation
   against catalog capabilities at its level. Structures already records
   provider and canon types; planning does not replace them or introduce
   codec/canon selection syntax. Codec laws remain abstract runtime
   premises, with catalog support validated here. Missing support rejects
   with the obligation.

Integration acceptance contracts (not a report of executed checks):
- H05: keep authored R/B references unchanged. Planning each plan's
  Structures must produce a program that grades and translates for GPU,
  with results exactly matching Sequential over the same Physical program.
  Compare planner and authored PL-Cost, print both, and require planner
  cost no greater than authored cost. At SF10, report both GPU timings and
  require planner execution time within 10% of the authored reference.
  These are acceptance obligations, not recorded outcomes.
- Tasks: the planner over the whole corpus yields graded programs that
  pass [9] conformance against Sequential (replaces the harness's own
  store-wrap + grade preparation, which is deleted).
- Facade: a `plan(machine)` phase; a pipeline with `plan` and no authored
  decomposition grades exactly as the planner says; authored advice plus
  `plan` fills the rest (PL-Precedence). GPU examples may drop hand
  tiles/stores where reproduced, but the authored H05 R/B references remain
  unchanged for the cost, correctness and timing comparisons above.
- Formal: `doc/formal/backend_ir.tex` subsection "Decomposition", rules
  PL-Closure, PL-Precedence, PL-Det, PL-Cost in `rules.tsv`, markers in
  the pass.

Executed box [10] acceptance: 3,436 corpus configurations, 10,272 agreed
scenarios, zero skips and zero failures. The workloads workspace passed 3,528
tests; root passed 784, Tasks 15, Sequence rewrites 19, and the GPU workspace
25. The catalog-aware testkit suite additionally passed 42 tests. The formal
inventory contains 234 rules, and all 27 public API snapshots passed.

H05 planned A/B/R matched their same-Physical Sequential references at SF1 and
the independent oracle at SF10 (72,983 rows). Symbolic planner/authored cost
ordering is `Equal` for B and `Less` for R; the acceptance tests print both
complete symbolic costs. SF10 GPU total-time medians, excluding warmup and
one-time input loading, were A 18.705631 ms authored / 19.290016 ms planned,
B 10.190016 / 10.626240 ms, and R 10.347200 / 10.878368 ms. Planned B and R
were respectively 4.28% and 5.13% above their authored references, within the
10% acceptance bound.

## [11] Lookup & Expand planning, evidence advice, swappable planners

**automatic = robust default (no CE); performance plans are authored through the advisory.**
Unadvised CQ and Sequence entry use the deterministic two-phase Lookup/Expand
rule: filtering Lookups first in the GYO/LargestRoot transfer order, Expands
as late as their consumers permit, and segmentation-first resource recovery.
Explicit ordering, selection, Physical/Structures, realization, staging and decomposition advice retain precedence;
`plan(preserve_order)` also requests order-preserving decomposition explicitly.
The 13 `sequence_conformance_*` crates exercise the automatic rule. GPU suite
generation enables automatic entries by default (`CQ_GPU_TPCH_AUTOMATIC=0`
opts out). Handoff gates are exact SF1/SF10 execution of all 14 supported entries,
13-crate agreement, and scoped API/lint/formal checks. Timings are reported,
not compared against an automatic-versus-advised performance gate.

Sources: Birler, Kemper, Neumann, "Robust Join Processing with Diamond
Hardened Joins" (VLDB 2024); Zhao et al., "Debunking the Myth of Join
Ordering" (RPT, SIGMOD 2025); Qiao, Boncz, Zhang, "Robust Predicate
Transfer with Dynamic Execution" (RPT+, VLDB 2026).

Observation: Sequence terms already have L&E semantics. `lookup` yields
the match range per row (L&E's iterator), `flatten` is Expand,
`compact(lookup)` is a filtering Lookup, and a `store` of looked-up rows
with a provider built over it is "a hash table that stores iterators"
(L&E §3.2, Figure 4). The authored H05 plan R is L&E's two-phase plan;
the generic path keeps written atom order and expands first (61 ms vs
10 ms at SF10). No new primitive is needed, only ordering rewrites.

Rewrites (checked by `check_rewrite`; laws Motion, BE-Offsets, BE-Scope):
1. `lookup_first`: derive the join graph and deterministic GYO/LargestRoot
   transfer order, with the deepest-parent chain tie break. Predicate-bearing
   relations and bridges filter before keyed provider builds. Scan-only roots
   and their one-consumer forward filters stream; nonroot keyed forwards retain
   rows. Schedule filtering Lookups before growth and place Expands at their
   first required consumer. Cyclic keys become available only after their
   required preceding Expand. The untested restricted-expansion-root experiment
   is not part of the default; explicit order and early-Expand advice remain
   available through the advisory.
2. `choose` on the published count of a filtered store when no evidence
   says the filter is selective (RPT+ §5 dynamic pipeline; our BE-Choose).
3. PL-Cost prefers Lookup-before-Expand (shrinking before growing) at
   equal work.

Evidence advice: estimation is not part of the query. An `evidence { … }`
advice block carries facts about data (row counts, keys, foreign keys,
selectivities, bounds, sortedness), hand-written or generated (build.rs
from a manifest, DuckDB statistics, a sampling pass). Recorded in the
Analysis beside obligations. Law PL-Evidence: estimates select among checked
candidates; a wrong estimate costs time, not answers. Keys are explicit author
assertions: false keys can change answers through BE-GroupKey. Consumers: Lookup order and early Expand placement (L&E §4
`earlyExpand`), the transfer-tree root (LargestRoot sizes), static
filter-worthiness (RPT+ threshold) instead of `choose`.

Evidence syntax (a complete advice block, also serialized in a planner trace):

```text
evidence {
    count Lineitem = 60000000;
    key Orders [order_key];
    foreign_key Lineitem [order_key] -> Orders [order_key];
    selectivity filtered_part = 1 / 10;
    bounds Orders.order_key = 0 ..= 60000000;
    sorted Orders [order_key];
}
```

Facts validate names and syntax, not their truth. Later facts of a kind take
precedence for planning; no fact establishes totality, uniqueness, referential
integrity, or a codec law for correctness checking. `Program.evidence` and
`Analysis.evidence` retain the advice independently of the obligations.
`Advice` implements parsing and token printing, so author-written
`store(name)`, `store_level(name)`, `publish(name, producer)`,
`tile(name, n, items m)`, `partition(name, n)`, `segment(name, n, items m)`,
`output(reserve)`, and `grade(gpu, name at host.device)` are the same
replayable operations as generated trace entries.
Lookup ordering uses
`lookup_first(using Provider, order [r, s, t], transfer [s], early [t])`:
all relation names denote checked Physical leaves, and omitted ordering
decisions are filled from the join graph and evidence. Generated traces freeze
those decisions explicitly. Runtime filter advice is
`adaptive(filter, published_store, threshold)`; an optional
`, select true` or `, select false` freezes a static evidence-based choice.
Both arms retain the exact joins and are independently checked.

`cq_sequence_plan::plan` obtains and replays `LookupFirst`, freezes the complete
adaptive candidate list before inserting any choices, replays every entry in
that list, then decomposes the result and prepends the ordering/adaptive trace.
The separate `decompose` entry preserves authored relational order. Retained
forward filters and multiply indexed/scanned leaves have explicit row providers;
the Lookup-first index provider must support Exact and Membership. Retained rows
use the generic Structures collection carrier, independently of keyed indexes. Adaptive
arms duplicate only pure pipe prefixes back to a publication or reusable host
scan, never opaque Rust or retained results. Reexpression preserves live host
adapters, while obsolete keyed host fallbacks become explicit retained indexes.

The Step 0 SF10 gate used one warmup and five measured runs: generic advised
H05 was **10.208480 ms**, versus authored R **10.308352 ms**, with all 72,983
answer rows exact. The generic trace records the retained customer prefix,
256-row outer partition, two 1024-row/4-item segments, and count output;
the suite and authored path use the same domain-coded input storage.

For complete Sequence input, the facade's named `plan = path` phase operates
on that Sequence program; for CQ input, the existing phase continues to
select relational order before lowering. Opaque guards keep their written
Before sets in either mode. To author a predicate-bearing leaf, put its atom
and guard first, as H09 does for Part; this is query syntax, not guard motion.

Swappable planner: `cq_sequence_plan::Planner` trait,
`plan(program, machine, catalog, evidence) -> Planned { program, trace }`;
the facade phase `plan = ::mini_linq::passes::plan` preserves authored order
by default; explicit `plan(lookup_first)` enables the rule-based planner.
`plan = ::other::dp_plan` swaps in a DP/Cascades search over
Lookup orders written as an ordinary pass. Any planner is tested by the
same replayable trace and the conformance harness.

Not covered: Expand3 (worst-case optimal cyclic joins), eager aggregation
(needs grouped fold), outer joins (not CQ).

Acceptance: generic H05 ≤ authored R at SF10; the GPU TPC-H suite
re-measured with the generic path; corpus conformance unchanged; formal
subsection "Lookup and Expand" with the citations; a DP planner stub is
not required, only the trait and one test that a custom planner pass
plugs into the facade phase.

Open verification item: the full release GPU workspace test build encounters
malformed LLVM IR in `gpu-h05-compiled.opt.ll:28514`. This CUDA toolchain
failure was independently observed in a clean HEAD (`3be6561`) worktree;
it is separate from the advised executable's exactness/performance results.
The new build-time compression dependency requires optimized release build
dependencies; the GPU workspace explicitly sets their optimization level to 3.

## [12] Grouped fold (aggregates)

Surface rule heads admit `count()`, `sum(e)`, `min(e)`, and `max(e)`.
The non-aggregate head variables are the group key. Aggregate outputs have
declared types: `count()` is `usize`, `sum(e)` has the value type of `e`,
and `min(e)` / `max(e)` have `Option<T>` for the value type `T`.

```text
relation answer(k: i32, n: usize, total: i32, low: Option<i32>);
answer(k, count(), sum(value), min(value)) :- R(k, id, value);
```

The internal relational syntax is
`group input by [k] { n: usize = count(); total: i32 = sum(value); }`.
Sequence records `group physical_result input_sequence`; this is the
segmented-fold boundary governed by **RA-Group** and **BE-Group**.
Aggregate heads currently belong to one rule; combining multiple grouped
rules is rejected rather than choosing an implicit aggregation scope.

The aggregate domain is the set of distinct valuations of **all named body
variables**, after projecting away anonymous wildcard witnesses. Every
aggregate in the head consumes the same domain. Grouping converts each
group's set of valuations into a bag of aggregate arguments: equal argument
values from different valuations retain their multiplicity. In particular,
two distinct lineitems with the same price contribute that price twice.
`count()` folds the constant one over this domain. SQL correspondence is
`SELECT keys, COUNT(*), SUM(e), ... FROM (SELECT DISTINCT named_body_columns
FROM join_and_filters) GROUP BY keys`, not `SUM(DISTINCT e)`.

Keyed empty input has no groups. An empty key denotes one global group even
for empty input: count and sum return zero, min and max return `None`.
Integer addition and the declared order must satisfy the commutative-monoid
premises; arithmetic failure propagates bottom, never a partial answer.
Opaque expressions remain in written order before grouping.

Elaboration adds an explicit RA group node, retained by IX and Physical.
Structures chooses canonicalization of the input valuation by its complete
named heading and grouping by key. Sequence's checked `group` term constructs
the key partition and folds each logical owner segment. This primitive is
necessary because existing whole-sequence `fold` cannot introduce a dynamic
partition. CPU pull fuses the keyed store/owner segments into a host hash
accumulator (and uses one scalar accumulator for global grouping); Tasks
performs the same host fold rather than inventing a collective at its
separated pool. Existing whole-sequence folds are unchanged.

Acceptance: cq-testkit conformance including duplicate input, equal-valued
distinct matches, empty keyed/global input, count/sum/min/max, and triangle
count through a head aggregate; a TPC-H H01-style lineitem sum/count query
checked against DuckDB's DISTINCT-join oracle; root and workloads suites,
unchanged unrelated goldens, formal inventory and scoped API snapshots.
GPU translation is outside this box: it will need keyed grouping, stable
owner-offset construction, segment-local monoid reduction and publication.

### Box 12d key-aware grouped fold

`evidence { Lineitem(l_orderkey, l_linenumber) key; }` asserts that no two
physical input rows have the same key. The prefix spelling
`key Lineitem [l_orderkey, l_linenumber];` denotes the same premise.
These assertions are not inferred from counts, bounds or sampled statistics.
Unlike estimates, a false key can change answers: the author is responsible
for its truth. The manifest-driven TPC-H suite asserts dbgen's declared keys
only when a loader projection carries every key column.

`group_without_canon(grouped_definition)` is checked by BE-GroupKey and
BE-Group: the rewritten input must retain positional uniqueness after every
valuation canonicalizer is removed. Keys transfer through renaming, filters,
scalar binds and joins; projections must preserve them. Missing keys, unions
and expanding Rust binds do not supply a certificate. The planner records
successful rewrites in its replay trace. GPU groups can then read resident
columns directly, evaluating group arguments without materializing complete
named valuations. With no key advice the existing canonical path is unchanged.

Measured SF10 acceptance (advised suite, one warmup and five measured runs;
before: baseline suite, after: key-aware grouping plus CQ-Arith):

| Query | Before total ms | After total ms | Speedup | After wall ms | Exact rows |
|---|---:|---:|---:|---:|---:|
| H01 | 9,107.255 | 590.690 | 15.42× | 595.870 | 4 |
| H06 | 3,310.908 | 11.776 | 281.15× | 16.895 | 1 |
| H12 | 44.427 | 25.930 | 1.71× | 25.939 | 2 |

All answers match the independent DISTINCT-valuation oracle. H01 and H06
have no canonical-valuation publication or Sequence dispatch; their group
argument kernels read resident input columns. H12 keeps its checked join
publication and removes the later valuation canon. The suite's historical
`dedup` timing bucket includes grouped argument evaluation, key ordering and
folding; it does not imply valuation dedup remains.
The unchanged CPU grouped corpus passed all eight tests. Evidence artifacts:
`/tmp/sweep3.json` (baseline), `/tmp/groupkeys-h01-sf10.json`,
`/tmp/groupkeys-h06-sf10.json`, `/tmp/groupkeys-h12-sf10.json`.

## [13] Negation in the corpus and on the GPU

CPU: the real-dbgen TPC-H inventory includes `retail_h04` (an EXISTS
implemented by a DISTINCT join), `retail_h04_not_exists` (its anti-join
variant), `retail_h16` (Part/PartSupp excluding a filtered BadSupplier set),
and `retail_h22` (phone-country-selected customers with no orders).
These are explicitly scoped CQ cores, not full canonical TPC-H answers:
H04 omits priority aggregation, H16 omits grouped supplier counts, and H22
omits the scalar average-balance subquery and grouped balance totals.

`LateLineitem` and `BadSupplier` are filtered input relations, constructed
once per CPU invocation before the CQ rule. Negation uses the existing
CQ-Neg/E-Neg semantics, including anonymous existential arguments in
`!Orders(_, custkey)`; it does not encode membership as a Rust guard.
The real-data kernels retain their independent embedded DuckDB SQL.
Synthetic prefiltered peers live in the extended fixture catalog because
the positive-only generator/evaluator is not a negation interpreter.
They run the existing retail pairwise configuration macros, all three
fixture scenarios, independent DuckDB goldens, and a direct existential
reference check that does not invoke the compiler.
Full-key intersection shapes exclude native HashMap merge (no OrderedRead)
and own-push merge (unsupported); the two anti-only cores retain the full
list, and the combined corpus still checks all 367 axis pairs.

The exporter discovers the added date, comment, and phone projections from
the authored declarations and records derived-relation names, exact
per-query answers/counts, and CPU timings in each SF1/SF10 manifest.
Comments are CSV-decoded before building string dictionaries. Aggregate
kernels have a separate contract and are not part of this DISTINCT exporter.

Acceptance: workloads workspace including the new configuration cases;
golden changes confined to the four new cases; SF1/SF10 answer counts
reported by the exporter.

GPU translation: pending.

### Box 11b proof reuse and automatic conformance

The invocation-local checker reuses successful Structures and structural
Sequence proofs under complete syntax equality, retaining at most two prefixes
and two Sequence proofs. Grading is always rechecked against the requested
machine. Physical definition lookup is indexed once per structural proof;
Lookup/Expand support graphs, headings, and query atoms use interned `u32`
identities. Original syntax retains spans and opaque Rust equality, without
rendering tokens to compare proofs. `symbol_name` formats each identifier once,
stripping a raw prefix in the resulting buffer.

The CPU conformance driver now explicitly calls the automatic planner for both
Sequential and Tasks. This selects automatic mode for the 13 corpus conformance
targets without changing the facade's gated default. Passing conformance and
performance results must still be recorded before that default changes.

### Box 12b GPU machine interpretation

The grouped GPU boundary consumes a separately published canonical valuation
store, retaining the provider recorded by Structures. Stable radix key ordering
and adjacent boundary selection construct owner offsets; collaborative kernels
fold typed count/sum/min/max per segment. Result publication retains logical
count, arithmetic/ancestor statuses, and independent value/validity columns.
`GroupedRows::to_host` decodes typed tuples including `Option<i64>` extrema.
An empty global key has one identity owner, whereas keyed empty input has none.
These are machine instances of BE-Group, not new planner decisions or laws.

Acceptance commands (release CUDA build, serialized with other oxide builds):
`cargo oxide test --arch sm_89 -- --release -p cq-gpu-sequence-runtime --test grouped_runtime`
and `cargo oxide test --arch sm_89 -- --release -p cq-gpu-sequence --test group --test grouped_h01 -- --nocapture`.
H01 reads the local exported dbgen databases and computes its independent
DuckDB DISTINCT-valuation oracle inside the test; no precomputed golden is used.

### Box 13b GPU negation acceptance

The pending GPU item above is implemented: the retained Physical Anti node
supplies negative polarity to `compact(lookup)`, and GPU translation uses the
same provider membership probe with an inverted keep bit (E-Neg/RA-Anti).
Inspection of all four suite lowering artifacts found the required polarity;
default lowering is unchanged. H04 is the positive membership control, while
H04-not-exists, H16 and H22 retain `anti eager`.

All four run exactly at SF1 and SF10 in advised mode, with one warmup and five
measured runs and additional Sequential conformance at SF1. Full counts,
phase medians, DuckDB t24 comparisons, timing-boundary caveats and the command
are in `experiments/gpu/README.md` under "Negation GPU acceptance";
machine-readable evidence is `/data/tpch-gpu/box13b-negation.json`.
Derived inputs reuse their original CPU setup expressions during loading;
the suite does not duplicate query predicates or choose plans in translators.
The release `cq-gpu-sequence --test rows` target passes all four tests,
including the two-key semi/anti translation regression asserting the same
membership primitive, opposite polarity and no witness expansion.

### Box 12b measured acceptance

The release CUDA tests passed: one runtime test (keyed/global empty identities,
count/sum/min/max, u32-width and i64 overflow); two translation/execution tests
(including duplicate physical rows and equal-valued distinct valuations); and
the H01 real-data test on both scales. H01 produced four exact groups at SF1
and four at SF10 against freshly computed DuckDB DISTINCT-valuation queries.

| Scale | GPU invocation wall ms | DuckDB t24 wall ms | GPU / DuckDB |
|---|---:|---:|---:|
| SF1 | 2352.269 | 1241.677 | 1.894 |
| SF10 | 682.524 | 11406.724 | 0.060 |

These are single invocation wall measurements, not warmed medians. GPU timing
excludes source loading/upload and includes invocation/module setup; SF1 is
the first query in the process. DuckDB timing includes the CLI invocation and
full oracle query. The existing `duckdb_timing.json` describes H05, so the test
correctly measured H01 rather than reusing that unrelated baseline.

### Box 11b machine resource and catalog contracts

The machine API additionally records `LevelNode.shared_bytes: Option<usize>`
and `Machine.layout: Option<fn(&DispatchShape) -> syn::Result<usize>>`.
`DispatchShape` describes ordered per-chain payload/fold demands, live-column
types and codec facts, tile and item counts; it contains no GPU byte constants.
`Analysis.resources` records checked shape, required bytes and capacity per
publication. BE-Resource rejects oversized authored or generated dispatches.

The planner starts at the machine width and preferred items. It first tries
nested inner-flatten segmentation, then plain tiling, and compares legal
survivors by PL-Cost. Only if neither fits does it try smaller items, then
smaller machine-derived strips. Named flatten and choice/union publications
that split dispatches are the last resort, not the first resource repair.
Single-boundary cuts are enumerated in definition order and their legal
survivors compared by PL-Cost before attempting multiple publications.
If all fail, one `syn::Error` retains every candidate diagnostic, including failed
publication preparation; suite reports print every component.

Catalog level lists govern reads. Host construction alone does not require or
establish host read capability. Generated Lookup-first stores select replayable
provider overrides using their actual live layouts; authored tokens and grades
retain precedence.

### Box 11b integration timing acceptance

Compare H05 SF10 advised and authored R in the same session using interleaved
authored/advised/authored/advised invocations, each with one warmup and five
measured iterations. Report all four medians; a relative difference within 1%
counts as equal for the advised ≤ authored acceptance gate. This comparison
band accounts for run-to-run timing variation without changing query plans.

The integration run measured the following SF10 medians (all 72,983 rows exact):

| Order | Implementation | Total ms |
|---|---|---:|
| A1 | Authored R | 10.531 |
| B1 | Advised | 10.234 |
| A2 | Authored R | 10.318 |
| B2 | Advised | 10.269 |

Both advised measurements are no slower than their preceding authored run;
the second pair is equal within the 1% band. The initial all-query suite
measurement was 10.374 ms versus a separate authored 10.310 ms, also within
that band, although the suite's existing fixed 10.35 ms ceiling returned a
failure exit status. The interleaved comparison leaves that original evidence
intact and changes neither the implementation nor the fixed-ceiling check.

### Box 11c shared checking and candidate preparation

`Checker::new(program)` seeds the invocation's complete checked source.
`Checker::check_shared` returns immutable `Rc<Analysis>` facts to read-only
planner/rewrite clients; the owned `check` API and machine-grading obligations
are unchanged. The source and two recent proofs are retained. Checked Structures
prefixes intern their physical-definition index and Lookup/Expand support queries;
Sequence proofs compare those checked identities rather than repeatedly comparing
the same opaque Rust inside one rewrite. An externally supplied mutable program
still undergoes complete prefix equality. Evidence-only changes reuse structural
facts but revalidate every evidence obligation.

Candidate-independent placement is checked once before tile/item enumeration.
Contiguous grade advice is checked as one typed batch, with each grade retained
in the replayable trace. Partition/segment construction returns its already
checked program instead of immediately replaying the same rewrite list a second
time. Resource alternatives first use nested `segment` stores and tiles;
named `store_level` and choice publications split dispatches only as a last resort.
`CQ_SEQUENCE_PLAN_TIMING` additionally reports ordering, adaptive-choice and
decomposition times; these are measured stages, not estimates.

Generated keyed tokens account for their reader layouts as well as provider
payloads: a String-carrying host reader requires a host-capable catalog token.
Where an authored descendant-only provider produces numeric rows for a host
consumer, the planner inserts a checked publication boundary instead of moving
the provider read to host. Authored provider tokens retain precedence.
These changes preserve the checked automatic-default contract: suite exactness
and corpus agreement are required; comparative performance belongs to advice.

Adaptive alternatives project each arm to the original checked live layout
before constructing their union. This is a checked Physical/Sequence projection,
not a target shortcut: a union cannot restore dead String columns that its
providers no longer carry. Later adaptive rewrites can traverse those pure
projections without crossing opaque code or retained publications.

`Analysis.provider_reads` records providers actually evaluated by each named
definition. Catalog grading consumes these checked sites instead of inferring
reads by walking a Map's entire Physical ancestry. A support-equal reheading
does not read its old providers again. Construction and offset capability
upgrades remain separate from actual read sites.

The checker interns provider/key/input edges once with each checked Structures
prefix, indexes materialized results and their proven keys, and retains inherited
read requirements as a set. Repeated reads reuse an already stronger provider
and codec obligation; choices do not multiply identical obligations. Complete
syntax checks, support proofs and machine grading remain mandatory.

### Box 13c numeric host membership build

The H04 anti-membership slowdown was host realization/upload work, not an
anti-join algorithm or a planner choice. At SF10 the filtered input contains
37,929,348 numeric rows (303,434,784 upload bytes). Nsight Systems measured
382.066 ms build while its 19 build kernels occupied about 10 ms: input
compression (five launches), key decoding (one), radix-pair ordering (seven,
including four onesweep passes), directory lower bounds (one), and directory
compression (five). Key decoding took 0.444 ms, the four sort passes about
3.006 ms, and directory lower bounds 2.970 ms. Nsight Compute measured key
decoding at 76.79% DRAM throughput and 89.89% achieved occupancy.

Primitive host boundaries now emit columns directly into reusable scratch
capacity, without the intermediate tuple copy and dynamic row-codec transpose.
Scratch is emptied after each use: inputs, results and membership indexes are
never cached. Exact bounds are reduced over the uploaded values on the GPU;
FOR encoding retains device count metadata instead of downloading the compact
extent. All realization, bounds, upload and provider construction remain in the
reported build phase. The provider algorithm and recorded plan are unchanged.

The final profile has 20 build launches (the same 19 plus `numeric_bounds`).
The new bounds launch is 568 blocks of 256 lanes over all 37,929,348 values:
0.352 ms and 95.64% DRAM throughput in Nsight Compute. Its 4,544 warp min/max
pairs transfer 72,704 bytes to the host, replacing the full CPU bounds scan.
One 303,434,784-byte H2D transfer remains per build (about 15.6 ms); there is
no membership-row D2H transfer in that phase. The compact-extent readback is
replaced by the bounds summary readback; the two scalar metadata readbacks
remain. Dataset preprocessing downloads happen once outside the timed build,
not once per membership construction. Nsight Systems' final build median was
47.710 ms; the unprofiled median was 45.808 ms.

Advised measurements below use one warmup and five measured executions.
Every execution matched the independent exported answer exactly; all eight
before/after `sequence_input` and `sequence_planned` strings were identical.

| Query | Scale | Build before ms | Build after ms | Total before ms | Total after ms | Exact rows |
|---|---|---:|---:|---:|---:|---:|
| h04 | SF1 | 0.511 | 0.512 | 1.154 | 1.154 | 52,523 |
| h04 | SF10 | 5.113 | 5.098 | 10.439 | 10.423 | 526,040 |
| h04_not_exists | SF1 | 36.367 | 3.814 | 36.612 | 4.051 | 4,695 |
| h04_not_exists | SF10 | 361.713 | 45.808 | 362.907 | 47.012 | 47,631 |
| h16 | SF1 | 1.216 | 1.354 | 1.450 | 1.594 | 118,274 |
| h16 | SF10 | 9.841 | 10.423 | 10.292 | 10.873 | 1,186,602 |
| h22 | SF1 | 0.898 | 0.844 | 1.045 | 0.992 | 14,086 |
| h22 | SF10 | 7.471 | 6.266 | 7.662 | 6.465 | 140,489 |

H16's first after batch had build medians 1.327/26.528 ms (SF1/SF10), with
SF10 samples 26.528, 233.343, 39.555, 9.549 and 10.454 ms. The table reports
the separate repeat, not a pooled or trimmed median; that repeat still had
two large SF10 samples (260.942 and 39.415 ms). This variability is retained
rather than attributed to a kernel improvement.

Evidence: `/tmp/antibuild-before.nsys-rep`, `/tmp/antibuild-after.nsys-rep`,
`/tmp/antibuild-before-ncu.ncu-rep`, `/tmp/antibuild-after-ncu.ncu-rep`,
`/tmp/antibuild-{before,after}-{query}-both.json`, and
`/tmp/antibuild-after-h16-repeat.json`. The release build used
`CQ_GPU_TPCH_QUERIES=h04,h04_not_exists,h16,h22` to isolate these measurements
from concurrent grouped-query integration. Added regressions cover primitive
column emission, signed extrema, duplicate rows, empty and changed inputs,
scratch reuse, mismatched column lengths, and multi-iteration bounds reduction.

### Box 11c historical build gate (automatic was opt-in)

The scoped language/planner run passed 122 tests, followed by the additional
inherited-read deduplication regression. Scoped root language/planner
`clippy --all-targets -D warnings` and rewrite-library clippy passed. The
Structures-checker API snapshot includes `new`, `check_shared` and
`Analysis.provider_reads`; `Checker` retains its `UnwindSafe` auto trait.

The 13 automatic corpus targets rebuilt in **2m08s Cargo time / 132.49s wall**
under `perf record`, below the six-minute build gate. All 13 executed:
3,436 configurations, 10,272 scenario agreements, zero divergences.
The profiled build recorded 6,872 planner invocations: median 3.823 ms,
p95 17.458 ms, maximum 5,999.138 ms; 100 exceeded 100 ms. These measurements
precede the final provider/key/input indexing and inherited-read set changes,
so they establish the build gate, not the remaining per-plan latency gate.
Sampling attributed 48.30% cumulative library samples to structural analysis,
22.06% to the compiler token-symbol display bridge and 13.04% to provider
storage checks; those percentages overlap. The profiler reported 37 lost
samples, and the profile remains `/tmp/box11c-conformance.perf`.

The release original-15-query GPU build succeeded with automatic generation
enabled and its explicit build-time query selector. SF10 generation results:

| Query | Automatic generation s | Generation status |
|---|---:|---|
| h03 | 0.043 | accepted |
| h04 | 0.015 | accepted |
| h04_not_exists | 0.007 | accepted |
| h05 | 1.018 | BE-Resource rejected |
| h07 | 0.923 | BE-Resource rejected |
| h07_block | 0.169 | BE-Resource rejected |
| h08 | 0.496 | BE-Resource rejected |
| h09 | 0.059 | accepted |
| h16 | 0.013 | accepted |
| h22 | 0.007 | accepted |
| h21_positive | 0.285 | BE-Resource rejected |
| d19 | 0.146 | BE-Resource rejected |
| d27 | 0.057 | accepted |
| d72_inner | 0.073 | BE-Resource rejected |
| d85 | 0.522 | BE-Resource rejected |

H05/H08 generation improved from the preserved baseline's 2.911/4.184 s,
but accepted generation is not execution exactness. The resource-rejected
queries and per-plan latency outliers left box 11c incomplete; automatic
was therefore still opt-in at that checkpoint, with advised precedence unchanged. Combined
candidate errors must be inspected when diagnosing the resource fallback:
the suite currently displays only the first error from `syn::Error`.

The seven generated automatic entries then executed exactly at SF10, with one
warmup and five measured runs. Matching advised runs used the same binary and
fixtures in the following batch (not interleaved):

| Query | Exact rows | Automatic total ms | Advised total ms |
|---|---:|---:|---:|
| h03 | 114,003 | 54.038 | 16.862 |
| h04 | 526,040 | 16.330 | 10.351 |
| h04_not_exists | 47,631 | 47.023 | 46.767 |
| h09 | 3,261,612 | 43.744 | 66.535 |
| h16 | 1,186,602 | 9.956 | 9.913 |
| h22 | 140,489 | 6.517 | 6.507 |
| d27 | 3,575 | 16.587 | 18.517 |

Evidence: `/data/tpch-gpu/box11c-automatic-sf10.json` and
`/data/tpch-gpu/box11c-advised-sf10.json`. The automatic full-suite command
returned failure because eight entries remained planner-rejected, not because
any executed result differed. These seven exact results do not satisfy the
all-query gate and do not authorize changing the unadvised default.

### Grouped retail corpus acceptance

The real-dbgen inventory now includes one H01 and the H02/H06/H10/H11/H12/
H13/H14/H15/H18/H19/H20 cores. All twelve match their independent DuckDB
oracles at SF0.1; twelve fixture oracle tests cover duplicates, distinct line
identities, empty groups and predicate boundaries. The exporter includes their
typed projections and exact SF1/SF10 answers. Scope and numeric details live in
`features/workloads/tpch/numeric_contract.txt`; H17's aggregate-dependent outer
sum remains outside the requested nonnested-aggregate inventory.

The GPU suite imports single-program grouped heads from their authored Rust,
preserves complete valuation domains, and decodes dictionary-valued answers.
Staged H02/H11/H13/H15/H18/H20 retain every authored compile! node and its
materialization dependencies in the suite. Each node has its own checked plan,
replay artifact and publication grade. Numeric grouped device publications are
adapted to resident DeviceColumns, retaining input key codec bounds without
downloading rows or their count; nullable and host-graded publications retain
host rows. H13's authored host
preprocessing and terminal counts/zeros union are preserved. Stage timings are
summed, and the same-Physical Sequential reference executes the complete graph.
Device-planning failures remain explicit on host-planned stage artifacts.
H14/H19 retain opaque String-valued witnesses and are rejected when opaque values
lack a supported device encoding. The `WideCanon` catalog provider removes H10's
former 128-bit complete-valuation limit; no witness projection or truncated
equality key is substituted.

Host row codecs now support sixteen columns (H10 requires fifteen). Downloaded
host publications map their carried layout to the downstream consumer's demand;
an eight-column publication/three-column probe regression covers the D72 shape.
A terminal stored group followed by a stored alias emits one GPU grouped
collective, not a redundant CPU group and host reupload. The ten Sequence core
and six device-column unit tests passed in release CUDA builds.

The unrestricted advised GPU executable builds. Final exact grouped timings
(one warmup, median of five, milliseconds) are:

| Query | SF1 total | SF10 total | Exact at both scales |
|---|---:|---:|---|
| H01 | 922.226 | 9491.752 | yes |
| H06 | 338.968 | 3534.769 | yes |
| H12 | 11.954 | 44.738 | yes |

These are correctness measurements, not performance acceptance. Evidence is
`/data/tpch-gpu/grouped-advised-final.json`; the earlier H10 capability failure
is retained in `/data/tpch-gpu/grouped-advised.json`. The final command returns
failure for the selected H14/H19 rejected entries, not for an answer mismatch
in H01/H06/H12. GPU suite tests additionally exercised importer/encoding and
plan replay; the earlier H10 capability failure is superseded by Box 12c.

### Box 11d resource recovery and ordering diagnosis

Resource recovery follows the deterministic segmentation-first order documented
above. Nested target-layout diagnostics survive grading and rewrite annotation;
the GPU suite joins every component of the final `syn::Error`, not only its
first `Display` message. This changes neither the layout premise nor target
translation. Halved tile widths cannot reduce a target's fixed-width scratch
allocation unless its supplied layout model records that reduction.

Full diagnostics exposed the previously hidden fallback failures: stored
flatten arms had ambiguous outer spaces (BE-Boundary), while storing their
choices at a separated level required an unknown capacity (BE-Length).
Placement now keeps a growing, host-readable store at host and uses the
existing checked publication factoring to transfer descendant producers.
It does not invent a capacity or weaken either premise. A regression forces
resource recovery through nested adaptive choices and replays the complete
checked trace.

The preserved box 11c SF10 reports expose these Lookup/Expand differences:

| Query | Automatic / advised ms | Sequence distinction |
|---|---:|---|
| h03 | 54.038 / 16.862 | Repeated membership in already-filtered Orders; retained filtered Lineitem; extra forward filtering and adaptive alternatives |
| h04 | 16.330 / 10.351 | Orders-driven automatic plan versus Lineitem-driven advised plan with the smaller Orders-date membership build |
| h04_not_exists | 47.023 / 46.767 | Input, planned term and trace identical |
| h16 | 9.956 / 9.913 | Input, planned term and trace identical |
| h22 | 6.517 / 6.507 | Input, planned term and trace identical |

For H03, writing `O' = O_date semi C_BUILDING`, automatic previously
recomputed membership `O' semi C_BUILDING`, separately retained the selected
Lineitem stream, and constructed nested choices around membership before the
same mandatory Expands. Advised streams selected Lineitem and expands O', then C.
Its upstream advice supplies no additional keys or selectivities. The generic
rule now omits membership already witnessed on the same provider/key left
spine and preserves scan-only streaming roots. Root forward filters also stay
in the one-consumer pipeline; only nonroot forwards serving keyed providers
are retained by the ordering rewrite.

For H04 the author's `no_reduce, order(Lineitem, Orders)` does not encode
unavailable statistics: recorded base counts already favor Lineitem
(59,986,052 versus 15,000,000). The earlier planner could not consider a root
whose retained index carried only membership keys. The generic rewrite now
recovers an exclusively consumed, unfrozen Rename/Select/Point/Range producer
as a full-row streaming root, preserves its scan adapters, removes the obsolete
projected index, and selects the opposite-side provider from the catalog.
Shared, frozen and union-produced indexes remain protected. These are
inferable rule defects, not documented author-knowledge exemptions.

The three identical-term rows were noninterleaved measurements; their small
differences do not establish a scheduling difference. All numbers in this
diagnosis are the preserved box 11c baseline, not post-change measurements.

#### Box 11d observed integration status

These intermediate measurements precede the final single-boundary/default
cutover. The untested restricted-root experiment was removed; timings below
are diagnostic, not release criteria:

| Check | Observed result |
|---|---|
| Language/planner integration | 130 tests passed before the final single-cut/root changes |
| Rewrite integration | 54 tests passed before the final no-op/root changes |
| Original-15 automatic generation | All 14 intended entries accepted; H07 remained Canon-rejected |
| Initial SF10 execution | The shared PTX module failed JIT because H05 signatures exceeded its parameter limit; not answer mismatches |
| Parameter-pruned H05 automatic | Exact 72,983 rows, 4,733.318 ms median, approximately 55 dispatches; intermediate diagnostic |
| Partial corpus profile | 2,900 calls, maximum 63.697 ms, p99 33.316 ms, zero above 100 ms; build stopped at two classic-intersection macro errors |
| Full 13-crate agreement | Not established at this intermediate checkpoint |

The partial profile forced one Cargo build job and is not comparable to the
earlier default-concurrency full corpus build. Its 567.93-second wall time
mostly sampled rustc/LLVM, not planning. The classic rejection was an unchanged
Physical expression followed by inappropriate Structures-only cleanup; a
no-op-before-cleanup regression and fix were added afterward.

The unacceptable H05 fallback globally published expansion branches and
choices. R instead has an untiled Region/Nation/Customer prefix, then nested
`flatten tile 1024 items 4 store flatten lookup` segments for Orders and
Lineitem inside the final dispatch, followed by full-key Supplier membership.
The revised policy must be measured against that R shape, not accepted merely
because the many-dispatch fallback fits shared memory.

Evidence: `/data/tpch-gpu/box11d-automatic-initial-sf10.json`,
`/tmp/paramprune.json`, `/tmp/box11d-conformance.perf`, and timing log
`artifact://1831`. Final handoff requires a complete corpus rebuild/execution,
fresh original-15 automatic/advised SF1 and SF10 exactness/timing reports,
and scoped formatting/clippy/API/formal checks.

### Box 12c arbitrary-width GPU canon

`WideCanon` follows `PackedCanon` in the target catalog. Selection compares the
complete declared scalar domains against each provider's maximum-bit
capability; no data evidence establishes a codec premise. Runtime packing uses
the recorded actual codec bounds to allocate N low-first u64 lanes. Stable
least-significant-lane radix passes carry row indices, then adjacent complete-row
equality retains one row in deterministic ascending packed-key order.

Both providers publish the same device count and decode through the same
column adapter. Intermediate providers, host downloads, grouped inputs and
union consumers all decode arbitrary-width fields, including lane crossings.
The formal `GPU-WideCanon` instance cites `BE-Canon`; runtime tests cover
three-lane duplicates, repeated calls and empty input, and catalog tests cover
the 128/192-bit boundary and rejection without a sufficiently wide provider.

Acceptance on the release sm_89 build: all eight runtime provider tests and
all ten catalog-selection tests passed. The formal inventory check passed
(241 rules). H10's one warmup and all five measured runs matched the exported
dbgen answers exactly; SF1 also passed Sequential-of-the-same-Physical
conformance. GPU timing includes build, kernels and final publication work;
DuckDB is the exported t24 median (not a new CPU measurement).

| H10 advised | Exact rows | GPU total ms | Build ms | Kernels ms | Final publication ms | DuckDB t24 ms |
|---|---:|---:|---:|---:|---:|---:|
| SF1 | 37,967 | 22.187 | 1.671 | 12.221 | 8.306 | 141.000 |
| SF10 | 381,105 | 299.008 | 11.871 | 266.077 | 21.056 | 1158.000 |

Evidence: `/tmp/canon-h10.json`. The final-publication phase label is the
suite's `dedup_ms`; intermediate canonicalization runs inside the kernel
interval, so that column is not an isolated canon benchmark.
## [15] CQ-Arith: earned arithmetic position independence

CQ defines `+`, `-`, `*`, and integer `/` by a nonzero literal as wrapping
operations over declared integer types. Decimal arithmetic operates on the
signed 64-bit coefficient (`Decimal::scaled`); integer-encoded dates retain
their declared width. Total numeric `if` expressions and explicit lossless
primitive `T::from` widening close the grammar. Arbitrary calls, lossy casts,
mixed-width operands and nonliteral divisors remain opaque.

The shared syntax recognizer returns the canonical wrapping expression.
CQ-GuardRecognize extracts typed total conjuncts without splitting through
opaque disjunctions or negations. CQ ordering and its independent VO checker
admit arithmetic clauses when their inputs are available, while preserving
opaque clauses' written Before domains. Bind/Guard syntax is retained in
later IRs; Physical type facts rederive totality for planner placement and
BE-Partial instead of trusting an annotation. Host and GPU row interpreters
emit the same primitive wrapping operations, with no target placement choices
inside translators.

Acceptance: recognition/rejection (especially variable or zero divisors),
overflow and signed-minimum division laws in debug CPU execution, arithmetic
Motion with opaque-domain rejection, BE-Partial device acceptance/rejection,
H06 DISTINCT-valuation exactness against DuckDB, and advised H06/H12 SF10
device exactness with before/after timings.

Observed acceptance: 124 syntax/VO/order/planner tests, 55 Sequence language
tests, 13 GPU row/core tests, and all five H06/H10/H12 CPU tests passed.
The CPU cases include debug-overflow laws, signed-minimum division by -1 and
H06's independent DuckDB DISTINCT-valuation oracle. Scoped library clippy
passed; the three changed public API snapshots were regenerated.

Both the generic CQ-Arith + WideCanon path and the combined key-covered GPU
grouping path were measured with one warmup and five exact runs at SF10:

| Query | Exact answer rows | Baseline median ms | CQ-Arith + WideCanon ms | With key-covered grouping ms |
|---|---:|---:|---:|---:|
| H06 | 1 | 3310.908 | 16.893 | 11.776 |
| H12 | 2 | 44.427 | 30.955 | 25.930 |

The generic path grades H06's range guards and multiplication (`r1`--`r4`)
at `host.device.block`; H12's guards, join and numeric conditionals
(`r1`--`r4`, `r6`--`r8`) likewise run there. The final column includes the
separate grouped-input optimization, not just arithmetic recognition.
Baseline evidence is `/tmp/sweep3.json`; generic-path evidence is
`/tmp/arith-h06-sf10.json` and `/tmp/arith-h12-sf10.json`; combined evidence
is `/tmp/groupkeys-h06-sf10.json` and `/tmp/groupkeys-h12-sf10.json`.
The generic canonical-publication path additionally needs declared primitive
bounds for computed columns: wrapping expressions may cross either endpoint,
so source codec bounds cannot be reused. WideCanon supplies complete keys when
those full declared ranges exceed 128 bits.
## [14] Staged programs in the GPU suite

The importer retains the authored compile! graph, not merely its final rule:
H02/H18/H20 have two nodes, H15 has three, H11 has four, and H13 has two
independent terminal branches. H13 also retains its authored one-time
QualifiedOrders preprocessing and counts/zeros iterator composition. Each node
has a separately checked/advised plan and replay artifact. Resident numeric
publications retain count, input key codec bounds, output column order and
ancestor statuses; nullable or host-planned publications use host rows.
Explicit caller advice is not discarded when device planning fails.

The advised correctness gate passes at SF1 and SF10, including exact comparison
against every exported answer TSV and same-Physical Sequential graph conformance
at SF1. Times below are milliseconds: one warmup, median of five per-run sums
of stage times, including inter-stage host publication when required.

| Query | SF1 rows | SF10 rows | SF1 staged / DuckDB t24 | SF10 staged / DuckDB t24 | Exact both |
|---|---:|---:|---:|---:|---|
| H02 | 460 | 4,667 | 22.600 / 11 | 496.737 / 42 | yes |
| H11 | 1,048 | 0 | 33.102 / 6 | 521.764 / 15 | yes |
| H13 | 150,000 | 1,500,000 | 15.164 / 184 | 157.185 / 1,588 | yes |
| H15 | 1 | 1 | 664.863 / 11 | 33,827.704 / 65 | yes |
| H18 | 57 | 624 | 493.079 / 87 | 5,393.855 / 771 | yes |
| H20 | 186 | 1,804 | 399.399 / 21 | 4,219.653 / 128 | yes |

These measurements include WideCanon but precede CQ-Arith/GroupKeys integration;
they are not claims of fully device execution or performance acceptance. H02
publishes its GPU minimum on the host because it is nullable; its final stage,
all H11 stages, H15's anti-dominance stage and H18's final stage are host-planned.
H20 retains its authored host predicates within GPU-planned stages. In
particular, H15's unchanged quadratic anti-dominance stage costs about 30 seconds
at SF10. The report records each host stage's device-planning diagnostic rather
than treating it as an unreported GPU execution.

Evidence: `/data/tpch-gpu/box14-stages-final-sf1.json` and the six
`/data/tpch-gpu/box14-h{02,11,13,15,18,20}-final-sf10.json` reports.

The release importer/encoding tests (9), stage-plan replay test (1), and resident
group publication regression (1) passed. The latter covers column order,
inherited key bounds, row count and ancestor failure propagation. Formal rule
inventory (241) and formatting of the new staged helper modules passed.
Scoped clippy checks the runtime with warnings, but the suite lint gate reports
44 `clippy::erasing_op` errors in generated zero-width CUDA allocations
(`capacity * 0`); no translator change or lint suppression is included here.

### Box 14b: arithmetic/key-aware staged replanning

H15 now computes per-supplier revenue, a global grouped `max()`, and an equality
join retaining every tied supplier. The nullable maximum preserves empty-input
semantics and removes the quadratic anti-dominance stage. Its independent
DuckDB oracle SQL is unchanged.

Every H02/H11/H13/H15/H18/H20 stage has a statistics-derived order and checked
schedule in `tpch/src/advice.rs`; raw DuckDB measurements, stage-by-stage
cardinalities, segmentation decisions, placement and exact compiler rejections
are recorded in `experiments/gpu/compare/tpch_statistics.md` (Box 14b).
All eight grouped stages use checked key evidence to remove valuation canon.
H11 arithmetic, H18's final guard/bind and H20's availability guard run on
device; resident grouped argument kernels evaluate the scan arithmetic.
H02 and H15 retain nullable host publications and host final stages, with their
device translator rejections retained rather than patched.

One warmup and five exact runs at each scale, using the same timing contract
as the preceding acceptance table:

| Query | SF1 before → after ms | SF10 before → after ms | DuckDB t24 SF1 / SF10 ms | Exact both |
|---|---:|---:|---:|---|
| H02 | 22.600 → 25.566 | 496.737 → 400.515 | 11 / 42 | yes |
| H11 | 33.102 → 15.872 | 521.764 → 41.827 | 6 / 15 | yes |
| H13 | 15.164 → 14.597 | 157.185 → 116.182 | 184 / 1,588 | yes |
| H15 | 664.863 → 12.177 | 33,827.704 → 18.305 | 11 / 65 | yes |
| H18 | 493.079 → 22.890 | 5,393.855 → 172.927 | 87 / 771 | yes |
| H20 | 399.399 → 16.097 | 4,219.653 → 97.352 | 21 / 128 | yes |

All six SF1 same-Physical Sequential comparisons also pass. H02's SF1
regression is reported rather than hidden. Evidence is
`experiments/gpu/compare/tpch_staged.results.json`; the rejected extra-segment
candidate measurements are retained in `tpch_staged.initial.json`, and exact
statistics SQL/results in `tpch_staged.statistics.json` alongside it.
The four release grouped integration suites (14 tests) and workloads catalog
suite (nine tests) pass, including H15's exact tied/empty/boundary DuckDB oracle.
Changed Rust ranges were formatted; existing unrelated warnings remain.
