# Stage I verification: known RA, independent semantics, and compiled execution

This guide specifies verification for the planned CQ-to-RA route and its
indexed materialized backend. [PLANNED-PASS-IMPLEMENTATION.md](PLANNED-PASS-IMPLEMENTATION.md)
maps the current implementation; [planning.tex](formal/planning.tex) gives the
rules and algorithms. The direct configurable physical edge has separate
[Access-refinement coverage](ACCESS-REFINEMENT.md).

## Implemented production checks

| Capability | Implementation and evidence |
|---|---|
| Occurrence and graph analysis | Separate occurrence IDs and positional mappings; GYO with reconstruction; running intersection; all 4,680 ordered hypergraphs with 1–4 occurrences over three variables compared against tree enumeration |
| Join planning | Written, Connected, GreedyWidth, Auto; Auto reduces alpha-acyclic bodies and uses a binary fallback for cyclic bodies |
| Selection | Six primitive-i32 comparisons; local and cross-occurrence predicates; constant predicates; extreme literals; aliases; Rust rejects incompatible operand types |
| Filter pushdown | Apply ready predicates before combination/pruning; retain variables needed by pending predicates |
| Pruning and arity | Exact live headings at consumption boundaries; bounds on all named results; independently inspected actual Rust tuple storage |
| YA reduction | Explicit semijoins, two passes, latest references, freeze; independent sibling order accepted; each frozen positive occurrence equals its full-join projection |
| Result indexes | Exact first-use composite keys on named RA versions; missing/wrong/duplicate/out-of-range keys rejected; separate base/result namespaces |
| Physical lowering | RA -> IndexRequirements -> staged Rust with one local set-valued Access region per result -> existing IteratorPipeline -> Rust |
| Quotation | Complete artifacts at all six existing boundaries compile and execute independently |
| Ownership | Non-Copy String values, non-Ord hash keys, aliases, duplicate input normalization, shared RA operands, and unused non-Clone inputs |

[Production acceptance tests](../crates/cq-compiler/tests/stage1_production.rs)
compile 25 patterns under four policies plus the 25 authored reference plans.
Each executable runs on its authored database and 64 generated databases:
6,500 planned executions and 1,625 authored-plan executions. Expected outputs
come from independent valuation, with the authored database also checked
against fixed expected rows. The logical interpreter inspects intermediates.

That suite exercises the **typed compiler API**: a test visitor invokes the
existing pull pass and set-valued Pipeline endpoint before standalone rustc compilation.
It does not exercise procedural-macro routing for all 25 shapes.

The separate [public-macro end-to-end suite](../crates/mini-linq/tests/stage1_shapes.rs)
closes that gap. Every shape is literal CQ passed to `planned_query!`; Cargo
expands the entire public chain and compiles the generated program. Test code
calls no compiler passes and performs no AST lowering. Each shape has its own
named test, and a coverage assertion compares the complete set of names and
the exact CQ programs against the independent reference fixtures.

For each shape this suite checks the authored input, 64 binary-domain inputs,
32 inputs over `{-2, 0, 3}`, and each used input emptied individually. Every
scenario also runs with reversed, duplicated host rows: **4,990 shape/input
presentations per profile**. Each presentation checks `build`/`materialize`,
two independent traversals of `query`, and `run`. Expected rows come from CQ
valuation and fixed authored answers. Actual outputs retain multiplicities
during comparison, so duplicate answers cannot disappear in the assertion.
The default public macro selects Auto; all four policies are covered through
the typed API. Continuations are configurable through ordinary Rust macro
aliases and the materialized Access option. The [endpoint tests](../crates/mini-linq/tests/collect_endpoints.rs)
check alias selection separately from these query-shape tests.

A separate edge checker reads actual RA definitions; it never calls the
planner's emission code to reconstruct expected output. Mutation tests omit
and repeat every administrative step, change dependencies and references,
remove selections/pruning, and forge arity metadata. Result tests alone cannot
satisfy these contracts. Local RA validity and cross-language preservation
remain different checks.

