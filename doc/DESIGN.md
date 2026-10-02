# MiniLinq design

Every execution and representation decision lives in an IR program. Customization means a stage pass A → B, a checked rewrite L → L, or an authored IR.

The principles and invariants the code enforces. Grammars, APIs and the exact
obligations of each language and pass live in that crate's `//!`
documentation; formal rules (`ST-Hole`, `IF-Dispatch`, …) are listed with their
implementing sources in [formal/rules.tsv](formal/rules.tsv).

## Meaning

**One denotation.** Queries compose in the Kleisli category for
`T X = ⊥ + Set X`: the relational centre is Set and `⊥` is one
undistinguished panic aborting the whole evaluation. A Rust bind or guard is
an opaque partial function of its free query variables and host captures.
The compiler reads only its free variables and pattern bindings. I/O,
mutation, counters and evaluation counts are outside this meaning.

The reference is the desugared body evaluated in written order. Each target
refines it: whenever the reference returns, the target returns the same set
without panic. Relational steps commute with each other and may move earlier
past Rust steps. They may not move later. Rust steps retain their written
relative order and stay after the steps binding their free variables. An
identical Rust chain and inclusion of each source Before set in the target
Before set is a **sufficient** condition for refinement. RA laws apply only
to relational steps; bind and guard nodes belong to the ordered chain.
Equality in the relational centre is identity in classical Set.

Relational comparisons use Rust's syntax inside `if`: `==`/`!=` are
`PartialEq` operations and `< <= > >=` are `PartialOrd` operations. Under the
Leibniz Eq and total, consistent Ord premises, operands may be query variables
(bare, `*x`, `&x`), literals including negative numbers, or bare host parameters.
`&&`, `||`, `!` and parentheses over such comparisons are classical connectives.
Calls, arithmetic, methods, casts, indexing, blocks, macros and field access
remain opaque; mixed disjunctions and negations remain entirely opaque.

A top-level `&&` splits into relational constraints and one residual opaque
guard in written order at the written position. For `a && f(x)`, Rust already
runs f exactly where a holds. Applying total relational a earlier gives that
same domain. For `f(x) && a`, Rust runs f on a superset, so applying a earlier
only shrinks its domain: a refinement. The relative order of opaque conjuncts
is preserved. Internal paired bounds and literal disjunctions retain the
existing RA′ select, IX const/range and Physical point/range representations;
they are not another CQ syntax. Captures are borrowed once outside query loops.

1. Leibniz admissibility of a column type: a type may be compared, joined,
   grouped, keyed or deduplicated by the relational centre only if its `==`
   decides Leibniz equality (Rust code cannot distinguish `==`-equal values).
   Holds for integers, bool, char, String/&str, the cq-load Date/Decimal,
   tuples and derived-Eq structs of admissible types; fails for f64/f32
   (0.0 == -0.0) and hand-written lenient Eq.
2. Functional extensionality of embedded Rust: an embedded expression receives
   query variables as shared references (`&T`, never `&mut`), so its value is
   a function of the values of its free variables; equal (Leibniz) inputs give
   equal results. Interior mutability, globals and I/O are outside the meaning,
   as already documented (evaluation count is not preserved).

Any surviving representative of `==`-equal values IS the value. Dedup,
canonical projection, and projecting away variables before an opaque Rust step
are sound whenever the Rust step's free variables are retained. `Eq` and the
selected representation's `Hash`/`Ord` bounds are necessary mechanical checks;
rustc cannot prove Leibniz admissibility of a hand-written implementation.

A query variable shared across atom occurrences stays shared and is joined
directly. At each atom's written position normalization replaces only variables
repeated within that occurrence
and literals with fresh columns and relational equalities. An already-bound
argument expression becomes an immediately preceding Rust bind; one depending
on that atom becomes a following bind and equality. Computed heads become
final binds. Every normal-form atom and output contains only variables.
Erasure preserves this chain; correspondence checks compare relational regions
while allowing only refinement-preserving hoisting.

