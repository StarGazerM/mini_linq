# MiniLinq design

Every planning, execution, and representation decision lives in an IR program.
Customization is a pass A → B, a checked rewrite L → L, or an authored IR.
Grammars and precise contracts live in crate `//!` documentation;
[formal/rules.tsv](formal/rules.tsv) maps formal rules to their sources.

## Meaning

Queries compose in the Kleisli category for `T X = bottom + Set X`: one panic
aborts the whole evaluation. Embedded Rust binds and guards are opaque partial
functions of shared-reference inputs and host captures. I/O, mutation, and
evaluation counts are outside this meaning.

The reference evaluates the desugared body in written order. Whenever it
returns, a target must return the same set without panic. Relational steps may
move earlier across Rust steps; Rust steps keep their relative order, binders,
and source-before relational requirements. Relational equality assumes Leibniz
identity; ordered comparisons assume a total, consistent order. Rust trait
bounds cannot prove these semantic premises for arbitrary implementations.

Normalization makes body atoms variable-only, inserts positioned equalities
and Rust binds, and turns computed heads into final binds. Ordering records
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
Leading-prefix stream reads use native traits when available, otherwise a
recorded fallback is built lazily once. Const/range reads use direct access or
scan/filter without building an index. Built structures answer reads at their
recorded key levels. Host types establish no plan canonicality.

A derived `pipe` has one stream consumer; a `let` retains rows or indexes.
Reusable host renames remain `pipe`. An `exists` consumes its entire source
and retains nonemptiness. A `canon` boundary and noninjective computed head use
the recorded dedup type. Merge requires ordered inputs. The printer preserves
these choices, borrows host/built fields, and emits runtime trait obligations.
A runtime representation has an abstraction function α; its reads and builds
must commute with α. Storage-law tests exercise these premises.

## Checks and evidence

A language contract checks one program and returns facts its consumers use.
A pass's independent `check(source, target)` checks both contracts and its
erasure, refinement, demand, or representation obligations without rerunning
`compile`. Free admissible choices remain free. A printer has no independent
Rust semantic checker: rustc trait checks, expansion fixtures, and executed
oracle agreement supply distinct evidence.

Folder boundaries enforce this ownership. Languages depend on shared syntax,
the pass protocol, and their base language; passes depend on their endpoints.
Runtime crates have no compiler dependencies. Language, pass, and facade crates
have no Cargo features. The isolation gate tests declared dependency closures.

## Extending

The facade accepts this named grammar; `Path(tokens)` supplies optional opaque
arguments to a pass. Omitted single phases use the core defaults; omitted lists
are empty. Surface passes run first, before CQ parsing and its contract.

```text
pipeline {
    surface = [Path(tokens), ...];
    plan = Path(tokens); ix = Path; lower = Path;
    physical = [Path(tokens), ...]; select = Path;
    structures = [Path(tokens), ...]; print = Path;
}
```

The resolved order is surface → plan → ix → lower → physical rewrites →
select → Structures rewrites → print. A pass macro receives
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
runs workspace tests. Escaping to in-tree source is a gate failure. Positive,
sibling-closure, and in-tree-only negative fixtures always run.

The standalone template at `/data/mini_linq-feature-template` demonstrates a
checked Physical rewrite, terminal printer, and custom runtime. Validate it with
`python3 tools/check_external.py --feature /data/mini_linq-feature-template --out /tmp/template-gate`.
The public API gate compares reviewed rustdoc-derived signatures.

External printers preserve recorded operators and representations or reject
unsupported choices. Their emitted Rust must refine written-order answers,
retain opaque Rust ancestry and borrowed columns, respect pipe consumption and
ordered-read requirements, and keep hosts immutable for a run. External
surface extensions compose checked queries in ordinary Rust.

General cost search, a proven WCOJ bound, bag semantics, and incremental
evaluation remain separate research work; finite workload measurements and
oracle agreement do not prove those properties.