Logical width here is column count. It is not tuple bytes, cardinality,
treewidth, or fractional hypertree width. The materialized Rust backend
currently supports at most twelve attributes per named result, matching
standard tuple Eq/Hash support. Predicates use primitive i32 comparisons;
logical contracts do not compare Rust type spellings. For cross-occurrence
predicates applied after reduction, full support is claimed only for the
locally selected positive family. WCOJ, cardinality estimation, general Rust
purity analysis, and projected-output-linear complexity are not claimed.

## The independent comparisons

For each fixture, keep four separately inspectable artifacts:

1. the authored CQ;
2. an authored known RA plan, including every rename and intermediate;
3. concrete input tables; and
4. hand-written expected output tuples in the source head's order.

The [fixtures](../crates/cq-compiler/tests/stage1_reference/cases.rs) contain
input/output values; each linked `.ra` file contains a complete CQ program
followed by its known plan. Expected RA is never generated by `compile_cq` or
the planner under test. A known plan is a valid reference construction,
not a claim that its shape is the only correct or cheapest plan.

The test-only [semantic oracles](../crates/cq-compiler/tests/stage1_reference/oracle.rs)
follow different algorithms:

- **CQ oracle:** enumerate complete valuations over the active domain and
  check each ordered atom tuple directly against its base relation.
- **RA oracle:** interpret the parsed DAG with nested-loop natural joins and
  set projection; retain every named intermediate for inspection.
- **Graph oracle:** enumerate labeled trees through Prüfer codes for up to
  seven occurrences and check running intersection directly. It does not use
  GYO reduction; the seven-occurrence barbell checks all 16,807 candidate trees.

The CQ and RA evaluators share integer/set representations and symbol-name
normalization, but no join-planning, key-derivation, or heading-inference
implementation. Fixed expected answers and deliberately incorrect plans
provide checks against a shared misunderstanding in the two evaluators.

Each of the 25 named reference tests also calls the test-only
[expanded-RA verifier](../crates/cq-compiler/tests/stage1_reference/verifier.rs). The authored
occurrence assignments, optional tree parents, and peak arities are in
[`contracts.rs`](../crates/cq-compiler/tests/stage1_reference/contracts.rs).
Every reported heading is compared with the independent interpreter's actual
intermediate heading. This makes structural and policy validation mandatory
alongside reference denotation checks; equivalent answers alone cannot pass.
The generated-data test checks the same reference contracts before evaluating
its inputs. R1 remains explicitly a semantic baseline, not an eager planner.

The production suite enforces:

```text
hand-written expected rows             // fixed examples
               = CQ valuation oracle
               = interpretation of known RA
               = interpretation of planner-produced RA
               = execution of Rust compiled from known RA
               = execution of Rust compiled from planner-produced RA
```

Compiling known RA independently isolates backend bugs from planning bugs.
Compile the result as actual Rust and run it; token inspection cannot replace
execution. Compare sets of ordered output tuples, including output arity.

## Twenty-five known query patterns

Occurrence subscripts below denote separate complete positional renames.
The linked files spell out the actual typed syntax. `pi` denotes set
projection; parentheses fix the reference combination tree. `YA` expands
semijoins into join/project for interpretation only.

