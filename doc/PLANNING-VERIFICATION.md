# Logical planning verification

The logical pass checks stop at RA. They cover normal form, hypergraph
analysis, valuation plans, reductions, and elaboration. The
[pass map](PLANNING.md) describes implementation ownership; Physical
realization has a separate, narrower acceptance domain.

## Active suites and independent oracles

| Suite | Current evidence |
|---|---|
| [`cq-lang-cq/tests/normal.rs`](../languages/cq-lang-cq/tests/normal.rs) | Normalization and kernels up to renaming |
| [`cq-lang-cq/tests/hypergraph_analysis.rs`](../languages/cq-lang-cq/tests/hypergraph_analysis.rs) | Incidence, GYO join trees, chordality, elimination order, width, and distinct occurrences on selected shapes |
| [`cq-lang-vo/tests/valuation_language.rs`](../languages/cq-lang-vo/tests/valuation_language.rs) | VO coverage, reductions, malformed plans and round trips |
| [`cq-order/tests/order.rs`](../passes/cq-order/tests/order.rs) | Written, Prefix and Elimination ordering with Coarse/Fine splits; pinned sequences and order correspondence |
| [`cq-reduce/tests/reduce.rs`](../passes/cq-reduce/tests/reduce.rs) | Guard pushdown, acyclic tree schedules, cyclic rejection, reduction correspondence, and source/target VO answer preservation on generated databases |
| [`cq-elaborate/tests/elaborate.rs`](../passes/cq-elaborate/tests/elaborate.rs) | Exact scenario RA, elaboration correspondence, kernel erasure, and RA/CQ evaluator answers on generated databases |
| [`cq-lang-ra/tests/relational_program.rs`](../languages/cq-lang-ra/tests/relational_program.rs) | RA local contracts and syntax round trips |

The [30 scenario programs](../test-support/relational/scenarios) hold complete
`.cq`, `.vo`, `.ra` and `.physical` programs, a `.policy`, and `.ix` and
`.structures` suffixes assembled into complete programs by `Scenario::ix` and
`Scenario::structured`. Pass tests pin compiling one stage's file to the next
stage's file and require `check` to accept it and to reject mutated targets;
each suite adds its own answer evidence as listed above, using the
[case data](../test-support/relational/cases.rs) and
[semantic evaluators](../test-support/relational/semantics.rs).

The oracles use different algorithms:

- CQ evaluation enumerates complete active-domain valuations and checks atoms.
- RA evaluation interprets the DAG with nested-loop natural joins and set
  projection, retaining every intermediate.
- [VO evaluation](../test-support/relational/valuation_semantics.rs) interprets
  subatoms and reductions step by step.

## Twenty-five known query patterns

The corpus holds these 25 patterns, a Fine (variable-at-a-time) triangle, and
`_reduced` Yannakakis variants of the four YA patterns. Every RA fixture is the
Written/Coarse elaboration, so plans are left-deep in body order and project
once, at the end. Occurrence subscripts below denote separate complete
positional renames; `pi` denotes set projection.

