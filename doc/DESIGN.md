# MiniLinq design

Every planning, execution, and representation decision lives in an IR program.
Customization is a pass A → B, a checked rewrite L → L, or an authored IR.
Grammars and precise contracts live in crate `//!` documentation;
[formal/rules.tsv](formal/rules.tsv) maps formal rules to their sources.

## Meaning

Queries compose in the finite-set monad. Every embedded `if`, `let` or `for`
expression is opaque, pure and total by premise: a panic means the plan is
wrong. The compiler preserves the expression unchanged and places it where
its free variables are bound (the whole environment for lexically opaque
syntax). There is no Rust recognition, splitting, arithmetic rewriting, type
classification or written-order dependency beyond those free variables.
I/O, mutation and evaluation counts are outside the meaning.

Only explicit `where P` has relational predicate semantics: representation
equality/order, membership, ranges, conjunction and disjunction over query
variables, literals and closed Rust operands. Closed operands are stored as
`Term::Closed(syn::Expr)` and evaluated once on the host. Only `where` supplies
predicate-derived reads, filter ranks, reduction supports, domains and join
equalities; atom literals remain occurrence restrictions. Rust trait bounds
enforce operations, not representation laws. Types belong to rustc; sizes and
encodings come from recorded provider and codec facts.

The written reference and planned query return the same set under these
premises. CQ → VO owns placement and join decisions; downstream passes refine
representations without moving predicates or changing joins. Device Rust is
emitted verbatim for cuda-oxide only when its read columns have identity device
representations and other inputs are closed captures; otherwise it runs at
host. Codec-encoded comparisons use `where` with host-encoded closed operands.

`{ B1 } | { B2 }` is union; `{ B1 } else { B2 }` is ordered fallback. Union
combines both sets and permits sibling reordering as refinement. Fallback uses
the first nonempty branch, so later branch Rust runs only after earlier branches
are empty. Mixed operators require nesting. Empty and single-branch blocks fail
compilation. Common new bindings form the interface I; other new bindings stay
local and cannot occur in a suffix or head. Declared interface types agree.
Multiple rules for one head are a union with that head's variables as interface.

The laws are `ctx ⋈ (B1 ∪ B2) = (ctx ⋈ B1) ∪ (ctx ⋈ B2)` and
`B1 else B2 = B1 ∪ (B2 ▷_F B1)`, where F is the block's free heading. Blocks
hoist to a derived relation over F ∪ I when no branch contains Rust; a finite
prefix domain supplies free columns without branch generators. If those columns
come from prefix lets, reductions wait for that context and filter its retained
prefix; the positions are recorded in VO. Other blocks use
per-row arms with one prefix stream. Free-variable bindings extend into arms;
union siblings are unordered and fallback siblings are ordered.
Rust environment packets record lexical names and written source paths, so
a positive context semijoin can reference prefix Rust without making it arm Rust.
Positive semijoins distribute into arms while negative operands remain the
original named branch results. No input keys or constraints are assumed.

Normalization leaves literals in body atoms, makes their other arguments distinct
variables, inserts positioned equalities and Rust binds, and turns computed heads
into final binds. A literal such as `R(a, 7)` reads the relation of matching rows
with the literal column dropped; it is not the guard `R(a,b), if b == 7`.
Elaboration records `rename R@0 {a -> a} constants {b = 7}`. Expressions such as
`meta.asia` still use positioned binds. Ordering records
valuation order and coarse/fine granularity in VO; reduction records relational
tree semijoins; elaboration writes RA′. IX adds exact demand descriptors without
changing RA′. Physical records operators, modes, intersection choices, semijoin
eagerness, and canonical boundaries. Structures adds concrete build, host
fallback, retained-row, and dedup types. Later passes consume those decisions.

## Programs and storage

Every IR is a complete program that parses, prints, and checks independently.
VO contains CQ, IX contains RA′, and Structures contains Physical. RA′ and
Physical stand alone over `cq-syntax`; neither carries its producer's IR.

Declarations supply ordered columns and optional leaf expressions. Declaration
order fixes host layout; types and runtime traits determine native access.
Leading-prefix stream reads use native traits when available. A single
nonleading borrowed-key read may use `IndexedColumn<N>` for its checked
declaration slot, preserving the original payload-column order. Unsupported
slots or capabilities use a recorded fallback built lazily once. Const/range
reads use direct access or scan/filter without building an index. Those predicate reads are distinct from
literal-bearing atom occurrences: every index for such an occurrence holds only
its filtered, projected data, including retained rows when it is a stream source.
Building that data uses native leading-prefix point lookup when available,
falling back to a host scan and filter, without changing the retained relation.
Only the occurrence's row source is constant-specific. Its ordinary `let`
entries, retained rows, and indexes use the same selection and build machinery
as every other retained relation; no occurrence-specific `payload/constants`
storage annotation is added. On GPU the filtered occurrence table is produced
in the build phase, so its query stream kernel reads it without a literal test.
Built structures answer reads at their recorded key levels. Host types establish
no plan canonicality.

A derived `pipe` has one stream consumer; fused block arms may share their
recorded prefix pipe, which is consumed once. A `let` retains rows or indexes.
Reusable host renames without literal bindings remain `pipe`. An `exists` consumes its entire source
and retains nonemptiness. A `canon` boundary and noninjective computed head use
the recorded dedup type. Merge requires ordered inputs. The printer preserves
these choices, borrows host/built fields, and emits runtime trait obligations.
A runtime representation has an abstraction function α; its reads and builds
must commute with α. Storage-law tests exercise these premises.