| Pattern / full known RA | Reference shape | Distinguishing assertion |
|---|---|---|
| [1. Scan](../crates/cq-compiler/tests/stage1_reference/plans/scan.ra) | `R(x)` | Repeated input rows have set meaning |
| [2. Projection and reordered head](../crates/cq-compiler/tests/stage1_reference/plans/projection.ra) | `pi_{x,y} R(x,y,z)`, output `(y,x)` | Existential witnesses collapse; heading order does not reorder the answer |
| [3. Two-hop self-join](../crates/cq-compiler/tests/stage1_reference/plans/self_chain.ra) | `pi_{a,c}(Road_0(a,b) join Road_1(b,c))` | One base table supports two different occurrence mappings |
| [4. Return trip](../crates/cq-compiler/tests/stage1_reference/plans/return_trip.ra) | `pi_a(Road_0(a,b) join Road_1(b,a))` | Equal variable sets do not erase argument direction |
| [5. Intersection](../crates/cq-compiler/tests/stage1_reference/plans/intersection.ra) | `R(x,y) join S(x,y)` | All columns are shared; both tuple constraints remain |
| [6. Unary guard](../crates/cq-compiler/tests/stage1_reference/plans/unary_guard.ra) | `pi_x(R(x,y) join Approved(y))` | Singleton hyperedge filters without supplying a new variable |
| [7. Chain with early projection](../crates/cq-compiler/tests/stage1_reference/plans/chain_projection.ra) | `pi_{a,d}(pi_{a,c}(R(a,b) join S(b,c)) join T(c,d))` | Forget `b` only after its last agreement; eliminate duplicate witnesses |
| [8. Claw](../crates/cq-compiler/tests/stage1_reference/plans/claw.ra) | `pi_{a,b,c}((A(h,a) join B(h,b)) join C(h,c))` | Cyclic SIP graph, acyclic occurrence hypergraph |
| [9. Fact/dimension star](../crates/cq-compiler/tests/stage1_reference/plans/fact_star.ra) | Ternary `F(x,y,z)` joined with three dimensions, trimming each spent key | Different overlaps with one intact ternary constraint |
| [10. Triangle](../crates/cq-compiler/tests/stage1_reference/plans/triangle.ra) | `(R(a,b) join S(b,c)) join T(c,a)` | A cyclic body with a composite closing test |
| [11. Four-cycle](../crates/cq-compiler/tests/stage1_reference/plans/cycle4.ra) | `pi_{a,c}(R join S) join pi_{a,c}(T join U)` | Bushy cyclic plan; both shared endpoint variables must match |
| [12. Cartesian product](../crates/cq-compiler/tests/stage1_reference/plans/cartesian.ra) | `R(x) join S(y)`, output `(y,x)` | Disconnected inputs still combine; no artificial equality |
| [13. Disconnected existential component](../crates/cq-compiler/tests/stage1_reference/plans/existential.ra) | `R(x) join pi_{} S(y,z)` | `{()}` and the empty relation have different meanings |
| [14. Ternary guard](../crates/cq-compiler/tests/stage1_reference/plans/ternary_guard.ra) | `(A(d,i) join B(i,t) join C(d,t)) join Stock(d,i,t)` | Pairwise witnesses admit a triple that the full tuple test rejects; universal hyperedge makes this body acyclic |
| [15. Composite overlap](../crates/cq-compiler/tests/stage1_reference/plans/composite.ra) | `pi_{x,y}(R(a,b,x) join S(b,a,y))` | A two-column key must respect different source-column orders |
| [16. Bushy four-edge chain](../crates/cq-compiler/tests/stage1_reference/plans/bushy.ra) | `pi_{a,e}(pi_{a,c}(R join S) join pi_{c,e}(T join U))` | Both operands are computed/projected results |
| [17. Full delivery reduction DAG](../crates/cq-compiler/tests/stage1_reference/plans/ya_delivery.ra) | Two upward filters, two downward filters, then `S,N,D` assembly | Shared intermediates, fresh versions, complete support, and safe forgetting |
| [18. Oriented self-joined chain](../crates/cq-compiler/tests/stage1_reference/plans/oriented_chain.ra) | `pi_{a,d}(Edge(a,b) join Edge(b,c) join Edge(d,c))` | Three occurrences of one input, including lookup through the last atom's second source column |
| [19. Spade](../crates/cq-compiler/tests/stage1_reference/plans/spade.ra) | `(R(x,y) join S(y,z) join T(x,z)) join H(x,w)` | A cyclic core with an acyclic appendage; graph reduction must not erase the tail from execution |
| [20. Barbell](../crates/cq-compiler/tests/stage1_reference/plans/barbell.ra) | Join two independently formed triangles through `Bridge(x,u)` | Seven occurrences, two cyclic regions, and a bridge; validates a larger bushy reference plan |
| [21. Loomis–Whitney-4](../crates/cq-compiler/tests/stage1_reference/plans/loomis_whitney_4.ra) | `ABC(a,b,c) join BCD(b,c,d) join ACD(a,c,d) join ABD(a,b,d)` | A cyclic hypergraph of ternary atoms; one composite extension followed by two full ternary tests |
| [22. Deep Yannakakis schedule](../crates/cq-compiler/tests/stage1_reference/plans/ya_deep.ra) | Four-edge chain rooted at an endpoint; three upward and three downward filters | Propagation through depth three; ancestors must use reduced children and descendants must use the latest parents |
| [23. Composite Yannakakis separators](../crates/cq-compiler/tests/stage1_reference/plans/ya_composite.ra) | `R(a,b,u), S(a,b,c), T(b,c,v)`, rooted at `S` | Both passes use entire two-column separators `{a,b}` and `{b,c}` |
| [24. Disconnected Yannakakis schedule](../crates/cq-compiler/tests/stage1_reference/plans/ya_disconnected.ra) | `R(x), S(y,z)` with an empty-separator tree edge | Both reduction directions propagate emptiness; the head-free component remains an existence obligation |
| [25. Cartesian-first connected chain](../crates/cq-compiler/tests/stage1_reference/plans/cartesian_first.ra) | `pi_{a,d}((R(a,b) cross T(c,d)) join S(b,c))` | A legal disconnected first combination inside a connected query; its row-volume proxy is lower on the authored input |

