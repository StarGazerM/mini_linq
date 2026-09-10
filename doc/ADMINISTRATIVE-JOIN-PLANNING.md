# Administrative join planning: CQ to planned relational algebra

For the exact textbook terminology, course adaptations, and proof status, see
the [LaTeX specification](formal/README.md) and its
[source correspondence](formal/TERMINOLOGY.md). In mathematical prose below,
**sort** is AHV's finite attribute set; source identifiers such as
`retained_heading` retain their implementation spelling.

Status: compiler design following notes revision `3f97151`, with R2 organized
into Stage I (graph planning, compiled RA, and Yannakakis) and Stage II (WCOJ
and cardinality estimation), as decided on 2026-09-06. Stage I is implemented
through the existing language boundaries; see the
[pass implementation plan](PLANNED-PASS-IMPLEMENTATION.md) and
[verification matrix](STAGE-I-VERIFICATION.md). The released R1 route and
public points/dates are unchanged.

The objective is an executable planner whose steps have explicit premises,
state changes, emitted algebra, and preservation obligations. Graph algorithms
choose applications of these rules. The resulting artifact is the existing
named relational algebra, extended with semijoin for the Yannakakis route.

## 1. Scope and boundaries

Start with the implemented CQ fragment: one positive rule, finite set
relations, ordered arguments, distinct variables within each atom and the
head, and every head variable bound in the body. Comparisons are now a
bounded extension described below. Constants and repeated terms within atoms,
negation, and aggregation remain outside this planner's fragment.
Different occurrences may use the same declared relation. Shared variables
must have compatible Rust types; the baseline delegates type compatibility to
Rust rather than comparing type spellings.

```text
checked CQ
  -> occurrence and variable incidence analysis
  -> administrative planning derivation
  -> RelationalPlan                         // named set-valued RA
  -> physical operator and storage choices
  -> executable Rust
```

The acyclic strategy adds a join-tree witness and a reduction schedule inside
the planning derivation. The incidence analysis and derivation can begin as
internal Rust values with an inspection format. They do not require new public
macro languages.

| Object | Meaning | Decisions it records |
|---|---|---|
| Occurrence hypergraph | Which variables each complete atom constrains | None about evaluation order |
| SIP graph | Which pairs of occurrences share variables | None about direction or cost |
| Join-tree witness | A tree satisfying running intersection | A structural organization of occurrences |
| Planning derivation | Rule applications with their premises and results | Combination, projection, and reduction schedule |
| RelationalPlan | A DAG of named algebra results | Exact logical evaluation operations |
| Physical plan | Concrete implementations of those operations | Indexes, materialization, traversal, and ownership |

In particular, a join tree has occurrence nodes; a binary evaluation tree has
intermediate join nodes. They are different objects.

## 2. Query analysis rules

Write the body as occurrence IDs `B = {0, ..., m-1}`. For occurrence `i`, keep
the original atom `A_i = R_i(v_i0, ..., v_in)`, its ordered argument mapping,
and the variable set `X_i`. Let `H` be the head-variable set; retain the
original ordered head separately. Define:

\[
X(C)=\bigcup_{i\in C}X_i,
\qquad
K(C)=X(C)\cap\left(H\cup X(B\setminus C)\right).
\]

`K(C)` is the boundary of a completed occurrence set: variables needed by the
head or by occurrences outside that set. This is a sufficient retained set
for this fragment, without assuming functional dependencies or reconstructible
values. It depends on the original query, even after intermediate projection.

**A-OCCURRENCE.** Assign one stable ID per written body occurrence; record its
declaration-to-argument mapping and `X_i`. Equal variable sets never merge
occurrences. `road(a,b)` and `road(b,a)` remain distinct tuple tests.

**A-INCIDENCE.** Add `(i,x)` exactly when `x` occurs in `A_i`. Keep one
occurrence-labeled hyperedge per `i`, together with the ordered atom. Incidence
sets alone cannot reconstruct argument positions.

**A-SIP.** For each distinct pair `i,j`, derive a labeled edge precisely when
`X_i ∩ X_j` is nonempty; its label is the entire intersection. The SIP graph
is a view of incidence, so its labels need not be stored independently.

**A-BOUNDARY.** For each candidate completed set `C`, compute `K(C)` from the
original occurrence incidence and head. A variable omitted from `K(C)` must
have no use outside `C` and no head use.

