# Logical planning verification

These checks stop at RA. They verify denotation, intermediate reduction support,
and derivation rules without compiling the retired whole-query physical routes.
The [planner map](PLANNING.md) describes implementation ownership; Physical
realization has a separate, narrower acceptance domain.

## Active suites and independent oracles

| Suite in `crates/cq-query-lowering/tests` | Current evidence |
|---|---|
| `composition.rs` | Typed planner selection, domain rejection, and derivation replay |
| `planned_semantics.rs` | All 25 patterns under Written/Connected/GreedyWidth/Auto on authored plus 64 generated databases; CQ/RA equality and full YA support when selected |
| `planned_derivations.rs` | Missing/repeated commands, phase dependencies, latest versions, alternative sibling schedules, selection/pruning and arity-bound checks |
| `expression_planning.rs` | Twelve expression shapes under four policies, expected occurrence orders, a handwritten RA normalization, all 120 orders of a five-clause example, and width/replay rejection |
| `rust_expression_contracts.rs` | Independent source/target checks for changed expressions, reordered guards, missing equalities, opaque-macro liveness, and legal re-entry |
| `relational_reference.rs` and `relational_reference/` | Authored known plans, fixed answers, semantic counterexamples, occurrence/heading/arity contracts and graph classifications |

The [25 authored `.ra` files](../test-support/relational/plans),
[case data](../test-support/relational/cases.rs), and
[semantic evaluators](../test-support/relational/semantics.rs) remain checked in.
Expected plans are not produced by the planner under test. The separate
[reference verifier](../crates/cq-query-lowering/tests/relational_reference/verifier.rs)
and [authored contracts](../crates/cq-query-lowering/tests/relational_reference/contracts.rs)
check actual intermediates and structure, not just final outputs.

The oracles use different algorithms:

- CQ evaluation enumerates complete active-domain valuations and checks atoms.
- RA evaluation interprets the DAG with nested-loop natural joins and set
  projection, retaining every intermediate.
- [Join-tree enumeration](../test-support/relational/join_trees.rs) uses Prüfer
  codes and checks running intersection directly, independently of GYO.

The CQ/RA evaluators share integer/set carriers and name normalization, but do
not share the planner's scheduling or index analysis. Fixed authored answers
and deliberately wrong plans help expose shared misunderstandings.
Production GYO tests compare all 4,680 ordered hypergraphs on one to four
occurrences over three variables against independent tree enumeration. The
seven-occurrence barbell reference enumerates 16,807 labeled trees.

The active equality chain is:

```text
fixed authored answers
    = CQ valuation
    = interpretation of the known RA plan
    = interpretation of the planner-produced RA plan
```

This is logical validation. The removed compiled-backend/public-query matrices
and their runtime effect counters are not current evidence. No existing golden
RA fixture alone proves that a new physical policy can execute it.

## Twenty-five known query patterns

Occurrence subscripts below denote separate complete positional renames.
The linked files spell out the actual typed syntax. `pi` denotes set
projection; parentheses fix the reference combination tree. `YA` expands
semijoins into join/project for interpretation only.

| Pattern / full known RA | Reference shape | Distinguishing assertion |
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
| [20. Barbell](../test-support/relational/plans/barbell.ra) | Join two independently formed triangles through `Bridge(x,u)` | Seven occurrences, two cyclic regions, and a bridge; validates a larger bushy reference plan |
| [21. Loomis–Whitney-4](../test-support/relational/plans/loomis_whitney_4.ra) | `ABC(a,b,c) join BCD(b,c,d) join ACD(a,c,d) join ABD(a,b,d)` | A cyclic hypergraph of ternary atoms; one composite extension followed by two full ternary tests |
| [22. Deep Yannakakis schedule](../test-support/relational/plans/ya_deep.ra) | Four-edge chain rooted at an endpoint; three upward and three downward filters | Propagation through depth three; ancestors must use reduced children and descendants must use the latest parents |
| [23. Composite Yannakakis separators](../test-support/relational/plans/ya_composite.ra) | `R(a,b,u), S(a,b,c), T(b,c,v)`, rooted at `S` | Both passes use entire two-column separators `{a,b}` and `{b,c}` |
| [24. Disconnected Yannakakis schedule](../test-support/relational/plans/ya_disconnected.ra) | `R(x), S(y,z)` with an empty-separator tree edge | Both reduction directions propagate emptiness; the head-free component remains an existence obligation |
| [25. Cartesian-first connected chain](../test-support/relational/plans/cartesian_first.ra) | `pi_{a,d}((R(a,b) cross T(c,d)) join S(b,c))` | A legal disconnected first combination inside a connected query; its row-volume proxy is lower on the authored input |

The triangle, four-cycle, spade, barbell, and Loomis–Whitney-4 are alpha-cyclic;
the other 20 bodies are alpha-acyclic. The test suite
checks those authored classifications by independent tree enumeration.
An additional test repeats an identical occurrence and retains both nodes.
The small graph oracle cannot replace broader production GYO testing.

All ten families in the [classic workload catalog](../workloads/src/catalog/classic.rs)
have counterparts in this reference suite.
This is family coverage, not 25 distinct topologies or exhaustive coverage of
all possible join graphs. Intersection uses an all-columns-shared variant here;
some fixtures also choose a different head projection from the catalog.

## Stronger checks than final answers

The reference suite also checks duplicate occurrences, complete composite
agreement, empty-heading existence, and all four authored YA reduction
schedules. Frozen occurrence versions are compared with their projections from
the complete query valuation on authored/generated databases.

Eight deliberately wrong plans cover reversed self-join positions, a dropped
joined column, premature projection, erased disconnected existence, replacing
a ternary constraint with pairwise witnesses, and stale reduction versions.
All first pass local RA validity. Some stale versions retain correct final
answers but fail full intermediate support, so result equality cannot replace
the reduction invariant.

The derivation verifier checks exact remaining-use sets across bushy cuts,
complete upward/downward schedules, running intersection, eager pruning and
bounds on every named heading. It allows legal independent sibling reorderings.
The canonical chain and a pruned plan can have equal answers but different
peak arity and policy compliance.

The graph tests distinguish alpha-acyclicity from stronger/head-sensitive
properties: removing the full ternary atom from `ternary_guard` exposes a cyclic
triangle; the endpoint-only self-chain is body-acyclic but not free-connex.
These checks do not authorize removing an atom from the executable query or
claim output-linear enumeration. The Cartesian-first reference demonstrates
that connectivity alone is not a universal cost argument.

The authored reduction plans spell semijoins using join/project so their
construction can be interpreted independently. Production RA also has an
explicit Semijoin operator, which Physical realizes as `semi` (membership on
the key).

## Reproduce current checks

```sh
cargo test -p cq-query-lowering --all-features
cargo test -p cq-query-lowering --all-features --release
cargo test -p cq-index-policy
cargo test -p mini-linq --test physical_query
cargo test --workspace
```

`cq-query-lowering --all-features` selects its three implemented planner families;
it does not enable the separate unfinished `cq-ir/hw4` exercise. Workspace
`--all-features` is not the ordinary completion gate.

The [full-chain test](../crates/mini-linq/tests/physical_query.rs) exercises
every stage after IX: hosts read in place through their declared keys,
compiler-built hash indexes, a user-defined index type through
`#[index(Type)]`, merge joins over built ordered maps, Yannakakis semijoin
passes as membership tests, a bounded scan of an ordered host, and a
computed column, each checked against independent relational enumeration.
This is additional physical coverage of supported queries, not compiled
execution of all 25 reference plans.
