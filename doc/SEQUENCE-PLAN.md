# Sequence back end: implementation contracts

Design: `doc/formal/backend_ir.tex` (Section "Sequence"). Sequence is the
shipped default CPU target. This file records the crates, their public
functions and contracts, with their dependencies and acceptance criteria.
Nothing here names a data structure or an algorithm; implementations live in
runtime crates behind traits.

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
pub struct Program { pub structures: cq_lang_structures::Program, pub sequence: Sequence, pub graded: Option<Graded> }
pub enum Term { Rows(Ident), Map(Ident, Box<Term>), Flatten(Box<Term>), Compact(Ident, Box<Term>),
                Tile{n: usize, items: Option<usize>, inner: Box<Term>}, Lookup{provider: Ident, keys: Vec<Ident>, inner: Box<Term>},
                Fold(Monoid, Box<Term>), Canon(Box<Term>), Store{capacity: Option<usize>, inner: Box<Term>},
                Choose(Vec<(Guard, Term)>), Ref(Ident) }

/// Machine model: a tree of levels with labels. Data, not code; each target exports one.
pub struct Machine { pub name: Ident, pub root: LevelNode }
pub struct LevelNode { pub name: Ident, pub collaborative: bool, pub dynamic: bool, pub children: Vec<LevelNode> }

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
}
pub mod contract { pub fn check(p: &Program) -> syn::Result<Analysis>; }   // BE-Alpha…BE-Choose structurally
pub fn check_grading(p: &Program, m: &Machine) -> syn::Result<Analysis>;   // BE-Scope…BE-Abort against m
pub fn check_rewrite(s: &Program, t: &Program) -> syn::Result<()>;         // Structures unchanged; supp∘α equal by construction rules
impl cq_stage::Language for Program { … }
```

Checks (each a named `// Formal rule BE-…:` marker):
- BE-Alpha: every Term is well-formed over Structures (named results exist, `map` names a total segment, `lookup` key ⊆ heading, `compact` names a select).
- BE-Bind: a `lookup` under a sequential `flatten` needs `Exact`; `Range` is recorded only where the flatten's offsets are materialized (directly under `store`, domain of a `tile`, lengths folded at a level with members); semi/anti need `Membership`; merge needs `Ordered`.
- BE-Offsets / BE-Length: lengths computed for every Def; a `store` at a separated level whose Len is `Unknown` must carry a capacity; host (and ungraded) stores need none.
- BE-Collective: applies to `fold`/`scan`/`flatten` graded at a level with members; at a leaf level they are sequential loops.
- `union [d, ...]` is a view with α = ⊎ and summed Len; opaque Guard/Bind are `map` Defs that must be graded at host (BE-Partial).
- BE-Code: codec premises are deferred to traits (recorded as obligations in Analysis, like Leibniz).
- BE-Choose: guards exhaustive; all alternatives have equal `supp∘α` by construction (same Structures result).
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
normal forms under support after checking the unchanged Structures prefix;
it does not claim to prove arbitrary user Rust or runtime trait laws.
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
views, probes → `flatten(lookup …)` (BE-Bind), canon → `canon`, final
result → `store` with `Unknown` capacity, ungraded. No tiles, no choose.
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
Acceptance: all workload goldens and `xtask fuzz` equal through
`sequence → pull` as through today's printer; no function in the crate
walks the whole program except the fold.

## [5] `features/sequence-tasks` (Rayon translation)

Same signature. Machine `tasks { host > pool(separated, dynamic) > task }`.
`tile n` at `pool` → `par_chunks(n)`; `store` at `pool` → per-task `Vec`
merged at join; a directly pool-graded `fold` is rejected, while publication
evaluates its producer at task; `canon` → BE-Canon prefilter + global.
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
removed. Shared `RowEmitter` and source helpers remain for `own_push`.
An explicit Structures terminal with no authored sequence, grade, or
decompose phases bypasses the default Sequence phases. Custom Sequence
terminal paths require explicit sequence and grade.

## [9] conformance

`testkit`: `assert_conformant(program, [targets])` runs one Sequence
program through each available translation and compares `supp∘α`.
Fixtures: the workload corpus (CPU targets), triangles and H05 (GPU).

## Cutover

CPU callers use the default Sequence target instead of the removed direct
Structures pull path. GPU callers select `machine_grade` and
`cq_gpu_sequence::translate` with explicit Sequence lowering; the old GPU printer and blocked-run path
are removed.