The triangle, four-cycle, spade, barbell, and Loomis–Whitney-4 are alpha-cyclic;
the other 20 bodies are alpha-acyclic. The test suite
checks those authored classifications by independent tree enumeration.
An additional test repeats an identical occurrence and retains both nodes.
The small graph oracle cannot replace broader production GYO testing.

All ten families in the [classic workload catalog](../workloads/src/catalog/classic.rs)
have counterparts in this reference suite.
This is family coverage, not 25 distinct topologies or exhaustive coverage of
all possible join graphs. Intersection uses a binary full-key variant here;
some fixtures also choose a different head projection from the catalog.

## Which acyclicity is checked

The production design uses **alpha-acyclicity** as the condition for a
join-tree/Yannakakis route. It also accepts alpha-cyclic bodies through the
ordinary binary planner. The fixture set intentionally covers both outcomes.

There are stronger notions. For the underlying hypergraph with distinct
hyperedges, the strict hierarchy is
`Berge-acyclic ⇒ gamma-acyclic ⇒ beta-acyclic ⇒ alpha-acyclic`.
Beta-acyclicity requires every subset of hyperedges to remain alpha-acyclic.
The Stage I design does not need those stronger recognizers to use Yannakakis.
See [Fagin, Sections 4–6](https://s3.us.cloud-object-storage.appdomain.cloud/res-files/500-jacm83b.pdf).
Occurrence IDs and argument maps remain distinct in executable plans regardless
of equal sets in the underlying hypergraph.

A regression checks that `ternary_guard` is alpha-acyclic but removing
`Stock(d,i,t)` leaves the cyclic triangle of pair constraints. Thus it is not
beta-acyclic. This is why a cyclic-looking partial body is insufficient to
classify the complete query. Removing an atom in this test investigates the
structural property; it is not an authorized semantic query rewrite.

**Free-connex acyclicity** is a separate, head-sensitive refinement: the body
is alpha-acyclic and remains so after adding a hyperedge containing all head
variables. Our two-hop query returning only endpoints fails this second check;
the single-atom projection passes. The suite checks both examples. This
distinction matters when specifying enumeration guarantees for projected
answers; body acyclicity alone does not justify those stronger performance
claims. See [Bagan, Durand, and Grandjean, Section 4](https://webusers.imj-prg.fr/~arnaud.durand/papers/BDGcsl07.pdf).

## Regression coverage

The [reference suite](../crates/cq-compiler/tests/stage1_reference.rs) covers:

- all 25 named patterns, checking local RA validity, parse/print round trips,
  CQ valuation, RA interpretation, and independently written answers;
- 64 deterministic binary-domain databases per pattern, including all-empty
  and all-full inputs, plus each used relation emptied individually;
- each frozen occurrence in the four Yannakakis schedules against its
  full-query projection, on authored and generated databases;
- whole composite keys, empty headings, duplicate-eliminating projection,
  occurrence identity, cyclic support counterexamples, and tree recognition;
- safety and eager-pruning contracts, arity limits, source/occurrence mappings,
  complete assembly, current reduction versions, schema preservation,
  upward/downward completion, and acceptance of noncanonical legal schedules;
- independent acyclicity classifications, alpha-versus-beta and
  body-versus-free-connex counterexamples, and the Cartesian-first comparison.

The test-only verifier also checks exact remaining-use sets across bushy cuts,
a branched tree's complete two-pass schedule, and rejection of a connected tree
that violates running intersection. Production graph and rule-kernel tests run
under `--lib`.

The contract checks use ordinary `Result` validation in debug and release.
The basic R1 chain has correct answers but retains a dead variable and reaches
arity four; the authored pruned plan has peak arity three. Its eager-pruning
rejection distinguishes policy compliance from result correctness. Safety,
eager pruning, and arity limits have separate rejection cases, so relaxing one
obligation does not silently establish the others.

The eight incorrect plans cover reversed self-join arguments, a dropped key
component, premature projection, erased disconnected existence, replacement
of a ternary constraint by pairwise witnesses, a stale top-down parent,
a stale child in the deep upward pass, and a stale ancestor in the deep
downward pass.
Each wrong RA plan first passes the existing local RA contract. Five produce
wrong answers; the three stale-version cases deliberately retain correct
final answers and fail the stronger intermediate full-support check.

The four reference reduction plans use semijoin's join/project definition because
they are independently authored expansion references. Production RA has
explicit Semijoin and a separate existence-probe implementation.

The reference interpreter uses i32 sets. The macro and production integration
tests additionally exercise Rust ownership and duplicate host inputs.

## Verification commands

Run the semantic, contract, compiled-backend and macro gates in both profiles:

```sh
cargo test -p cq-compiler --lib --test stage1_reference --test stage1_production --test pass_contracts
cargo test -p cq-compiler --release --lib --test stage1_reference --test stage1_production --test pass_contracts
cargo test -p mini-linq --test stage1_shapes --test collect_endpoints
cargo test -p mini-linq --release --test stage1_shapes --test collect_endpoints
```

The public-macro tests are ordinary Cargo integration tests; release builds
optimize the generated query programs alongside the test binary. The typed
production suite also checks predicate lowering, DAG sharing, semijoin work,
extra requested indexes, and input/result names with distinct index namespaces.
These coverage descriptions specify what the suites exercise, not a stored
record of the latest run or a workspace-wide test count.

The wider integration gates are:

```sh
cargo test --workspace --features mini-linq/compiled-workloads
cargo test --workspace --release --features mini-linq/compiled-workloads
cargo clippy --workspace --all-targets --features mini-linq/compiled-workloads -- -D warnings
cargo test --manifest-path xtask/Cargo.toml
cargo fmt --all --check
git diff --check
```

The optional HW4 tests are not enabled by these commands. R1 regressions check
the basic canonical plan, index and lazy-pull contracts independently of the
planned route.

Reproduce the generated boundary artifacts with:

```sh
cargo run -p cq-compiler --example planned_boundaries -- doc/planned-example
```

The production fixture test compares all six checked-in artifacts with current
pass output. The macro test separately compiles each artifact, while the
standalone rustc harness executes the fully lowered program.