The back end separates representation **R**, evaluation unit **E**, and control
**C** ([formal model](formal/backend.tex)). R records tuples or base-occurrence
positions with owned computed columns; its contract checks lineage coverage,
while storage laws require every operation to commute with its bag abstraction.
E records scalar or batched demand on representable, indexable pipe segments.
Absent R/E entries mean tuples/scalar; the default staging pass records the
former fixed 128-row probe lookahead. Unsupported batch representations may
end a stage without making Rust partial.
C is the emitter's pull/unfold or push/fold realization of one stream algebra:
exhaustion preserves the bag and answer-determined stopping preserves answers.
Canonical boundaries establish sets; every layer retains the recorded plan,
original Rust payloads and free-variable bindings.

A point or range `let` used only by keyed probes and semijoins on one shared
key records one index, for example `filtered = HashIndex [b] payload [b,c]
constants {a = "a"};`. The payload annotation is the projected row schema:
the index extracts its non-constant key and stores the remaining fields.
The selection or derived union's chained scan streams directly into that build. Constant fields come from
program text when downstream expressions need them. Structures checks that
the projection and constant bindings cover every downstream column use.
Derived unions record the same projected schema on every direct or curried
index. Index construction replays their chained scan; a row-scan consumer
keeps an explicit `rows` decision. Correlated unions retain one prefix stream;
other results with multiple independent indexes keep a `rows` decision.
realization follows that recorded decision without inferring retention.

## Checks and evidence

A language contract checks one program and returns facts its consumers use.
A pass's independent `check(source, target)` checks both contracts and its
erasure, refinement, demand, or representation obligations without rerunning
`compile`. Free admissible choices remain free. A printer has no independent
Rust semantic checker: rustc trait checks, expansion fixtures, and executed
oracle agreement supply distinct evidence.

Folder boundaries constrain language dependencies to shared syntax, the pass
protocol, and their base language. Passes depend on their endpoints, with explicit
adapter dependencies for `cq-physical-realization`, `cq-sequence-pull`, and
`cq-sequence-plan`'s checked selection/lowering adapters. Shared Sequence rewrites
live in `cq_sequence_plan::rewrites`; provider retyping and host retention live
in `cq_structure_selection`. Feature macros call these core-owned implementations.
Runtime crates have no compiler dependencies. Language, pass, and facade crates
have no Cargo features. `cq-testkit` depends only on languages and runtime;
CPU target planning/translation conformance and workload-dependent tests live
in `features/sequence-conformance`, not in an optional core backedge.
The isolation gate tests declared dependency closures, including dev dependencies.

## Extending

The facade accepts this named grammar; `Path(tokens)` supplies optional opaque
arguments to a pass. Omitted single phases use the core defaults; omitted lists
are empty. Surface passes run first, before CQ parsing and its contract.

```text
pipeline {
    surface = [Path(tokens), ...];
    plan = Path(tokens); ix = Path; lower = Path;
    physical = [Path(tokens), ...]; select = Path;
    structures = [Path(tokens), ...];
    represent = Path(tokens); stage = Path(tokens);
    sequence = Path(tokens); decompose = [Path(tokens), ...];
    grade = Path(tokens); print = Path(tokens);
}
```

The resolved order is surface → plan → ix → lower → physical rewrites →
select → Structures rewrites → represent → stage → sequence → decompose →
grade → print. The shipped CPU target is `sequence` → `sequential` →
`translate_pull`; the translator also emits the profile-counter variant.
An explicit Structures terminal such as `own_push::push`, with no authored
sequence, grade, or decompose phases, bypasses the default Sequence phases.
Custom Sequence terminal paths must explicitly select sequence and grade.
A pass macro receives
`arguments { tokens }; pipeline { remaining_paths }; <program>`; the
arguments packet is omitted when empty. An authored IR can start directly at
a compatible macro with an explicit queue.

Use `cq_stage::pass::<In, Out>` for a language-changing pass and
`cq_stage::rewrite::<L>` for a same-language rewrite. Languages implement
`Parse`, `ToTokens`, and `cq_stage::Language`. The protocol checks source and
target contracts and queue shape; a changing pass supplies its own independent
correspondence check. `rewrite` also invokes `Language::check_rewrite`.
Only a terminal Rust printer can end the queue. Failures become
`compile_error!`. Pass arguments record choices in target IR; subsequent passes
receive complete programs.

Features are external consumers, with versioned public core dependencies and
local development paths. `python3 tools/check_external.py` packages every core
crate, relocates every feature and its sibling consumer dependencies outside
the checkout, substitutes packaged core sources, and builds all targets and
runs workspace tests. Vendoring includes every repository feature's locked
third-party closure rather than relying on accidental core dependencies.
Escapes are checked using canonical filesystem paths, including symlinked
temporary directories. Positive, sibling-closure, and in-tree-only negative
fixtures always run.

The public API gate compares reviewed rustdoc-derived signatures.

External printers preserve recorded operators and representations or reject
unsupported choices. Their emitted Rust must refine written-order answers,
retain original Rust expressions and their free-variable bindings, respect pipe consumption and
ordered-read requirements, and keep hosts immutable for a run. External
surface extensions compose checked queries in ordinary Rust.

General cost search, a proven WCOJ bound, bag semantics, and incremental
evaluation remain separate research work; finite workload measurements and
oracle agreement do not prove those properties.