| Language | Presentation | Decision recorded |
|---|---|---|
| CQ-NF | variable-only atoms, relational constraints, Rust guards and binds | none (canonical form) |
| VO | the order in which valuations are built; granularity of each step | valuation order, split |
| VO (reduced) | + reductions applied to a clause before it is joined | where reductions go |
| RA′ | relational operators and an opaque Rust ancestry chain | none (elaboration) |
| IX | demands: key sets and stream/const/range supplies | none (derived) |
| Physical | one operator per result; `pipe`/`let`/`exists` modes | operator, intersection and semijoin choice, mode, dedup placement |
| Structures | built types, each host fallback type, boundary dedup types | representation |

Step `i` of a valuation plan denotes the partial valuations on the variables
bound so far that satisfy every subatom seen; the last step denotes `⟦Q⟧`.

**Storage adds no meaning.** Every runtime structure is a representation of a
relation with an abstraction function `α`; each operation commutes with `α`,
and `α` of a built value is exactly the rows it was built from
(`α ∘ build = id`). The laws are tested in `runtime/cq-storage/tests/`.

**Canonicality is a plan property.** A `canon` boundary deduplicates through
the concrete dedup type recorded in Structures, including before a let is materialized. Uniqueness propagates
through joins of unique inputs, semijoins, selects, binds, guards, and projections
that retain an inferred key. Host types supply no canonicity premises.

**Keys are curryings.** A relation `R ⊆ Tup(X)` read on a key `K ⊆ X` is used
at the curried type `Tup(K) → P(Tup(X∖K))`. An IX demand is such a key: an
unordered set on an exact RA result, with `stream` or `const` supply.
`i0 = @r2[1] const;` means one key fixed by literals; `stream` means keys
supplied by another result. A literal equality selection is a join with a
one-tuple relation and therefore creates a const demand; column equalities
and Rust chain nodes do not. `point select r if (x = 5)` answers a const
demand by native leading-prefix lookup or scan/filter, never by building an
index. Only stream probes can amortize building. A leveled Structures entry
`Type [K₁][K₂]` is one curried index; by `(A × B → C) ≅ (A → B → C)` it
answers a read on every prefix `K₁ ∪ … ∪ Kᵢ`. That is a trie
(`cq_storage::Trie`): `build` is currying, `lookup` is application, and a
lazily built level is call-by-need. Level order is representation and never
appears above Structures.

Conjunctions of column identities with constants or literal membership create
`const` demands. Constant or captured order bounds on one column create
`@r[c] range`. Physical reads a window on an ordered leading-column host and
otherwise scans and filters, with no index build for this demand. Copied bound
keys seek directly on BTreeMap; other ordered cursors step to the lower bound
without invoking Clone. Disjunctions stay boolean filters. Stream demands and
their amortized builds are unchanged.

**Modes are binding time.** A derived `pipe` has one stream read; a `let`
retains rows or built indexes for repeated or keyed reads. An `exists` retains a unit relation as nonemptiness while consuming its entire source; its default collection is `cq_storage::Exists`. A host rename is a
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
host access. Structures records built lets and the fallback type for each host read, without claiming which native traits a host implements.

**Declaration order is layout.** A grouped host key occupies the leading
columns. Only a read whose column set equals a leading prefix is eligible for
native lookup. A different prefix length or unsupported key type falls back;
a non-leading stream read builds, even when its Rust key type matches.
A const read scans and filters when native lookup is unavailable.

**Host dispatch is lazy.** `cq_storage::dispatch::Host` uses stable autoref
selection for stream `seek`/`contains` and const point reads. A native read
uses the host trait. Point reads require `PointLookup`, which returns the
stored key together with its payload; `IndexRead` alone exposes only the
payload and cannot preserve a Rust-visible key binding. Unsupported point
reads scan the original full rows. Its fallback is initialized once on first use within the
query block, shared by all probes, and never built on the native path.

**Structures chooses concrete types.** Built entries name Rust types such as
`::cq_storage::HashIndex<'_, _, _>`, `::std::collections::HashSet<_>`,
`::std::collections::BTreeMap<_, ::std::vec::Vec<_>>`, or a curried
`::cq_storage::Trie<'_, _, Inner>`. Each host read records a fallback type;
each `canon` boundary and noninjective computed head records a dedup type.
Retained row buffers record their collection type. The printer reads these
entries directly. Rust verifies the selected types' Build/read/insert laws;
contracts reject missing reads and known incompatible standard representations.

