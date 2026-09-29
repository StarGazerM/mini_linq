# Logical planning and its compiler boundary

CQ reaches RA through `CQ → CQ-NF → VO → VO → RA`. These passes choose a
valuation order, optional reductions, and named RA; they do not choose
structures or an execution model. The [verification guide](PLANNING-VERIFICATION.md)
describes logical checks and authored reference plans. Post-IX compilation
follows [Compiler ownership and composition](COMPILER-STRUCTURE.md).

## Ownership and composition

| Operation | Implementation |
|---|---|
| CQ → CQ-NF; make atom arguments distinct variables | `cq_ir::cq::normal::normalize` |
| CQ-NF hypergraph facts: incidence, GYO join tree, elimination order and width | `cq_ir::cq::hypergraph::Analysis` |
| CQ-NF → VO; choose valuation order and granularity | `cq_order::compile` |
| VO → VO; push single-atom guards and optionally apply tree semijoins | `cq_reduce::compile` |
| VO → named RA | `cq_elaborate::compile` |
| RA → named index demands and uses | `cq_index_policy::compile` |
| IX plus policy → Physical | `cq_physical_lowering::compile` |
| Physical plus policy → Structures | `cq_structure_selection::compile` |
| Structures → Rust | `cq_physical_realization::compile` |

`mini_linq!` selects `cq_to_ra!`: normalization, Written/Coarse ordering,
and elaboration. `cq_to_ra_hypergraph!` uses the same order, then
`Pushdown::Atoms` and `Reductions::Tree` when the body is acyclic before
elaboration. These facades pass RA to `mini_linq::choose_index_policy!`; a
caller can import `index_demands` under that name. IX then goes to the
caller's `physical_policy!`, which appends a query-independent `policy { … }`
and invokes `lower_physical!`; Physical records one operator and mode per
result, and Structures records what answers each read.

Planning can be tested through the typed pass APIs without a physical policy.
The [full-chain test](../crates/mini-linq/tests/physical_query.rs) follows the
supported path into execution. The
[runnable composition example](../examples/composition/README.md) starts from
explicit RA.

`cq_order::Heuristic` selects `Written`, `Prefix(Connected)`,
`Prefix(GreedyWidth)`, `Prefix(Auto)`, or `Elimination`. `Split::Coarse`
places one full subatom per atom, giving a binary join plan;
`Split::Fine` places one new variable per subatom, giving a generic join
plan. Prefix rankings use bound-variable connectivity and heading arity;
Elimination uses the hypergraph analysis's variable order. They do not
estimate cardinalities or perform global cost search. The GYO join tree
belongs to analysis and optional reduction, not to a cost model.

`cq::normal::normalize` converts constants, repeated variables, wildcards,
and expression arguments into `let` and `if` items around atoms. The
[expression guide](RUST-EXPRESSIONS.md) describes how those expressions are
kept available. `cq_order` places guards and bindings when their variables
are bound, preserving query order within each kind. `cq_reduce` can turn a
guard over one atom into a reduction on that atom; opaque guards remain
steps. Rust lexical scope remains in `cq::rust::free_variables`;
type ascriptions belong to `Let::binding_types`.

`cq::normal::check(module)` validates CQ-NF.
`cq_order::check(source, target, options)` and
`cq_reduce::check(source, target, options)` check their VO boundaries;
`cq_elaborate::check(source, target)` checks RA correspondence. VO and RA
erase to the same CQ-NF `Kernel`, compared up to variable renaming;
`relational_plan::kernel` reads the query back from the emitted RA, rather
than trusting its source annotation. Downstream passes consume the written
algebra.

## Languages used by planning

- CQ-NF atoms have distinct variable arguments. `Kernel` records atoms,
  guards, bindings and head after normalization.
- VO records subatom steps, guard and binding steps, and optional reductions.
  A subatom identifies its source clause, so duplicate-looking occurrences
  remain distinct. A semijoin reduction or pushed guard restricts a clause's
  relation before its first join and erases under set semantics.
- RA Semijoin denotes a natural join followed by projection to the left
  heading, including composite and empty separators. `cq_elaborate` emits
  renames, joins or semijoins, selections, extensions and a final projection
  from the VO steps.
- IX names indexes on exact RA versions: `i = @r[0,2]` uses ordinals in `r`'s
  sorted logical heading, and `j.right = i` records consumption. Empty shared
  headings still require an explicit `[]` demand. No structure is chosen by IX.
- Physical fixes one physical operator and mode per result; Structures binds
  every keyed read to a host key or a built type. Neither participates in
  logical join planning.

## Administrative rules and contracts

`cq_ir::cq::hypergraph::Analysis` records each atom occurrence's variables
and incidence, the primal graph, GYO steps and join tree when acyclic,
maximum-cardinality-search elimination order (perfect when the primal
graph is chordal), and width. Guards and bindings are not hyperedges. GYO
works on residual analysis edges; it does not delete executable atoms. The
join tree used for reductions differs from the VO step order and the RA
dependency DAG.

`cq_order::compile` schedules the normalized clauses according to its
heuristic and split. Every atom is fully covered by the resulting subatoms;
an already-bound variable appears in any subatom that tests it. A guard or
binding waits for its inputs. `cq_reduce::compile` preserves the step order
and split while inserting selected reductions. Tree reduction visits the
join tree upward and downward before the first atom step; it requires an
acyclic body. `cq_elaborate::compile` turns each VO step into named RA over
the current relation of partial valuations, then projects for the head.

After normalization, the order, reduction and elaboration passes check their
source/target correspondence; order and reduction take options as well.
VO and RA kernels must agree with the CQ-NF kernel up to renaming. These
checks concern logical content and well-formed steps; they do not measure
runtime cost. The [pass tests](PLANNING-VERIFICATION.md) also compare selected
plans with independently evaluated query answers.

| Concept | Current location |
|---|---|
| Normalization and kernel | [normal.rs](../crates/cq-ir/src/cq/normal.rs) |
| Incidence, GYO, join tree, elimination order and width | [hypergraph.rs](../crates/cq-ir/src/cq/hypergraph.rs) |
| VO syntax, contract and kernel | [valuation.rs](../crates/cq-ir/src/valuation.rs) |
| Order, heuristic and split | [cq-order](../crates/cq-order/src/lib.rs) |
| Guard pushdown and tree reductions | [cq-reduce](../crates/cq-reduce/src/lib.rs) |
| VO elaboration and correspondence | [cq-elaborate](../crates/cq-elaborate/src/lib.rs) |
| RA erasure and kernel | [relational_plan/kernel.rs](../crates/cq-ir/src/relational_plan/kernel.rs) |

## Scope and evidence

The analysis's width is elimination width plus one over the primal graph;
RA heading arity, cardinality, bytes, treewidth and fractional hypertree
width are different measures. No cardinality estimator, global cost
optimizer, or maximum-arity rejection policy is claimed here.

The current [planning tests](PLANNING-VERIFICATION.md) cover normal form,
hypergraph facts, VO contracts, pass correspondence, guard pushdown,
reduction schedules, and selected CQ/RA answer comparisons. They do not
certify generated execution for every authored reference plan. Physical
lowering, structure selection and realization have their own narrower
integration tests.
