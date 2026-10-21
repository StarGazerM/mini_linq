# Sequence back end: implementation plan

Design: `doc/formal/backend_ir.tex` (Section "Sequence"). This file fixes the
crates, their public functions and the contracts between them so that the
boxes can be implemented in parallel. Every box has (a) an owner crate,
(b) its public API, (c) what it may depend on, (d) its acceptance test.
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
pub enum Len { Input(String), Scan(String), Lit(usize), Add(Box<Len>, Box<Len>), Unknown(/*capacity*/ Option<usize>) }

pub struct Analysis {
    pub structures: cq_lang_structures::Analysis,
    pub lengths: BTreeMap<String, Len>,                 // every Def
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
- BE-Bind: `lookup` requires a provider with the `Range` capability when under `flatten`.
- BE-Offsets / BE-Length: lengths computed for every Def; a `store` whose Len is `Unknown` must carry a capacity.
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
an explicit ancestor store. A tile descends through the unique child level;
an ambiguous or absent child is rejected. Nonhost collectives require a
collaborative grade and uniform guards, and cannot be nested inside a tile
without a whole-level participation certificate (the present grammar has
none). Each separated store must be named by a definition so that dispatch
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

## [4] `passes/cq-sequence-pull` (sequential translation; replaces `cq-physical-realization` + `cq-print` at cutover)

```rust
pub fn translate(p: &cq_lang_sequence::Program, a: &Analysis) -> syn::Result<syn::Expr>;
```
One function per Term constructor; `tile` and `choose` are no-ops (one
part); `store` → `Vec`; output flatten → push. Reads only `Analysis`.
Acceptance: all workload goldens and `xtask fuzz` equal through
`sequence → pull` as through today's printer; no function in the crate
walks the whole program except the fold.

## [5] `features/sequence-tasks` (Rayon translation)

Same signature. Machine `tasks { host > pool(separated, dynamic) > task }`.
`tile n` at `pool` → `par_chunks(n)`; `store` at `pool` → per-task `Vec`
merged at join; `fold` at `pool` → reduction; `canon` → BE-Canon prefilter
+ global. Acceptance: conformance with [4] on the corpus; the parallel
Datalog extension's sharded admission expressible as `tile` + `canon`.

## [6] `experiments/gpu/sequence` (CUDA translation; replaces `experiments/gpu/printer` at cutover)

Same signature. Machine `gpu { host > device(separated, dynamic) > block(collaborative) > warp(collaborative) > lane }`.
Templates: dispatch (one kernel per `Dispatch`, grid = length of its outer
stored space, dynamic claiming), `tile` at block → block of lanes, `items`
→ per-lane loop, `flatten` at block → scan in shared memory + owner search,
`store` at device → buffer sized by Len or capacity with overflow flag,
output protocols, `canon` → runtime provider. Runtime: `cq-gpu-runtime`
types for providers (range cursor via RCCL sort + lower_bound), codecs
(dense IDs, FOR), canon (packed sort + unique), scans. Acceptance: the H05
term in the design section translates to the reference's kernels and RCCL
calls; exact at SF1/SF10; within a few percent of 9.7 ms at SF10.

## [7] runtime traits (`cq-storage`, `cq-gpu-runtime`)

```rust
pub trait RangeCursor<K> { fn range(&self, k: &K) -> (usize, usize); }   // contiguous bucket
pub trait Membership<K>  { fn contains(&self, k: &K) -> bool; }
pub trait Codec<T> { type Code: Copy + Eq; fn enc(&T) -> Option<Self::Code>; fn dec(Self::Code) -> T; }  // injective; Ord impl ⇒ monotone
pub trait Canon<Row> { fn canon(rows) -> Self; }
```
Storage-law tests per implementation (α commutes), as today's `cq-storage`.

## [8] facade

`cq_stage::Configuration` gains phases `sequence`, `decompose = [...]`,
`grade`; defaults `sequence = passes::sequence; grade = passes::sequential;
print = passes::translate_pull`. `compile!`/`mini_linq!` unchanged for
users. Old `print_pull` remains until [4] passes goldens, then is removed.

## [9] conformance

`testkit`: `assert_conformant(program, [targets])` runs one Sequence
program through each available translation and compares `supp∘α`.
Fixtures: the workload corpus (CPU targets), triangles and H05 (GPU).

## Cutover (after [6] is exact on H05, triangles, Datalog bridge)

Delete `experiments/gpu/printer`, `blocked-run`, the `print_pull` path;
move `experiments/gpu/datalog` to `grade = gpu; print = cq_gpu::translate`.