**The printer inserts no binding clones.** Rows borrow host and built fields,
and a value bound by an outer source is used by its inner loops as that source
produced it. The only copies are those a type imposes: an owned host lookup
key constructed from bound columns, and the rows the recorded dedup type
retains. The head expression alone decides what the result owns.

## Contracts and evidence

Four kinds of evidence stay separate; none substitutes for another.

| Evidence | Owner | Question it answers |
|---|---|---|
| Language contract | `contract::check(&Program)` in each language crate | Is this one program well formed? It returns the analysis consumers use. |
| Pass correspondence | `check(&source, &target, options…)` in each pass except the printer | Does this target preserve source meaning and discharge its recorded decisions? |
| Fixtures | per-stage scenario files in `test-support/relational/scenarios`, expansion captures | Is this the output the compiler is meant to produce? |
| Oracles | set evaluators, SQL, independent reference answers | Does the executed plan compute the query's answers? |

**A language contract is local.** It sees one program; an extension's
contract also checks its base. Every `compile` ends by checking its target
contract, so an invalid program never leaves a pass. Compilation may also call its public correspondence check; the check itself never re-runs compilation.

**A pass check is independent of compile.** It checks that the source is in
the pass's domain, the target contract, the erasure or kernel relation, and
the obligations of every explicit choice (split granularity, reduction tree
edges, demand derivation and names, recorded operators, declaration
layout, read admissibility, concrete representation types). Choices the options leave free
(a prefix or elimination ranking, sibling reduction order, let/pipe
when several are admissible, structure entry order) are accepted in any
admissible form; the selector is never re-run to compare.
`tools/check_dependencies.py` rejects a `check` body that calls `compile`.

**The printer has no check.** Nothing independent can decide whether Rust
means a Structures program. Its evidence is oracle agreement of the executed
expansion and rustc's trait guards; expansion captures pin the emitted syntax,
and `tools/check_expansions.py` compares them byte for byte across a refactor; `--review` captures every intentional type/slot difference.

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
  Structures records built types and state. Every
  stage boundary is a pipeline pass path (see Extending).

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
plan uniqueness through the recorded dedup type.

## Planning

**Analysis, then transformation.** Logical planning is four passes: normalize
(CQ → CQ-NF: atom arguments become distinct variables, constants and
literals become relational equalities, computed arguments become positioned Rust binds), order (CQ-NF → VO: a
heuristic fixes clause order, a split fixes granularity), reduce (VO → VO:
join-tree semijoins, with no Rust guard pushdown) and elaborate (VO → RA: each step one
named RA shape). The hypergraph (incidence, GYO, join tree, elimination order,
width) is an analysis the passes read, not an IR. Orders are heuristics: no
cardinality estimate, cost model or global search is claimed.

**Reductions erase to the identity.** A Yannakakis semijoin changes the size
of relational intermediates while preserving the answer. Rust guards never
move into reduction inputs. The
reduce check erases reductions back to the source plan and verifies that tree
semijoins form a full reducer of a join tree; tree reduction requires an
acyclic body.

**Default passes choose deterministic IR.** Default lowering uses probe,
keep, first, and eager decisions, marking the canonical return as needed.
Other choices are authored in Physical by a custom lowering or rewrite.
`probe smallest` is admissible only for its adjacent binary intersection;
`semi lazy` requires reusable host membership reads on its shared key.

**No pass reselects.** A later pass never revisits a choice an earlier one
recorded. IX derives demands from the fixed RA plan; lowering records concrete operators and modes using declaration layout; selection preserves the Physical program
unchanged; the printer preserves the operator graph and storage choices and
decides only Rust control flow and layout.

**Worst-case-optimal join is a configuration, not an IR.** Generic join
decomposes into `Split::Fine` in the order pass (bind one variable at a
time), canonical Physical projections (enumerate each prefix as a set), prefix checks as
semijoins over built lets can share a curried structure; host reads dispatch by type. `probe smallest`
(enumerate the smaller candidate set, test the other). The core execution tests compare these authored pipelines against independent
set answers. Optional growth/performance experiments are pending migration in
`features/workloads/`; no current complexity bound is claimed by the core gate.

