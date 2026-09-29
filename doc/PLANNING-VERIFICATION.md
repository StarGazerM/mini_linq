# Logical planning verification

The logical pass checks stop at RA. They cover normal form, hypergraph
analysis, valuation plans, reductions, and elaboration. The
[pass map](PLANNING.md) describes implementation ownership; Physical
realization has a separate, narrower acceptance domain.

## Active suites and independent oracles

| Suite | Current evidence |
|---|---|
| [`cq-ir/tests/valuation_language.rs`](../crates/cq-ir/tests/valuation_language.rs) | Normalization, kernels up to renaming, coarse/fine VO coverage, reduction placement, and malformed-plan rejection |
| [`cq-ir/tests/hypergraph_analysis.rs`](../crates/cq-ir/tests/hypergraph_analysis.rs) | Incidence, GYO join trees, chordality, elimination order, width, and distinct occurrences on selected shapes |
| [`cq-order/tests/order.rs`](../crates/cq-order/tests/order.rs) | Written, Prefix and Elimination ordering with Coarse/Fine splits; VO contracts, pinned sequences, and kernel equality |
| [`cq-reduce/tests/reduce.rs`](../crates/cq-reduce/tests/reduce.rs) | Guard pushdown, acyclic tree schedules, cyclic rejection, pass correspondence, and CQ/RA answers over the shared cases |
| [`cq-elaborate/tests/elaborate.rs`](../crates/cq-elaborate/tests/elaborate.rs) | Named RA shapes, pass correspondence, kernel preservation, and selected generated-database comparisons |
| [`cq-ir/tests/relational_plan.rs`](../crates/cq-ir/tests/relational_plan.rs) | RA contract and erasure checks |

The [25 authored `.ra` files](../test-support/relational/plans),
[case data](../test-support/relational/cases.rs), and
[semantic evaluators](../test-support/relational/semantics.rs) remain checked
in. The reduction tests read the authored files as query sources and compare
their generated RA with fixed answers and CQ valuations on generated
databases. The authored RA shapes remain useful reference examples; the
current suite does not compare every generated intermediate with those
shapes.

The oracles use different algorithms:

- CQ evaluation enumerates complete active-domain valuations and checks atoms.
- RA evaluation interprets the DAG with nested-loop natural joins and set
  projection, retaining every intermediate.
- [Join-tree enumeration](../test-support/relational/join_trees.rs) is retained
  as a reference helper; current hypergraph tests check selected join trees
  and analysis facts.

The CQ/RA evaluators share integer/set carriers and name normalization, but
do not share ordering, reduction or index analysis. Fixed authored answers
help expose shared misunderstandings. The current hypergraph tests cover
specific chain, star, cycle and self-join cases; they do not claim exhaustive
enumeration of small hypergraphs.

The checked correspondences are:

```text
CQ-NF kernel
    = VO kernel
    = RA kernel

authored answer on shared cases
    = interpretation of the reduced, elaborated RA plan

CQ valuation on generated databases
    = interpretation of the reduced, elaborated RA plan
```

`cq::normal::check` validates CQ-NF. `cq_order::check(source, target, options)`
and `cq_reduce::check(source, target, options)` check their VO boundaries;
`cq_elaborate::check(source, target)` checks RA correspondence. The
erasure-based kernel comparison is up to variable renaming. This is logical
validation; an authored RA fixture alone does not show that a physical
policy can execute it.

## Twenty-five known query patterns

Occurrence subscripts below denote separate complete positional renames.
The linked files spell out the actual typed syntax. `pi` denotes set
projection; parentheses fix the reference combination tree. `YA` expands
semijoins into join/project for interpretation only.

