# MiniLinq design

The principles and invariants the code enforces. Grammars, APIs and the exact
obligations of each language and pass live in that crate's `//!`
documentation; formal rules (`ST-Hole`, `IF-Dispatch`, …) are listed with their
implementing sources in [formal/rules.tsv](formal/rules.tsv).

## Meaning

**One meaning across presentations.** Every language denotes the same finite
set of answers, `⟦Q⟧(D) = { h(head) | h : Vars → Dom, h(body) ⊆ D }` (set
semantics). A pass changes presentation, never meaning, so the correspondence
between adjacent languages is an erasure: CQ-NF is the kernel every plan
erases to (compared up to renaming of variables), VO reductions erase to the
unreduced plan, RA erases back to the kernel, IX drops its demands, Physical
reads its operators back as RA (probe/merge as natural join, semi as
semijoin, range as select, modes and `canon` discarded), and Structures drops
its storage entries.

| Language | Presentation | Decision recorded |
|---|---|---|
| CQ-NF | atoms over distinct variables, guards, bindings | none (canonical form) |
| VO | the order in which valuations are built; granularity of each step | valuation order, split |
| VO (reduced) | + reductions applied to a clause before it is joined | where reductions go |
| RA | straight-line named algebra (SSA without φ) | none (elaboration) |
| IX | demands: the key sets the plan applies results at | none (derived) |
| Physical | one operator per result; `pipe`/`let` modes | operator, mode, dedup placement |
| Structures | the bindings Γ: the value answering each keyed read | representation |

Step `i` of a valuation plan denotes the partial valuations on the variables
bound so far that satisfy every subatom seen; the last step denotes `⟦Q⟧`.

**Storage adds no meaning.** Every runtime structure is a representation of a
relation with an abstraction function `α`; each operation commutes with `α`,
and `α` of a built value is exactly the rows it was built from
(`α ∘ build = id`). The laws are tested in `runtime/cq-storage/tests/`.

**Canonicality is a plan property.** A `canon` boundary deduplicates through
`distinct_state!`, including before a let is materialized. Uniqueness propagates
through joins of unique inputs, semijoins, selects, extends, and projections
that retain an inferred key. Host types supply no canonicity premises.

**Keys are curryings.** A relation `R ⊆ Tup(X)` read on a key `K ⊆ X` is used
at the curried type `Tup(K) → P(Tup(X∖K))`. An IX demand is such a key: an
unordered set on an exact RA result. A leveled Structures entry
`Type [K₁][K₂]` is one curried index; by `(A × B → C) ≅ (A → B → C)` it
answers a read on every prefix `K₁ ∪ … ∪ Kᵢ`. That is a trie
(`cq_storage::Trie`): `build` is currying, `lookup` is application, and a
lazily built level is call-by-need. Level order is representation and never
appears above Structures.

**Modes are binding time.** A derived `pipe` has one stream read; a `let`
retains rows or built indexes for repeated or keyed reads. A host rename is a
reusable `pipe` read. Merge requires ordered inputs: rustc checks a direct
host's `OrderedRead`; a built let or a same-key merge pipe supplies plan order.

## Program shape

**Every IR is a complete program.** Each boundary carries one `Program` that
parses, prints and checks on its own. An extension contains its base program
(VO extends CQ, IX extends RA, Structures extends Physical); RA and Physical
are standalone over `cq-syntax`. No IR carries its producer's program: RA
holds no CQ and Physical no IX.

**Languages make no capability claims.** Declarations contain only a name,
ordered columns and an optional leaf expression. Rust types and traits decide
host access. Structures records built lets, never host capabilities.

**Declaration order is layout.** A grouped host key occupies the leading
columns. Only a read whose column set equals a leading prefix is eligible for
native lookup. A different prefix length or unsupported key type falls back;
a non-leading read always builds, even when its Rust key type matches.

**Host dispatch is lazy.** `cq_storage::dispatch::Host` uses stable autoref
selection for `seek`, `contains`, and leading-column `range`. A native read
uses the host trait. Its fallback is initialized once on first use within the
query block, shared by all probes, and never built on the native path.

**Provider slots select built types and distinct state.** `index_provider!`
accepts `probe`, `semi`, `ordered`, or `trie Type`. The default provider selects
HashIndex, HashSet, BTreeMap, and Trie respectively. A caller can bind any
provider whose types implement the reads used by its plan. `distinct_state!`
handles canonical boundaries. Missing IX demands and unserved derived pipe
reads remain independent contract errors.

**The printer inserts no binding clones.** Rows borrow host and built fields,
and a value bound by an outer source is used by its inner loops as that source
produced it. The only copies are those a type imposes: an owned host lookup
key constructed from bound columns, and the rows a distinct-state provider
retains. The head expression alone decides what the result owns.

## Contracts and evidence

Four kinds of evidence stay separate; none substitutes for another.