These rules inspect syntax, not input tuples. Cardinality estimates, when
introduced, are separate policy inputs.

## 3. Administrative rules for answer assembly

Fix a family of occurrence relations `L_i` with sorts `X_i`. Initially
`L_i = δ_{m_i}(I(R_i)) = Val_I(A_i)`, where `m_i` is the complete positional
attribute renaming and `Val_I` is the course's valuation relation. Section 6
also permits a frozen, reduced family whose complete join is unchanged.

A planning state is `(D,F)`: `D` is the sequence of emitted named definitions,
and `F` is a frontier of entries `(C,V,r)`. An entry means:

\[
\operatorname{sort}(r)=V,
\qquad K(C)\subseteq V\subseteq X(C),
\qquad
\llbracket r\rrbracket=\pi_V\left(\mathop{\bowtie}_{i\in C}L_i\right).
\tag{Assembly invariant}
\]

Frontier occurrence sets partition `B`. An emitted definition is immutable;
each new result gets a fresh ID. Replacing a frontier entry does not delete
the definitions on which its result depends. `emit` below appends a typed RA
definition to `D` and returns its result ID.

### P-START

Emit one complete `Rename` per occurrence and initialize:

\[
F=\{(\{i\},X_i,r_i)\mid i\in B\}.
\]

For assembly after reduction, use the final reduced occurrence references in
place of the initial `r_i`, retaining the reduction definitions in `D`.
Do not project occurrence schemas before completing the reduction passes.

### P-COMBINE

Choose two frontier entries `(C,V,r)` and `(E,W,s)` with `C ∩ E = ∅`.

```text
t = emit(natural_join r with s)
replace the two entries by (C ∪ E, V ∪ W, t)
```

The join tests agreement on **every** variable in `V ∩ W`. All original
crossing variables are still present: `X(C) ∩ X(E) ⊆ V ∩ W`, by the boundary
premises. Thus the two named tuples combine exactly when their shared attributes agree.

If the intersection is empty this is a Cartesian product, which is legal.
If `W ⊆ V`, the join only filters the left relation under set semantics.
These are derived cases of natural join, not additional logical operators.

### P-TRIM

For a frontier entry `(C,V,r)`, choose a proper subset `U` with
`K(C) ⊆ U ⊂ V`:

```text
t = emit(project r keep U)
replace the entry by (C, U, t)
```

The default eager policy chooses `U = K(C)`. Identity projections can be
omitted. Projection has set semantics, including duplicate elimination.
An empty sort is valid: projecting a nonempty relation onto `{}` produces
`{()}`, whereas projecting an empty relation produces the empty relation.
Consequently a disconnected component with no head variables still checks
existence; it cannot simply disappear.

### P-OUTPUT

When the sole frontier entry is `(B,V,r)`, trim to `H` if necessary and emit
`output r as <original ordered head>.` Its denotation is the source CQ.
Output tuple order belongs to this metadata, not to sort-set iteration.

### Preservation and termination

P-START establishes the assembly invariant. P-COMBINE preserves it because
neither side has forgotten an equality needed by the other. P-TRIM preserves
it by projection composition and the remaining-use premise. At P-OUTPUT,
`K(B) = H`, giving the CQ result.

Each combination reduces the frontier size by one. Between combinations,
each trim strictly reduces an entry's finite sort. A policy that continues
combining until one entry remains therefore terminates. These rules construct
sound plans; they do not enumerate every equivalent RA rewrite or perform CQ
minimization.

### Rule-to-code correspondence

The state above is `JoinState` in
[rules.rs](../crates/cq-compiler/src/cq_to_relational/rules.rs). Its `apply`
dispatches to one method per transition: `introduce` (P-START), `select`
(P-SELECT), `combine` (P-COMBINE), `trim` (P-TRIM), and `output` (P-OUTPUT).
`retained_heading` computes K(C,P). `begin` (P-BEGIN) requires all ready local
selections and enters reduction when a checked tree is present, otherwise
assembly. `up`, `down`, and `freeze` implement Y-UP, Y-DOWN, and Y-FREEZE.
Up and Down have distinct dependency premises and share only the emitted
semijoin operation. `into_module` requires an already finished state.