| Pattern / full known RA | Reference shape | Distinguishing point |
|---|---|---|
| [1. Scan](../test-support/relational/plans/scan.ra) | `R(x)` | Repeated input rows have set meaning |
| [2. Projection and reordered head](../test-support/relational/plans/projection.ra) | `pi_{x,y} R(x,y,z)`, output `(y,x)` | Existential witnesses collapse; heading order does not reorder the answer |
| [3. Two-hop self-join](../test-support/relational/plans/self_chain.ra) | `pi_{a,c}(Road_0(a,b) join Road_1(b,c))` | One base table supports two different occurrence mappings |
| [4. Return trip](../test-support/relational/plans/return_trip.ra) | `pi_a(Road_0(a,b) join Road_1(b,a))` | Equal variable sets do not erase argument direction |
| [5. Intersection](../test-support/relational/plans/intersection.ra) | `R(x,y) join S(x,y)` | All columns are shared; both tuple constraints remain |
| [6. Unary guard](../test-support/relational/plans/unary_guard.ra) | `pi_x(R(x,y) join Approved(y))` | Singleton hyperedge filters without supplying a new variable |
| [7. Chain with early projection](../test-support/relational/plans/chain_projection.ra) | `pi_{a,d}(pi_{a,c}(R(a,b) join S(b,c)) join T(c,d))` | Forget `b` only after its last agreement; eliminate duplicate witnesses |
| [8. Claw](../test-support/relational/plans/claw.ra) | `pi_{a,b,c}((A(h,a) join B(h,b)) join C(h,c))` | Cyclic SIP graph, acyclic occurrence hypergraph |
| [9. Fact/dimension star](../test-support/relational/plans/fact_star.ra) | Ternary `F(x,y,z)` joined with three dimensions, trimming each no-longer-needed join attribute | Different overlaps with one intact ternary constraint |
| [10. Triangle](../test-support/relational/plans/triangle.ra) | `(R(a,b) join S(b,c)) join T(c,a)` | A cyclic body with a composite closing test |
| [11. Four-cycle](../test-support/relational/plans/cycle4.ra) | `pi_{a,c}(R join S) join pi_{a,c}(T join U)` | Bushy cyclic plan; both shared endpoint variables must match |
| [12. Cartesian product](../test-support/relational/plans/cartesian.ra) | `R(x) join S(y)`, output `(y,x)` | Disconnected inputs still combine; no artificial equality |
| [13. Disconnected existential component](../test-support/relational/plans/existential.ra) | `R(x) join pi_{} S(y,z)` | `{()}` and the empty relation have different meanings |
| [14. Ternary guard](../test-support/relational/plans/ternary_guard.ra) | `(A(d,i) join B(i,t) join C(d,t)) join Stock(d,i,t)` | Pairwise witnesses admit a triple that the full tuple test rejects; universal hyperedge makes this body acyclic |
| [15. Composite overlap](../test-support/relational/plans/composite.ra) | `pi_{x,y}(R(a,b,x) join S(b,a,y))` | Both joined columns must respect different source-column orders |
| [16. Bushy four-edge chain](../test-support/relational/plans/bushy.ra) | `pi_{a,e}(pi_{a,c}(R join S) join pi_{c,e}(T join U))` | Both operands are computed/projected results |
| [17. Full delivery reduction DAG](../test-support/relational/plans/ya_delivery.ra) | Two upward filters, two downward filters, then `S,N,D` assembly | Shared intermediates, fresh versions, complete support, and safe forgetting |
| [18. Oriented self-joined chain](../test-support/relational/plans/oriented_chain.ra) | `pi_{a,d}(Edge(a,b) join Edge(b,c) join Edge(d,c))` | Three occurrences of one input, including lookup through the last atom's second source column |
| [19. Spade](../test-support/relational/plans/spade.ra) | `(R(x,y) join S(y,z) join T(x,z)) join H(x,w)` | A cyclic core with an acyclic appendage; graph reduction must not erase the tail from execution |
| [20. Barbell](../test-support/relational/plans/barbell.ra) | Join two independently formed triangles through `Bridge(x,u)` | Seven occurrences, two cyclic regions, and a bridge in a larger bushy reference plan |
| [21. Loomis–Whitney-4](../test-support/relational/plans/loomis_whitney_4.ra) | `ABC(a,b,c) join BCD(b,c,d) join ACD(a,c,d) join ABD(a,b,d)` | A cyclic hypergraph of ternary atoms; one composite extension followed by two full ternary tests |
| [22. Deep Yannakakis schedule](../test-support/relational/plans/ya_deep.ra) | Four-edge chain rooted at an endpoint; three upward and three downward filters | Propagation through depth three; ancestors must use reduced children and descendants must use the latest parents |
| [23. Composite Yannakakis separators](../test-support/relational/plans/ya_composite.ra) | `R(a,b,u), S(a,b,c), T(b,c,v)`, rooted at `S` | Both passes use entire two-column separators `{a,b}` and `{b,c}` |
| [24. Disconnected Yannakakis schedule](../test-support/relational/plans/ya_disconnected.ra) | `R(x), S(y,z)` with an empty-separator tree edge | Both reduction directions propagate emptiness; the head-free component remains an existence obligation |
| [25. Cartesian-first connected chain](../test-support/relational/plans/cartesian_first.ra) | `pi_{a,d}((R(a,b) cross T(c,d)) join S(b,c))` | A legal disconnected first combination inside a connected query |

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

After normalization, the order, reduction and elaboration boundaries use
`check(source, target)`, with options supplied to order and reduction. These
checks establish the target contract and erasure-based correspondence;
kernels compare up to renaming. Selected tests also compare elaborated RA
output with authored answers or independent CQ valuations. Final answer
equality alone does not inspect every intermediate reduction relation.

The graph tests distinguish a chordal but nonconformal triangle from
acyclic chains and stars, and keep repeated relation occurrences distinct.
GYO removes residual analysis edges without removing executable atoms.
The Cartesian-first reference demonstrates that connectivity alone is not
a universal cost argument.

The authored reduction plans spell semijoins using join/project as
reference shapes. Elaborated RA has an explicit Semijoin operator for VO
reductions and partial subatom checks; Physical can realize it as `semi`
(membership on the key).

## Reproduce current checks

```sh
cargo test -p cq-ir --test valuation_language --test hypergraph_analysis --test relational_plan
cargo test -p cq-order
cargo test -p cq-reduce
cargo test -p cq-elaborate
cargo test -p cq-index-policy
cargo test -p mini-linq --test physical_query
```

These commands name the current pass and integration suites. They are
reproduction commands, not a claim that every authored RA shape is produced
by each heuristic.

The [full-chain test](../crates/mini-linq/tests/physical_query.rs) exercises
every stage after IX: hosts read in place through their declared keys,
compiler-built hash indexes, a user-defined index type through
`#[index(Type)]`, merge joins over built ordered maps, Yannakakis semijoin
passes as membership tests, a bounded scan of an ordered host, and a
computed column, each checked against independent relational enumeration.
This is additional physical coverage of supported queries, not compiled
execution of all 25 reference plans.