## Modularity

**Dependencies follow folders.** A language depends on `cq-syntax`, `cq-stage`, its base and the languages needed by its public correspondence proof; a pass on exactly its source and target languages (a printer also on
`cq-print`). Runtime crates depend on no compiler crate; passes name runtime
items only in emitted Rust tokens. Language, pass and facade crates have no Cargo
features. `tools/check_dependencies.py` derives the rule from the folders;
`--isolate` tests each crate with only its declared chain present.

**Every boundary is a public pass path.** The facade exposes only the default
planner and passes in `mini_linq::passes`. Optional planner variants, dialects,
stacked languages and extensions belong to external consumer workspaces under
`features/`. `features/PENDING` records the wave-2 migration boundary.

**Dialects are printers.** A dialect (pull, push, parallel, batched,
incremental) is a printer and its runtime, checked by running it against
oracles and the reference printer, not by proof. Every printed program meets:

- **D1** Execution refines the desugared written-order reference: on every
  nonpanicking reference input, the result is the same set without panic.
- **D2** Orders a `merge` relies on are preserved, or the dialect refuses.
- **D3** A pipe is consumed once; this licenses moves, fusion, phases, fan-out.
- **D4** Embedded Rust is an opaque partial function. Preserve its chain and
  free-variable binders; move relational steps only earlier across it. The
  chain/Before check is sufficient for refinement. Other effects and
  evaluation counts are outside the denotation.
- **D5** Hosts are immutable for a run.
- **D6** A column of declared type `T` denotes `&T` in every embedded
  expression and no printer copies a binding; ownership is created only by a
  CQ `let`, a final computed column, or the endpoint.

## Extending

`mini_linq::mini_linq! { <CQ> }` runs the defaults. A caller can select any
public macro path without registering it in the core:

```rust,ignore
mini_linq::compile! {
    pipeline {
        plan = my_passes::plan(arguments);
        physical = [my_passes::rewrite];
        structures = [my_passes::types];
        surface = [my_surface::expand];
    }
    struct Q; relation R(x: i32); answer(x) :- R(x);
}
```

The named phases are `plan`, `ix`, `lower`, `physical`, `select`, `structures`,
`print`, and `surface`. Single stage entries are paths with optional argument
tokens in parentheses; rewrite/surface entries are lists of those paths.
Omitted entries use `mini_linq::passes::{plan_written, index_demands, lower,
select, print_pull}`. Surface passes run before CQ parsing and its contract.
The pass owns the meaning of its arguments and records all resulting choices
in its target IR.

Each macro calls `cq_stage::pass::<In, Out>(input, |source, arguments| ...)`
or `cq_stage::rewrite::<L>(input, |source, arguments| ...)`. The checked
language implements `cq_stage::Language`. Continuations receive
`pipeline { remaining_paths }; <program>`; opaque arguments are forwarded as
`arguments { tokens };` before that packet. Only a terminal Rust printer can
end the pipeline. A default pass rejects nonempty arguments.

Physical rewrites preserve RA′ erasure and admissible read supplies; Structures
rewrites preserve Physical and discharge all reads/boundaries. RA′ reuses the
elaboration tableau/Rust-chain proof. VO preserves CQ and valuation erasure,
retains existing reductions, and checks the target's existing coverage and
Before contract. The exact protocol and IR fields are documented in cq-stage.

Optional extensions and dialects were moved unchanged to `features/` for wave 2.
They are excluded from the core workspace until they consume these public APIs.
`tools/check_external.py` packages the core and tests every ready feature from a
copy outside the repository; its positive and in-tree-only negative fixtures
always run, even when every feature is pending. `tools/check_public_api.py`
checks reviewed rustdoc-derived public API snapshots.

## Open

- Broader extension surfaces, including mutually recursive strata and nullable
  or bag-valued aggregates. The core's native negation/aggregation syntax still
  parses and is rejected by CQ; the extensions expand their own surfaces first.
- Bag or annotated (K-relation) semantics.
- A proven WCOJ complexity bound; cardinality estimation and cost-based
  ordering.
- Native keyed reads of nested host tries.
- Incremental evaluation across changing inputs.