[hypergraph.rs](../crates/cq-compiler/src/cq_to_relational/hypergraph.rs)
contains G-PRIVATE (`GyoState::remove_private`) and G-ABSORB
(`GyoState::absorb`); those transitions change only the analysis graph.
The [complete mapping](PLANNED-PASS-IMPLEMENTATION.md#administrative-rules-and-contracts)
also points to the [expression judgments and prefix rules](RUST-EXPRESSIONS.md#dependency-judgments-and-their-code).
Their separate domain and current limitations are explicit there.

### Enforced safety and eager-policy contracts

The planned edge returns an existing `relational_plan::Module`; a separate
`JoinPlanReport` records GYO and administrative steps for diagnostics.
`check_cq_plan(source, target, report)` reads the actual emitted RA and checks
occurrence mapping, phase/dependency premises, current references, coverage,
retention, and output. It does not invoke the emitting rules or scheduling
policy to regenerate one preferred plan. RA local validity remains separate.

Each rule checks its fallible premises before emitting one typed definition
and updating its state. Reduction owns a current-reference vector. Freeze
consumes that reduction state and transfers final references to assembly.
Independent sibling reductions may be reordered; parent/child dependencies
and completion of both passes are mandatory.

All policies in this graph-rule domain use eager boundary pruning. The optional
`max_arity` bounds every named result, including full occurrence renames and
joins before projection. It is not a cardinality or fractional-width bound.
The GreedyWidth policy minimizes the next candidate's materialized column
count over frontier pairs and makes no claim of global optimality.

Selection extends retention to pending predicate variables:

```text
K(C,P) = X(C) intersect (H union X(B minus C) union vars(P))
```

CQ spells a predicate as `if (x < y)` and RA as `select r if (x < y)`.
The fragment consists of i32 variable/literal comparisons `== != < <= > >=`.
Logical checking verifies name availability; Rust resolves types and aliases.
The lowered operands are explicitly ascribed primitive i32, so overloaded
Rust comparisons cannot introduce effects into a movable predicate.
Predicates are applied as soon as their variables fit one frontier entry;
pruning then removes their last uses. Arithmetic and arbitrary calls are not
part of this logical fragment.

**P-SELECT.** Extend the administrative state with the pending predicate set
`P`. If `p ∈ P` and `vars(p) ⊆ V` for frontier entry `(C,V,r)`, perform:

```text
t = emit(select r if p)
replace (C,V,r) by (C,V,t)
P = P minus {p}
```

For assembly with predicates, the global preservation equation is
`pi_H(select_and(P)(join { value(r) | (C,V,r) in F })) = CQ(I)`.
P-SELECT moves one conjunct into a relation containing all its variables.
P-TRIM uses `K(C,P)`, so it cannot remove a variable used by a remaining
conjunct. P-COMBINE preserves the join, and P-OUTPUT requires `P` empty.
Local selection uses the same command before Begin; assembly selection runs
after Begin on the binary route or after Freeze on the reduction route.

Local selections precede reduction. A cross-occurrence predicate may remain
for assembly. In that case the freeze full-support theorem describes the
locally selected positive occurrence family, not the complete theta-filtered
query. Remaining predicates are still required exactly once before output.

The historical fixed-order join/project witness checker now lives only in
[reference tests](../crates/cq-compiler/tests/stage1_reference/verifier.rs).
It continues to check independently authored RA fixtures; it is neither a
production IR nor a required backend input. The materialized pass accepts
locally valid indexed RA without a CQ-planning certificate.

## 4. Policies make this a planner

The first implementation should supply two deterministic strategies:

1. **Written order:** combine singleton occurrences into a left-deep chain in
   source order. This provides an interpretable baseline. Eager trimming is a
   separate choice; the exact R1 baseline keeps only its final projection.
2. **Connected preference:** start at the least occurrence ID; at each step
   prefer an unused occurrence sharing a variable with the current group,
   breaking ties by occurrence ID. If none shares a variable, choose the
   least remaining ID and emit the necessary Cartesian step. Trim eagerly.

Both strategies choose legal rules; neither claims cheapest execution.
Connected preference must not become a legality restriction: the notes give
a Cartesian-first counterexample under their row-volume proxy.

The rule system also supports bushy combinations. A later greedy or subset-DP
policy can search that space using an explicit cost model. Source order,
deterministic graph preference, and cost-based search remain separate policy
choices. Initial R2 work requires no cardinality estimator or cost optimizer.

## 5. Administrative hypergraph reduction and join-tree extraction

The route decision checks **alpha-acyclicity** of the entire body. Stronger
beta/gamma/Berge conditions are not required for this route, and alpha-cyclic
bodies remain supported by ordinary binary planning. In particular, a
subfamily of an alpha-acyclic body's atoms can be cyclic. Head-sensitive
free-connex acyclicity is an additional consideration for projected-answer
enumeration guarantees, not an extra premise for correct Stage I execution.
The [acyclicity distinctions and regressions](STAGE-I-VERIFICATION.md#which-acyclicity-is-checked)
give precise scope and references.

To recognize alpha-acyclicity, maintain an active occurrence set `A`, residual
variable sets `Z_i` for active occurrences, a forest on all original
occurrences, and a reduction trace. Initially `A = B`, `Z_i = X_i`, and every
occurrence is a separate forest root. Active occurrences are exactly the
forest roots. Original `X_i` and atom labels are immutable.

**G-PRIVATE.** If `x ∈ Z_i` occurs in no other active residual set, remove `x`
from `Z_i` and record the step.

**G-ABSORB.** If distinct active occurrences satisfy `Z_i ⊆ Z_j`, attach the
forest rooted at `i` under `j`, remove `i` from `A`, and record `(i,j)`.
Equal sets are allowed, with distinct IDs. Empty sets are allowed too; an
empty separator can connect otherwise disconnected components.

**G-FINISH.** If one active root remains with an empty residual set, return
the recorded tree. A one-atom query succeeds after G-PRIVATE removes its
variables and has a one-node tree.

**G-STUCK.** If none of these rules applies, return `Cyclic` with the residual
and trace. Ordinary assembly from Section 3 remains available.

This is a GYO-style reduction with attachment recording. Implement a fixed
tie order, such as private-variable removals first and then the lexicographically
least eligible `(i,j)` absorption. Each reduction decreases the active
occurrence count or the number of active incidences.

**G-CHECK-WITNESS.** Independently verify the extracted tree on the original
sets `X_i`: every occurrence appears once, it is a tree, and for every variable
the occurrences containing that variable induce a connected subtree. Label
each tree edge `(i,j)` by the full original intersection `X_i ∩ X_j`.
Residual sets are not the source of execution keys. A failed witness check is
an implementation error, not a reason to report the source query as cyclic.

G-PRIVATE is not a projection instruction. G-ABSORB does not delete an
executable atom or its existence test. These transformations operate on
analysis state only. In particular, duplicate-looking hyperedges and empty
residual groups retain their original occurrence nodes in the witness.

## 6. From a join tree to a Yannakakis plan

RelationalPlan includes the following logical operator:

```text
Semijoin ::= "semijoin" RelationId "with" RelationId
```

Both operands must name earlier results. Its sort is the left sort,
and its set denotation is:

\[
\llbracket\operatorname{semijoin}\;r\;\operatorname{with}\;s\rrbracket
=\pi_{\operatorname{sort}(r)}
  (\llbracket r\rrbracket\bowtie\llbracket s\rrbracket).
\]

The right operand may be an intermediate result. Empty shared sorts mean
an existence test on the right. Multiple shared attributes form one joint
key; separate per-column witnesses are insufficient.

### Y-START and Y-FILTER

Given a checked join tree `T`, choose a root and deterministic child order.
Let `I_i` be each full-sort renamed occurrence after its assigned local
selections; in the positive-only fragment, `I_i = rho_i(I(R_i))`.
Keep a compile-time map
`current[i]` from occurrence IDs to their latest RA references.

For a scheduled directed filtering step `i ← j`, let `r = current[i]` and
`s = current[j]`, then:

```text
t = emit(semijoin r with s)
current[i] = t
```

The assignment updates the compiler's reference map. Generated RA uses fresh
immutable definitions, not in-place updates or cyclic dependencies.

At every step, occurrence schemas remain `X_i`, each current relation is a
subset of its original renamed input, and:

\[
\mathop{\bowtie}_i\llbracket\operatorname{current}[i]\rrbracket
=\mathop{\bowtie}_i I_i.
\tag{Reduction invariant}
\]

Semijoins preserve this complete join even outside the acyclic case. This is
a different invariant from assembly: a reduced occurrence may depend on many
other occurrences, so RA dependency sets are not disjoint assembly ownership.

### Y-UP

Visit non-root nodes in reverse breadth-first order. After all descendants
of child `c` have been processed, apply Y-FILTER `p ← c` on its edge to
parent `p`. A parent with several children uses its latest result for each
successive filter. Any children-before-parent schedule satisfies this rule.

### Y-DOWN

After Y-UP completes, visit non-root nodes in breadth-first order. For each
parent-child edge, apply Y-FILTER `c ← p` before visiting that child's
descendants. The parent reference must include its own top-down reduction,
when it has a parent. Any parent-before-child schedule satisfies this rule.

There are `m-1` filters in each pass. The empty-separator edges used for
disconnected components propagate emptiness by the same rules.

### Y-FREEZE and Y-ASSEMBLE

After both passes, freeze `L_i = current[i]`. On a checked join tree, the
full-reducer guarantee is:

\[
\llbracket L_i\rrbracket
=\pi_{X_i}\left(\mathop{\bowtie}_j I_j\right).
\]

Every surviving occurrence tuple has complete support. Now initialize the
assembly frontier from this frozen family and apply P-COMBINE/P-TRIM/P-OUTPUT.
For the initial Yannakakis strategy, use a root-first traversal of `T`, joining
each next occurrence after its parent has joined the current group.

The complete route is recognition, witness checking, two reduction passes,
and answer assembly. Recognition alone gives a structural planner; the two
passes alone give a full reducer. A cyclic residual selects the ordinary
planner, without claiming global consistency from pairwise filtering.

### Physical obligations

Preserve `Semijoin` through logical planning so physical lowering can implement
it by existence probes without enumerating every matching pair. The
join-project equation defines its meaning and can support a reference
interpreter; it does not specify an efficient implementation.

The Stage I physical route materializes named set results and
builds indexes over intermediates through IndexRequirements and staged Rust. This makes every RA boundary executable and
its cardinality inspectable. Use the following fixed choices before adding a
cost model:

| Logical operation | Initial physical implementation |
|---|---|
| `Rename` | Read the normalized input set using the occurrence's complete positional mapping |
| `NaturalJoin` with shared variables | Build a right-side hash multimap on the entire shared key; probe with each left row and insert combined rows into a result set |
| `NaturalJoin` with no shared variables | Enumerate the Cartesian product into a result set |
| `Semijoin` with shared variables | Build a set of right-side composite keys; retain each left row exactly when its complete key is present |
| `Semijoin` with no shared variables | Retain the left set exactly when the right set is nonempty |
| `Project` | Construct the retained tuple and insert into a result set, including the empty-tuple case |
| `Unit` | Construct the singleton set containing the empty tuple |

Evaluate definitions in dependency order, once per definition; borrow
predecessor sets while constructing owned result rows, and retain results
needed by subsequent consumers. Shared DAG operands reuse the same result.
Keep intermediate tuple layout deterministic and separate from sort-set
semantics; the output uses the source head's explicit order. Fixed right-side
index construction needs no cardinality estimator. Account for its cost in
the work counters.

This is an explicit physical choice for the initial compiled route. Logical
`rN` naming does not itself require materialization, and later streaming or
fusion can use the same logical plan with their own execution contracts.

Claim Yannakakis performance only after specifying the join/probe algorithms,
index construction costs, and measurements. In particular, the number of
distinct projected head tuples may be much smaller than the full join. This
specification does not infer time linear in input plus projected output from
acyclicity, or constant-delay enumeration for arbitrary projected CQs.

## 7. Complete target example

This is the extended RA syntax for the delivery query. Choose `S` as
the join-tree root. Process children in order `D,N` during reduction, then
choose the root-first assembly order `S,N,D`. Reduction order and assembly
order are separately recorded policy choices.

```text
struct EligibleProgram;
relation based_at(driver_id: i32, depot_id: i32);
relation stocks(depot_id: i32, item_id: i32);
relation needs(job_id: i32, item_id: i32);
eligible(driver, job) :-
    based_at(driver, depot), stocks(depot, item), needs(job, item);
relational {
    d0 = rename based_at {driver_id -> driver, depot_id -> depot};
    s0 = rename stocks {depot_id -> depot, item_id -> item};
    n0 = rename needs {job_id -> job, item_id -> item};
    s1 = semijoin s0 with d0;
    s2 = semijoin s1 with n0;
    d1 = semijoin d0 with s2;
    n1 = semijoin n0 with s2;
    sn = natural_join s2 with n1;
    live = project sn keep {depot, job};
    all_rows = natural_join live with d1;
    result = project all_rows keep {driver, job};
    output result as eligible(driver, job);
}
```

Every operand names an earlier result, every definition is reachable from
output, and original occurrences remain represented even if their reduced
relations happen to be equal on an instance. `item` is forgotten after `S,N`
are assembled; `depot` survives until the remaining `D` agreement is tested.
No tuple counts were needed to justify either projection.

## 8. Mapping to the current compiler

| Current code | Required change for this design |
|---|---|
| [`cq.rs`](../crates/cq-ir/src/cq.rs) | Reuse checked atoms and declarations; construct occurrence IDs and incidence without expanding the language |
| [`relational_plan.rs`](../crates/cq-ir/src/relational_plan.rs) | Reuse general binary `NaturalJoin`, `Project`, named DAG references, inferred sorts, and output metadata; add `Semijoin` for Section 6 |
| [`staff_r1.rs`](../crates/cq-compiler/src/staff_r1.rs) `compile_cq` | Retain the canonical HW1 route; add a separately named planner entry using these rules |
| [`pass_contracts.rs`](../crates/cq-compiler/src/pass_contracts.rs) `relational_plan_is_exact` | Retain the R1 exact shape contract; give the planner a derivation-preservation contract instead |
| `basic_left_deep_plan` and `lowerable_access_sequence` in those compiler files | Supply a physical route accepting the planned algebra; current code accepts direct right-hand renames and one final projection |
| [`index_requirements.rs`](../crates/cq-ir/src/index_requirements.rs) | Existing source-relation keys do not describe indexes over reduced/intermediate results; define such physical requirements at the new route's boundary |
| [`iterator_pipeline.rs`](../crates/cq-ir/src/iterator_pipeline.rs) | Current linear pull operators do not execute a shared reduction DAG; supply the required execution support explicitly |

Local RA well-formedness remains a sort/reference judgment. It does not
prove that a plan implements a particular CQ. The planner's edge contract
must account for occurrence coverage, safe forgetting, and reduction
preservation. Check rule traces and results independently; do not define
correctness by regenerating one preferred plan and comparing its spelling.

## 9. R2 stages and acceptance criteria

R2 has two internal stages. They belong to the cumulative project and are
distinct from Basic HW2, which means index selection in R1.

| R2 stage | Implementation scope | Completion artifact |
|---|---|---|
| **Stage I: graph planning and compiled RA** | Occurrence analysis, administrative join/projection rules, deterministic policies, GYO reduction, checked join-tree extraction, semijoin scheduling, and answer assembly | An inspectable CQ-to-RA derivation and executable compiled RA, using Yannakakis for acyclic bodies and the ordinary binary strategy for cyclic bodies |
| **Stage II: WCOJ and cardinality estimation** | Variable-at-a-time execution, trie access and leapfrog intersection, cardinality estimates, and explicit policies that use estimates to choose variable orders/access arrangements | A multiway physical plan with its variable order and accesses exposed, compared with Stage I on identical queries and input data |

Stage I includes join-tree discovery in the intended implementation. Supplied
trees remain useful intermediate fixtures while the graph algorithm is being
implemented. Staff provides typed syntax, rule and witness checkers, oracle
infrastructure, and executable backend scaffolding for the student-facing
release.

For Stage II, separate two contracts: a supplied valid variable order must
produce correct WCOJ execution, and estimates may improve the chosen order or
access arrangement. In particular, Leapfrog Triejoin can execute a fixed
variable order; an accurate cardinality estimator is not a correctness
precondition. Its variable-order contract is described in
[Veldhuizen, Section 3](https://www.openproceedings.org/2014/conf/icdt/Veldhuizen14.pdf).
Estimate error can change work without changing answers. Specify estimation
inputs and quality measurements in the Stage II design; that work does not
block Stage I.

Stage II reuses occurrence identities, query incidence, the original head,
and semantic oracles. Its multiway execution must consider the participating
atoms together; simply reordering a sequence of binary `NaturalJoin`
operators does not implement it. Give that execution choice an explicit
physical representation when Stage II is designed.

The current public [`project.md`](https://github.com/StarGazerM/modern-query-processing-notes/blob/3f971516f8784ead40e8635a85dbe319ba5e4729/project.md) still describes supplied
join trees and triejoin under its earlier stage numbering. This document
records the revised R2 implementation scope; assigning student functions,
points, and dates belongs to the release handout. No new points or dates are
assigned here.

### Stage I implementation order

1. Add occurrence analysis and independently checked graph witnesses.
2. Implement administrative assembly and deterministic binary policies,
   emitting the existing RA fragment with intermediate projections.
3. Add logical `Semijoin` and emit the two reduction passes plus assembly.
4. Compile the complete resulting DAG using the physical choices in Section 6.
5. Compare CQ valuation, RA interpretation, and compiled Rust results; expose
   the rule trace and operator work counters.

Stage I is complete when the resulting plans execute correctly. Producing a
valid graph witness or parsable algebra is an intermediate checkpoint.

The concrete [Stage I verification matrix](STAGE-I-VERIFICATION.md) supplies
25 authored CQ/RA/input/output fixtures and the executable reference suite.
It records independent reference checks alongside planner contracts, actual
compiled execution, and the complete typed boundary artifacts.

The Stage I implementation gate includes the following semantic cases:

- All deterministic policies agree with a direct valuation-based CQ oracle;
  RA interpretation and compiled execution also agree on the same instances.
- An outward road without a return road fails `road(a,b), road(b,a)`; equal
  hyperedge sets do not merge those constraints.
- A ternary relation's pairwise projections admit a spurious triple; the
  original atom and composite-key semijoin reject it.
- The common-variable star has a cyclic SIP graph but admits a join tree;
  the triangle fails running intersection for every candidate tree.
- Single-atom, duplicate-occurrence, disconnected, and empty-input cases
  preserve occurrence coverage and empty-sort existence tests.
- Premature projection fails on the notes' depot counterexample; valid
  trimming merges duplicate existential witnesses without changing answers.
- On acyclic inputs, each reduced occurrence equals the corresponding
  projection of the complete join; the cyclic two-value triangle is a
  counterexample to claiming that pairwise support is complete support.
- The complete target above executes both passes with fresh results and
  uses the latest parent references during the top-down pass.
- Inspect at least one bushy assembly plan and one intermediate projection;
  successful parsing alone is not executable-backend coverage.

Record operator row counts, retained arities, semijoin probes, and index
construction work separately. A graph heuristic's favorable count on one
fixture is evidence about that fixture, not an optimality claim.

### Design validation snapshot

On 2026-09-06, a standalone simulation of the stated graph rules agreed with
independent enumeration of candidate join trees on 2,165 small hypergraphs
(up to four variables and four occurrences, including duplicate sorts).
The delivery example and its full-reducer invariant agreed with a direct CQ
valuation oracle on all 4,096 input families over a two-value domain.
Empty-sort projection behavior was also checked. These are bounded checks
of this specification. The production implementations and their acceptance
suite now use the existing typed pass chain; current gates are recorded in
[STAGE-I-VERIFICATION.md](STAGE-I-VERIFICATION.md).

## Sources and derivation

The occurrence convention, whole-tuple constraints, retained-set rule, and
planning counterexamples come from the course notes at `3f97151`:
[2.2 SIP graph](https://github.com/StarGazerM/modern-query-processing-notes/blob/3f971516f8784ead40e8635a85dbe319ba5e4729/04-the-picture-that-did-not-choose.md),
[2.3 hypergraph](https://github.com/StarGazerM/modern-query-processing-notes/blob/3f971516f8784ead40e8635a85dbe319ba5e4729/04-the-variables-inside-the-picture.md),
[2.4 remaining uses](https://github.com/StarGazerM/modern-query-processing-notes/blob/3f971516f8784ead40e8635a85dbe319ba5e4729/05-the-variables-the-plan-still-needed.md), and
[2.5 planning and join trees](https://github.com/StarGazerM/modern-query-processing-notes/blob/3f971516f8784ead40e8635a85dbe319ba5e4729/06-the-order-the-compiler-chose.md).
The notes remain a separate repository; the local `doc/notes` checkout is not
part of this commit. References above use its pinned revision:
[pinned upstream](https://github.com/StarGazerM/modern-query-processing-notes/tree/3f97151).

The theoretical basis for hypergraph reduction, running intersection, and
full semijoin reducers is Abiteboul, Hull, and Vianu,
[*Foundations of Databases*, Chapter 6, Section 6.4](https://webdam.di.ens.fr/Alice/pdfs/Chapter-6.pdf),
especially Theorem 6.4.5. The named administrative states, transitions,
immutable result schedule, and compiler contracts above are the
course implementation formalization.