| Evidence | Owner | Question it answers |
|---|---|---|
| Language contract | `contract::check(&Program)` in each language crate | Is this one program well formed? It returns the analysis consumers use. |
| Pass correspondence | `check(&source, &target, options…)` in each pass except the printer | Does this target correspond to this source under these options? |
| Fixtures | per-stage scenario files in `test-support/relational/scenarios`, expansion captures | Is this the output the compiler is meant to produce? |
| Oracles | set evaluators, SQL, independent reference answers | Does the executed plan compute the query's answers? |

**A language contract is local.** It sees one program; an extension's
contract also checks its base. Every `compile` ends by checking its target
contract, so an invalid program never leaves a pass. `compile` never invokes
`check`; tests, tools and callers do.

**A pass check is independent of compile.** It checks that the source is in
the pass's domain, the target contract, the erasure or kernel relation, and
the obligations of every explicit choice (split granularity, reduction tree
edges, demand derivation and names, policy-mandated operators, declaration
layout, read admissibility, provider slots). Choices the options leave free
(a prefix or elimination ranking, sibling reduction order, let/pipe
when several are admissible, structure entry order) are accepted in any
admissible form; the selector is never re-run to compare.
`tools/check_dependencies.py` rejects a `check` body that calls `compile`.

**The printer has no check.** Nothing independent can decide whether Rust
means a Structures program. Its evidence is oracle agreement of the executed
expansion and rustc's trait guards; expansion captures pin the emitted syntax,
and `tools/check_expansions.py` compares them byte for byte across a refactor.

**Fixtures pin what checks leave free.** Each pass test pins `compile(stage)`
to the next stage of every scenario, requires `check` to accept it and to
reject targeted mutations, and compares answers with an oracle.

## Openings

The checked core is closed; extensions add no core operators. They produce
Rust contexts `C[h₁, …, hₙ]` whose holes are checked core programs. There is
no residual-hole IR. The core opens in four places:

- **Expression holes.** Atom arguments, guards, bindings, computed columns and
  heads embed arbitrary Rust expressions. The CQ contract checks logical scope
  only; rustc resolves host values, types, ownership and lifetimes. Source
  identifiers keep their hygiene through every boundary, and embedded Rust
  sees every column of declared type `T` as `&T`.
- **Leaf holes.** `relation R(…) = (expr);` evaluates `expr` once, in
  declaration order, at the top of the printed block under a fresh borrowed
  binding; the value is then an ordinary host.
- **Context holes (ST-Hole).** A quotation is an ordinary macro invocation in
  the caller's Rust. The stage continuation replaces that one quotation and
  leaves the context intact; rustc checks the closing bindings. A context may
  collect an answer and feed it to another quotation, or supply a leaf.
- **Below the core.** Host representations dispatch through Rust traits;
  `index_provider!` and `distinct_state!` select built types and state. Every
  stage boundary is a caller import (see Modularity).

Each crossing between the core and its outside is one of four checked
interfaces:

| Interface | What crosses | Check |
|---|---|---|
| Surface | CQ text after extension clauses are expanded; `name! { tokens }` passes through | the CQ contract rejects every clause still unexpanded (IF-Surface) |
| Output | answer sets reused as ordinary hosts | `Answers`/`GroupedAnswers` construction retains distinct rows (IF-Output) |
| Input | declarations: ordered columns and leaf expressions | local contracts, then type dispatch (IF-Input, IF-Leaf, IF-Dispatch) |
| Environment | the enclosing Rust scope of embedded expressions | logical scope in CQ, the rest by rustc (IF-Environment) |

**Answers are ordinary hosts.** `Answers::collect_from` retains distinct rows;
`GroupedAnswers::collect_from` deduplicates `(key, payload)` pairs and supports
native keyed reads on the leading declared columns. Rows may hold `&T`, and
neither collector clones referents. A later query still establishes its own
plan uniqueness through the caller's distinct-state provider.

## Planning

**Analysis, then transformation.** Logical planning is four passes: normalize
(CQ → CQ-NF: atom arguments become distinct variables, constants and
computed arguments become `let` and `if` items), order (CQ-NF → VO: a
heuristic fixes clause order, a split fixes granularity), reduce (VO → VO:
guard pushdown and join-tree semijoins) and elaborate (VO → RA: each step one
named RA shape). The hypergraph (incidence, GYO, join tree, elimination order,
width) is an analysis the passes read, not an IR. Orders are heuristics: no
cardinality estimate, cost model or global search is claimed.

**Reductions erase to the identity.** A guard pushed onto an atom or a
Yannakakis semijoin changes the size of intermediates, never the answer. The
reduce check erases reductions back to the source plan and verifies that tree
semijoins form a full reducer of a join tree; tree reduction requires an
acyclic body.

**Policy is strategy only.** The caller supplies one query-independent
`policy { … }` (`join`, `semijoin`, `project`, `intersect`); no policy names a query's results.