| Pattern / full known RA | Reference shape | Distinguishing point |
|---|---|---|
| [1. Scan](../test-support/relational/scenarios/scan.ra) | `R(x)` | Repeated input rows have set meaning |
| [2. Projection and reordered head](../test-support/relational/scenarios/projection.ra) | `pi_{x,y} R(x,y,z)`, output `(y,x)` | Existential witnesses collapse; heading order does not reorder the answer |
| [3. Two-hop self-join](../test-support/relational/scenarios/self_chain.ra) | `pi_{a,c}(Road_0(a,b) join Road_1(b,c))` | One base table supports two different occurrence mappings |
| [4. Return trip](../test-support/relational/scenarios/return_trip.ra) | `pi_a(Road_0(a,b) join Road_1(b,a))` | Equal variable sets do not erase argument direction |
| [5. Intersection](../test-support/relational/scenarios/intersection.ra) | `R(x,y) join S(x,y)` | All columns are shared; both tuple constraints remain |
| [6. Unary guard](../test-support/relational/scenarios/unary_guard.ra) | `pi_x(R(x,y) join Approved(y))` | Singleton hyperedge filters without supplying a new variable |
| [7. Chain with final projection](../test-support/relational/scenarios/chain_projection.ra) | `pi_{a,d}((R(a,b) join S(b,c)) join T(c,d))` | Final endpoint projection collapses duplicate witnesses; no early pruning |
| [8. Claw](../test-support/relational/scenarios/claw.ra) | `pi_{a,b,c}((A(h,a) join B(h,b)) join C(h,c))` | Cyclic SIP graph, acyclic occurrence hypergraph |
| [9. Fact/dimension star](../test-support/relational/scenarios/fact_star.ra) | `pi_{a,b,c}(((F(x,y,z) join A(x,a)) join B(y,b)) join C(z,c))` | Different overlaps with one intact ternary constraint; final projection only |
| [10. Triangle](../test-support/relational/scenarios/triangle.ra) | `(R(a,b) join S(b,c)) join T(c,a)` | A cyclic body with a composite closing test |
| [11. Four-cycle](../test-support/relational/scenarios/cycle4.ra) | `pi_{a,c}(((R join S) join T) join U)` | Left-deep cycle closing on both endpoint attributes, then projecting `a,c` |
| [12. Cartesian product](../test-support/relational/scenarios/cartesian.ra) | `R(x) join S(y)`, output `(y,x)` | Disconnected inputs still combine; no artificial equality |
| [13. Disconnected existential component](../test-support/relational/scenarios/existential.ra) | `pi_x(R(x) join S(y,z))` | Cartesian join, then projection onto `x`; emptiness of `S` remains significant |
| [14. Ternary guard](../test-support/relational/scenarios/ternary_guard.ra) | `(A(d,i) join B(i,t) join C(d,t)) join Stock(d,i,t)` | Pairwise witnesses admit a triple that the full tuple test rejects; universal hyperedge makes this body acyclic |
| [15. Composite overlap](../test-support/relational/scenarios/composite.ra) | `pi_{x,y}(R(a,b,x) join S(b,a,y))` | Both joined columns must respect different source-column orders |
| [16. Four-edge chain](../test-support/relational/scenarios/bushy.ra) | `pi_{a,e}(((R join S) join T) join U)` | Final endpoint projection over a longer chain |
| [17. Full delivery reduction](../test-support/relational/scenarios/ya_delivery_reduced.ra) | `D ⋉`-reduced `S`, then `N`; downward `S`, then `D`; then `D,S,N` assembly | Explicit semijoins, fresh versions of reduced relations, and final projection |
| [18. Oriented self-joined chain](../test-support/relational/scenarios/oriented_chain.ra) | `pi_{a,d}(Edge(a,b) join Edge(b,c) join Edge(d,c))` | Three occurrences of one input, including lookup through the last atom's second source column |
| [19. Spade](../test-support/relational/scenarios/spade.ra) | `(R(x,y) join S(y,z) join T(x,z)) join H(x,w)` | A cyclic core with an acyclic appendage; graph reduction must not erase the tail from execution |
| [20. Barbell](../test-support/relational/scenarios/barbell.ra) | Join two triangles through `Bridge(x,u)` | Seven-occurrence left-deep plan connecting two cyclic regions |
| [21. Loomis–Whitney-4](../test-support/relational/scenarios/loomis_whitney_4.ra) | `ABC(a,b,c) join BCD(b,c,d) join ACD(a,c,d) join ABD(a,b,d)` | A cyclic hypergraph of ternary atoms; one composite extension followed by two full ternary tests |
| [22. Deep Yannakakis schedule](../test-support/relational/scenarios/ya_deep_reduced.ra) | Four-edge chain rooted at `U`; three upward and three downward semijoins | Propagation through depth three; ancestors use reduced children and descendants use the latest parents |
| [23. Composite Yannakakis separators](../test-support/relational/scenarios/ya_composite_reduced.ra) | `R(a,b,u), S(a,b,c), T(b,c,v)`, rooted at `T` | Both passes use entire two-column separators `{a,b}` and `{b,c}` |
| [24. Disconnected Yannakakis schedule](../test-support/relational/scenarios/ya_disconnected_reduced.ra) | `R(x), S(y,z)` with an empty-separator tree edge | Both reduction directions propagate emptiness; the head-free component remains an existence obligation |
| [25. Cartesian-first candidate](../test-support/relational/scenarios/cartesian_first.ra) | `pi_{a,d}((R(a,b) join S(b,c)) join T(c,d))` | The fixture pins the written connected order; a Cartesian-first `R × T` combination is legal but not pinned |

The triangle, four-cycle, spade, barbell, and Loomis–Whitney-4 are
alpha-cyclic; the other 20 bodies are alpha-acyclic. Current hypergraph
tests check selected shapes and preserve distinct occurrence identities.
The full catalog is a reference corpus, not an exhaustive graph oracle.

All ten families in the [classic workload catalog](../workloads/src/catalog/classic.rs)
have counterparts in this reference catalog.
This is family coverage, not 25 distinct topologies or exhaustive coverage of
all possible join graphs. Intersection uses an all-columns-shared variant here;
some fixtures also choose a different head projection from the catalog.

## Stronger checks than final answers

The VO contract rejects uncovered atom clauses, a subatom that omits an
already-bound column, early guards, and reductions placed after their
clause's first step. The order tests pin representative Written, Prefix,
and Elimination sequences for both splits. The reduction tests pin an
acyclic two-pass schedule, guard pushdown, and refusal of tree reduction
on a cyclic query.

Each boundary after normalization has `check(source, target, options…)`: an
independent correspondence that establishes the target contract, the
erasure relation to the source (kernels compare up to renaming), and the
obligations of the selected options, without re-running the planner.
Selected tests also compare elaborated RA output with authored answers or
independent CQ valuations. Final answer equality alone does not inspect every
intermediate reduction relation.

The graph tests distinguish a chordal but nonconformal triangle from
acyclic chains and stars, and keep repeated relation occurrences distinct.
GYO removes residual analysis edges without removing executable atoms.

Reduced RA fixtures use explicit `semijoin`; evaluators may implement its
semantics with matching and projection. Physical realizes it as `semi`
(membership on the key).

## Reproduce current checks

```sh
cargo test -p cq-lang-cq --test normal --test hypergraph_analysis
cargo test -p cq-lang-vo --test valuation_language
cargo test -p cq-lang-ra --test relational_program
cargo test -p cq-order
cargo test -p cq-reduce
cargo test -p cq-elaborate
cargo test -p cq-index-policy
cargo test -p mini-linq --test physical_query
```

These commands name the current pass and integration suites.

The [full-chain test](../facade/mini-linq/tests/physical_query.rs) exercises
every stage after IX: hosts read in place through their declared keys,
compiler-built hash indexes, a user-defined index type through
`#[index(Type)]`, merge joins over built ordered maps, Yannakakis semijoin
passes as membership tests, a bounded scan of an ordered host, and a
computed column, each checked against independent relational enumeration.
This is additional physical coverage of supported queries, not compiled
execution of all 25 reference plans.