**No pass reselects.** A later pass never revisits a choice an earlier one
recorded. IX derives demands from the fixed RA plan; lowering respects `join`,
`project` and declaration layout; selection preserves the Physical program
unchanged; the printer preserves the operator graph and storage choices and
decides only Rust control flow and layout.

**Worst-case-optimal join is a configuration, not an IR.** Generic join
decomposes into `Split::Fine` in the order pass (bind one variable at a
time), `project = canon` (enumerate each prefix as a set), prefix checks as
semijoins over built lets can share a curried structure; host reads dispatch by type. `intersect = smallest`
(enumerate the smaller candidate set, test the other). On the fan instance in
`facade/mini-linq/tests/wcoj.rs`, the hash-operation count of that
configuration grows at most 2.5× per doubling of `n`, while the binary and
first-candidate plans grow at least 3.5×. This is measured, not a proven
complexity bound.

## Modularity

**Dependencies follow folders.** A language depends on `cq-syntax` and at most
its base; a pass on exactly its source and target languages (a printer also on
`cq-print`). Runtime crates depend on no compiler crate; passes name runtime
items only in emitted Rust tokens. Language and pass crates have no Cargo
features. `tools/check_dependencies.py` derives the rule from the folders;
`--isolate` tests each crate with only its declared chain present.

**Every boundary is a caller import.** Each stage macro continues through a
caller-resolved slot (`mini_linq::cq_to_ra!`, `mini_linq::choose_index_policy!`,
`mini_linq::physical_policy!`, `mini_linq::select_structures!`,
`mini_linq::structures_to_rust!`, `mini_linq::distinct_state!`) using the
`cq-stage` continuation ABI. A stage can therefore be swapped, stacked or
diverged by crate choice ([examples](../examples/README.md)):

- swap: `own-dialect` replaces the printer with a push dialect;
- stack: `stacked-ir` inserts Physical → Trace → Physical, byte-identical output;
- diverge: `diverged-ra` builds IX from its own surface, without CQ, VO or planning.

**Dialects are printers.** A dialect (pull, push, parallel, batched,
incremental) is a printer and its runtime, checked by running it against
oracles and the reference printer, not by proof. Every printed program meets:

- **D1** The distinct result of running the program is `⟦Q⟧`.
- **D2** Orders a `merge` relies on are preserved, or the dialect refuses.
- **D3** A pipe is consumed once; this licenses moves, fusion, phases, fan-out.
- **D4** Embedded Rust is pure; this licenses evaluation order and count
  (laziness, batching, parallelism). The recognized comparison fragment is
  pure by construction; other expressions are trusted.
- **D5** Hosts are immutable for a run.
- **D6** A column of declared type `T` denotes `&T` in every embedded
  expression and no printer copies a binding; ownership is created only by a
  CQ `let`, a final computed column, or the endpoint.

## Extensions

The crates in `extensions/` exercise the four openings without edits to
`languages/`, `passes/`, `runtime/` or `facade/`. They emit core quotations into
caller-resolved slots and never run passes. `cq-ext-union` makes one hole per
disjunct and shares a lazy distinct sink; `cq-ext-negation` projects a negative
hole into `GroupedAnswers` and uses an environment membership guard;
`cq-ext-aggregate` folds a hole's distinct rows into grouped output;
`cq-ext-recursion` runs linear delta holes to a finite active-domain fixpoint,
feeding `Answers` through leaf declarations; `cq-ext-correlated` closes borrowed
hosts and caller parameters around per-row existential/scalar quotations.
Union and negation stack only by binding `union_hole!` to negation: neither
crate depends on or recognizes the other. Authored leaves run once per context.
Union streams when no local leaves must be retained; contexts retaining leaves
collect before returning to close their lifetimes.

Each extension has an independent structural correspondence check and mutation
tests; executed programs are compared with 51 DuckDB scenarios from 17 authored
queries. These checks establish the recorded context/quotation correspondence,
not arbitrary Rust semantics. Negation checks declared transitive host dependency
edges (opaque leaf dependencies must be supplied by the caller). Group-by folds
use the explicitly projected CQ row set; recursive heads only copy active-domain
columns; scalar subqueries require at most one distinct result and have an
explicit empty default. Each extension crate's `//!` documentation gives its surface syntax and premises.
The permanent boundary gate is `tools/check_dependencies.py`: extensions may
depend only on surface CQ/syntax, stage/storage/state-provider openings and other
extensions, and the core cannot depend on or import an extension. An incidental
diff against an old commit is not the long-term architectural gate.

## Open

- Broader extension surfaces, including mutually recursive strata and nullable
  or bag-valued aggregates. The core's native negation/aggregation syntax still
  parses and is rejected by CQ; the extensions expand their own surfaces first.
- Bag or annotated (K-relation) semantics.
- A proven WCOJ complexity bound; cardinality estimation and cost-based
  ordering.
- Native keyed reads of nested host tries.
- Incremental evaluation across changing inputs.
