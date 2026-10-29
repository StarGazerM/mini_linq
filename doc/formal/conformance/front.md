# Rule conformance: front

Scope: CQ, VO, RA, IX, Physical, Structures; cq-order, cq-reduce, cq-elaborate, index policy, physical lowering/realization, structure selection, facade entry.

Produced 2026-10-28 by a read-only audit pairing each `\Rule` in `doc/formal/*.tex`
(and its `doc/formal/rules.tsv` row) with the code anchor that implements it.
Status legend: `exact` (code checks the written premises), `weaker` (checks
less), `stronger` (checks more), `unimplemented` (abstract or no checker),
`no-rule` (a code decision with no governing rule; the invariant column states
the rule that is missing). "Exact" is structural conformance under the rule's
stated runtime premises, not a proof. Items marked [INFERENCE] were not
executed. `doc/formal/check.py` verifies that every rule named here exists in
`rules.tsv` and every cited file exists.

## Rule conformance

**Scope.** Front/middle-end rules plus lexical dependencies, representations, staging and context. Contract-7 rows replace the retired recognition model; `unverified` states an updated obligation without claiming verification.

**Interpretation.** `exact` means the inspected implementation matches the structural rule **under its documented semantic/runtime premises**; it does not mean those premises have been proved or execution tested. An abstract law without an executable checker is identified explicitly. “—” denotes a code decision, not an invented formal rule name.

### Non-exact rules and decisions without a rule

| Rule | TeX file:line | Code anchor opened | Premises as written | What the code checks/does | Status | INVARIANT |
|---|---|---|---|---|---|---|
| STR-Reader | `doc/formal/ir_structures.tex` | `languages/cq-lang-structures/src/lib.rs` (`ReaderRef`, `analyze`); `passes/cq-print/src/access.rs` (`fallback_type`) | Tagged roles and named results are disjoint; each fallback matches its exact read. | Parses/prints `host scan`, `host candidates`, `host membership`, or `host result(name)` and carries the tag through analysis and consumers. | **exact** (was no-rule) | Reader identity never changes with result spelling. |
| STR-Group; SEL-Group; P-GroupState | `doc/formal/ir_structures.tex` | `languages/cq-lang-structures/src/lib.rs` (`check_group`); `passes/cq-structure-selection/src/lib.rs` (`group_accumulator`); `passes/cq-physical-realization/src/operators.rs` (`group_with_accumulator`) | Every keyed Group has exactly one concrete accumulator entry; providers satisfy the finite-map protocol. | Rejects missing/duplicate/misplaced entries; selector records HashMap with `cq_storage::HashState` and realization consumes the recorded type, preserving authored collection hashers. | **exact**, conditional on provider laws (was no-rule) | Concrete accumulator choices, including the default hasher, are printable IR decisions. |
| WF-BlockPlacement; O-BlockPlacement; O-WrittenBlocks; E-BlockPlacement; CK-BlockPlacement | `doc/formal/ir_vo.tex`; `doc/formal/passes.tex` | `languages/cq-lang-vo/src/blocks.rs` (`Placement`); `languages/cq-lang-vo/src/lib.rs` (`WF-BlockPlacement`); `passes/cq-order/src/lib.rs` (`compile_written`); `passes/cq-elaborate/src/block_contract.rs` (`check_placement`) | VO records and checks Hoisted versus Correlated; ordering chooses explicitly; elaboration preserves the recorded choice. | Required parsed/printed enum; Rust-free hoisting, bound correlated free variables, repeated-read agreement, reduction exclusion, and exact RA correlation provenance are checked. | **exact** (was no-rule) | Placement is printable VO evidence, never an elaborator mode. |
| —: implicit planner insertion | `doc/formal/composition.tex:6-8`; `doc/DESIGN.md:138-140` | `facade/mini-linq-macros/src/lib.rs:86-108` | `π = resolve(configuration, defaults)`; omitted lists empty. | Searches the resolved queue for an exact pass path and conditionally inserts `plan_sequential`. | **no-rule** | Pipeline execution follows explicit resolution. |
| CQ-Atom | `doc/formal/ir_cq.tex:68-84` | `languages/cq-lang-cq/src/lib.rs:596-674` (`CQ-Atom`) | Declared relation, matching arity; computed arguments see incoming scope plus simultaneous bare tuple binders. | Derives bare binders once, rejects escaped branch locals, checks computed arguments under that scope. The rule now states the intentional A-Local semantics; later-clause variables remain forbidden. | **exact** (was weaker than stale left-to-right argument wording) | Column permutation does not alter atom scope; body clauses remain left-to-right. |
| WF-VO; WF-VO-Rust | `doc/formal/ir_vo.tex` (`WF-VO-Rust`) | `languages/cq-lang-vo/src/lib.rs` (`WF-VO-Rust`) | Every unchanged Rust expression appears once with its free variables bound. | Placement uses free-variable dependencies only. | **unverified** | Rust has one pure-total meaning. |
| STR-Lineage | `doc/formal/ir_structures.tex:48-71` | `languages/cq-lang-structures/src/positional.rs:84-130` (`STR-Lineage`) | Single-occurrence spine and injective heading-to-host mapping; eager semi propagates its left lineage independently of support. | Propagates the left lineage without requiring RHS lineage; joined/computed supports remain admissible. | **exact** (was stronger) | Position lineage preserves one base occurrence. |
| ST-Entry | `doc/formal/composition.tex:6-8` | `facade/mini-linq-macros/src/lib.rs:81-110` | `π = resolve(configuration, defaults)` and `π ≠ []`. | Resolution is followed by configuration-sensitive queue mutation. | **weaker** than the stated queue-equality premise | Entry starts the resolved checked queue. |
| O-Leaf | `doc/formal/passes.tex:457` | Inventory: `doc/formal/rules.tsv:91`; related implementation `passes/cq-elaborate/src/lib.rs:812-843` | `i ∈ O`. | No stored ghost join-tree leaf witness; elaborator directly assembles RA. | **unimplemented**, intentionally abstract | Join-tree leaves cover fixed occurrences. |
| O-Join | `doc/formal/passes.tex:459-460` | Inventory: `doc/formal/rules.tsv:92`; related implementation `passes/cq-elaborate/src/lib.rs:836-843` | `T₁:C`, `T₂:E`, `C ∩ E = ∅`. | No stored ghost binary-tree coverage witness. | **unimplemented**, intentionally abstract | Join-tree composition preserves disjoint coverage. |
| L-Retention | `doc/formal/passes.tex:462-463` | Inventory: `doc/formal/rules.tsv:93`; projection construction `passes/cq-elaborate/src/lib.rs:827-835` | `L:V`, `M:W`, `V ∩ (W ∪ H ∪ fv(P)) ⊆ U ⊆ V`. | Used as a semantic justification; there is no general retention-law/subtree-pruning checker. The following doc paragraph explicitly limits the elaborator’s search. | **unimplemented**, intentionally abstract | Projection retains every observable/joined column. |

### CQ, normalization, and shared syntax

| Rule | TeX file:line | Code anchor opened | Premises as written | What the code checks/does | Status | INVARIANT |
|---|---|---|---|---|---|---|
| FV-Name | `doc/formal/ir_cq.tex:33` | `languages/cq-syntax/src/rust.rs:119-131` | `x ∉ A`. | Records a simple path not in the bound-name set. | exact | Free simple names are retained. |
| FV-Bound | `doc/formal/ir_cq.tex:35` | `languages/cq-syntax/src/rust.rs:119-131` | `x ∈ A`. | Does not record bound simple paths. | exact | Local binders do not become query dependencies. |
| FV-Children | `doc/formal/ir_cq.tex:37-38` | `languages/cq-syntax/src/rust.rs:215-218` | Each child has a free-variable/opacity result. | Visits children and accumulates names/opacity. | exact | Dependency analysis is a conservative union. |
| FV-Closure | `doc/formal/ir_cq.tex:40-41` | `languages/cq-syntax/src/rust.rs:154-160` | Body analyzed under `A ∪ binds(patterns)`. | Opens a scope and binds closure inputs before visiting it. | exact | Closure parameters shadow outer names. |
| FV-For | `doc/formal/ir_cq.tex:43-44` | `languages/cq-syntax/src/rust.rs:163-170` | RHS under old scope; body under pattern-extended scope. | Visits iterator expression first, then binds the item pattern in a nested scope. | exact | Generator RHS cannot see its own item binder. |
| FV-CondLet | `doc/formal/ir_cq.tex:46-47` | `languages/cq-syntax/src/rust.rs:101-108` | Analyze RHS, then extend the conditional scope. | Visits RHS before binding the pattern. | exact | Conditional bindings have lexical scope. |
| FV-And | `doc/formal/ir_cq.tex:49-50` | `languages/cq-syntax/src/rust.rs:109-113` | Right condition sees bindings from the left. | Processes conjunction conditions left-to-right. | exact | Let-chain scope follows short-circuit order. |
| FV-Cond | `doc/formal/ir_cq.tex:52-53` | `languages/cq-syntax/src/rust.rs:114` | Condition is neither special case. | Ordinary expression visitation. | exact | Ordinary conditions introduce no binders. |
| CQ-Var | `doc/formal/ir_cq.tex:71` | `languages/cq-lang-cq/src/lib.rs:661-671` | No additional premise. | Adds atom bare variables to the local scope. | exact | Bare atom arguments bind variables. |
| CQ-Expr | `doc/formal/ir_cq.tex:79` | `languages/cq-lang-cq/src/lib.rs:602-616,668-671` | `e ✓ B`. | Checks known logical uses against the supplied scope, including the simultaneous tuple-binder scope for an atom. | exact | Expression uses are scope-checked. |
| CQ-Guard | `doc/formal/ir_cq.tex` (`CQ-Guard`) | `languages/cq-lang-cq/src/lib.rs` | Expression uses are in scope. | Retains complete Rust, without recognition or splitting. | **unverified** | Rust syntax never becomes a relational constraint. |
| CQ-Bind | `doc/formal/ir_cq.tex:80-81` | `languages/cq-lang-cq/src/lib.rs:767-786` | RHS in scope; pattern names fresh against `B`. | Checks RHS uses and rejects shadowing/rebinding escaped branch locals. | exact | Scalar bindings are fresh and scoped. |
| CQ-For | `doc/formal/ir_cq.tex:83-84` | `languages/cq-lang-cq/src/lib.rs:790-808` | RHS in scope; item names fresh against `B`. | Same freshness/scope obligations as scalar binds; iterator/item typing left to Rust. | exact | Generators extend the incoming scope safely. |
| CQ-Block | `doc/formal/ir_cq.tex:86-88` | `languages/cq-lang-cq/src/lib.rs:536-538,621-649` | At least two branches; branch scopes valid; common new interface; agreeing types. | Recursively checks branches, intersects visible bindings, checks block types through `blocks::types`. | exact | Branch-local names do not escape. |
| ORD-Admissible | `doc/formal/ir_cq.tex` (`ORD-Admissible`) | Abstract representation law | Order is total and identity-consistent. | No type-name checker; rustc enforces bounds. | **unimplemented**, intentionally abstract | Representation laws are premises. |
| CQ-Comparison | `doc/formal/ir_cq.tex` (`CQ-Comparison`) | `languages/cq-lang-cq/src/lib.rs` (`CQ-Where`) | Bound variables and closed host operands. | Checks relational grammar and scope, never Rust types. | **unverified** | Relational comparisons have their own denotation. |
| CQ-Closed | `doc/formal/ir_cq.tex` (`CQ-Closed`) | `languages/cq-lang-cq/src/lib.rs` (`CQ-Closed`); `languages/cq-lang-cq/src/normal.rs` (`A-Closed`) | No query free variables, including branch binders. | Conservative lexical dependency analysis; closed atom expressions become occurrence restrictions, not row bindings. | **exact, conditional** | Arbitrary closed expressions remain unchanged once-bound host operands. |
| CQ-Where | `doc/formal/ir_cq.tex` (`CQ-Where`) | `languages/cq-lang-cq/src/lib.rs` (`CQ-Where`) | Explicit relational syntax with bound operands. | Parses Constraint and prints `where`. | **unverified** | Only explicit predicates drive predicate-derived planning facts. |
| CQ-Neg | `doc/formal/guards.tex` (`CQ-Neg`) | `languages/cq-lang-cq/src/lib.rs` | Declared input, arity, bound variables, no bindings. | Checks scope; representation laws remain premises. | **unverified** | Negation tests but never binds. |
| WF-CQ | `doc/formal/ir_cq.tex` (`WF-CQ`) | `languages/cq-lang-cq/src/lib.rs` | CQScope. | Checks declarations, scopes and relational syntax; rustc owns typing. | **unverified** | No guard-recognition obligation. |
| WF-NF | `doc/formal/ir_cq.tex` (`WF-NF`) | `languages/cq-lang-cq/src/normal.rs` | Distinct variable/literal atom arguments and variable head. | Preserves original expression payloads. | **unverified** | Types are not semantic classifiers. |
| A-Variable | `doc/formal/passes.tex:18-19` | `languages/cq-lang-cq/src/normal.rs:216-228` | Bare variable not previously seen within this atom. | Retains its first occurrence. | exact | First positional occurrence is unchanged. |
| A-Repeat | `doc/formal/passes.tex:21-22` | `languages/cq-lang-cq/src/normal.rs:220-227` | Repeated variable; fresh replacement. | Creates a fresh atom variable and a positioned equality. | exact | Repeated arguments retain equality semantics. |
| A-Wildcard | `doc/formal/passes.tex:24-25` | `languages/cq-lang-cq/src/normal.rs:235-238` | Wildcard; fresh replacement. | Creates an unconstrained fresh witness. | exact | Anonymous witnesses do not become output dependencies. |
| A-Closed | `doc/formal/passes.tex` (`A-Closed`) | `languages/cq-lang-cq/src/normal.rs` (`A-Closed`); `languages/cq-lang-cq/tests/normal.rs` (`closed_atom_operands_are_relational_restrictions_not_rust_bindings`) | Atom expression has no query free variables; fresh column. | Keeps the expression in a closed equality constraint after the atom, including union branches. | exact | Closed restrictions introduce no row Bind or Rust block dependency. |
| A-Prefix | `doc/formal/passes.tex` (`A-Prefix`) | `languages/cq-lang-cq/src/normal.rs` (`A-Prefix`) | Row-dependent expression; no dependencies on newly supplied atom names; fresh variable. | Emits a typed binding before the atom. | exact | Previously bound row arguments execute before the atom; query-closed arguments follow A-Closed. |
| A-Local | `doc/formal/passes.tex:31-33` | `languages/cq-lang-cq/src/normal.rs:244-248` | Expression depends on atom-supplied names; fresh variables. | Emits a post-atom binding and equality. | exact | Dependent arguments execute after their binder. |
| IF-Surface | `doc/formal/passes.tex:570-572` | `languages/cq-lang-cq/src/lib.rs:737-743` | CQ parses and no unexpanded extension remains. | Rejects remaining extension clauses. | exact | Unchecked surface extensions do not enter core CQ. |
| IF-Input | `doc/formal/passes.tex:574-575` | `languages/cq-syntax/src/lib.rs:334-369` | Unique relation/column names; positive arity. | Checks duplicates and arity without reordering declaration columns. | exact | Declaration order is host layout. |
| IF-Environment | `doc/formal/passes.tex:581-583` | `languages/cq-lang-cq/src/lib.rs:600-614,750-765` | Visible logical uses available; enclosing Rust expression type-checks. | Checks logical scope; enclosing Rust resolution/typechecking remains Rust’s obligation. | exact, conditional | Embedded Rust retains its closing environment. |

### Ordering, hypergraph analysis, and reduction

| Rule | TeX file:line | Code anchor opened | Premises as written | What the code checks/does | Status | INVARIANT |
|---|---|---|---|---|---|---|
| D-Atom | `doc/formal/passes.tex:50-51` | `passes/cq-order/src/lib.rs:158-167` | Atom supplies its variables; requires the subset supplied by scalar lets/generators. | Computes provided names and intersection with `let_bound`. | exact | Atoms cannot replace Rust-produced binders. |
| D-Guard | `doc/formal/passes.tex:53-54` | `passes/cq-order/src/lib.rs:181-184` | `R = uses(e,Bw)`. | Derives dependencies against the written prefix. | exact | Guard inputs exist before evaluation. |
| D-Bind | `doc/formal/passes.tex:56-57` | `passes/cq-order/src/lib.rs:185-191` | Pattern bindings and RHS written-prefix uses. | Computes both. | exact | Scalar binding dependencies are explicit to scheduling. |
| D-For | `doc/formal/passes.tex:59-60` | `passes/cq-order/src/lib.rs:192-198` | Item bindings and RHS written-prefix uses. | Computes both. | exact | Generator scheduling respects RHS dependencies. |
| D-Constraint | `doc/formal/ir_vo.tex:41-42` | `passes/cq-order/src/lib.rs:170-171` | Constraint variables available. | Requires variables and provides none. | exact | Constraints do not bind names. |
| C-Ready | `doc/formal/passes.tex` (`C-Ready`) | `passes/cq-order/src/lib.rs` (`C-Ready`) | Pending clause and bound free variables. | No Rust-chain or relational-predecessor condition. | **unverified** | Dependencies determine placement. |
| S-Written | `doc/formal/passes.tex:82` | `passes/cq-order/src/lib.rs:838-842` | No additional premise. | Uses `0..clauses.len()`. | exact | Written reference retains NF order. |
| S-Connected | `doc/formal/passes.tex:84-85` | `passes/cq-order/src/lib.rs:298-305` | Atom and number of shared equality-class names. | Ranks connected before disconnected, then clause index. | exact | Connected growth is a planner decision. |
| S-Filtered | `doc/formal/passes.tex:87-88` | `passes/cq-order/src/lib.rs:310-337` | Atom, connectivity, local filter rank. | Uses connectivity, filter rank, and written index. | exact | Filter priority is syntax-based. |
| S-Greedy | `doc/formal/passes.tex:90-92` | `passes/cq-order/src/lib.rs:298-305` | GreedyWidth/Auto; shared names; union width. | Ranks by connectivity and available/supplied union width. | exact | Greedy width is recorded in the chosen schedule. |
| S-Elimination | `doc/formal/passes.tex:94-96` | `passes/cq-order/src/lib.rs:343-369` | Analyzed elimination order; earliest supplied variable position. | Computes each atom’s earliest variable rank. | exact | Elimination ranking uses analyzed variable order. |
| O-Coarse | `doc/formal/passes.tex:98-99` | `passes/cq-order/src/lib.rs:416-440` | Chosen clause sequence. | Emits one full subatom or copied central/Rust step per clause. | exact | Coarse plans preserve full-row correlation. |
| O-Fine | `doc/formal/passes.tex:101-104` | `passes/cq-order/src/lib.rs:520-590` | Flush ready steps; first eligible variable; introduce then check. | Implements those operations and tracks incomplete atoms. | exact | Fine plans introduce at most one new variable per subatom. |
| H-FilterRank | `doc/formal/passes.tex:126-128` | `languages/cq-lang-cq/src/hypergraph.rs:64-127` | Local predicates through equality classes and literal arguments; minimum strength. | Assigns literal/point/range/domain ranks and ignores opaque guards. | exact | Selectivity priority makes no cardinality/key assertion. |
| CK-Order | `doc/formal/passes.tex` (`CK-Order`) | `passes/cq-order/src/lib.rs` (`check`) | NF source, valid VO, OrderObligations including independent branch premises. | Checks unchanged CQ, declarative branch policy, greedy Filtered choices, split and reduction exclusions without invoking branch selection. | exact, independently checked | Ordering preserves source CQ and schedule obligations. |
| H-Occurrence | `doc/formal/passes.tex:147-148` | `languages/cq-lang-cq/src/hypergraph.rs:140-177` | Atom or Rust-free block occurrence. | Adds occurrences with their headings; excludes Rustful block edges. | exact | Hypergraph vertices describe relational occurrences. |
| H-Incidence | `doc/formal/passes.tex:150-151` | `languages/cq-lang-cq/src/hypergraph.rs:229-242` | Occurrence and equality-expanded variable in its edge. | Inserts the occurrence into that variable’s incidence set. | exact | Incidence agrees with equality-expanded edges. |
| H-Adjacency | `doc/formal/passes.tex:153-154` | `languages/cq-lang-cq/src/hypergraph.rs:256-270` | Distinct occurrences with nonempty shared equality-class heading. | `shared` derives labels from equivalence; no stored adjacency IR. | exact, derived | Adjacency carries all shared join variables. |
| G-Private | `doc/formal/passes.tex:156-157` | `languages/cq-lang-cq/src/hypergraph.rs:285-299` | Variable occurs in only one residual edge. | Deletes the least private pair and records a GYO step. | exact | GYO preserves join-tree existence. |
| G-Absorb | `doc/formal/passes.tex:159-160` | `languages/cq-lang-cq/src/hypergraph.rs:300-312` | Distinct residual edges; one contained in the other. | Deletes the child and records its parent/step. | exact | Absorbed ears retain reconstruction provenance. |
| G-Accept | `doc/formal/passes.tex:162-163` | `languages/cq-lang-cq/src/hypergraph.rs:314` | Terminal residual has at most one empty edge. | Uses precisely that terminal predicate. | exact | Accepted hypergraphs are acyclic. |
| G-Reject | `doc/formal/passes.tex:165-166` | `languages/cq-lang-cq/src/hypergraph.rs:314` | Negation of G-Accept’s predicate. | Other terminal residuals are not marked acyclic. | exact | Cyclicity is not silently accepted. |
| H-JoinTree | `doc/formal/passes.tex:168-169` | `languages/cq-lang-cq/src/hypergraph.rs:339-340` | `acyclic = true`. | Produces `join_tree` only when acyclic. | exact | Full reducers require an actual join tree. |
| H-SpanningCore | `doc/formal/passes.tex:261-263` | `languages/cq-lang-cq/src/hypergraph.rs:315-339` | GYO ears plus most-shared/earliest core spanning tree. | Constructs the core tree with that ordering. | exact | Reduction trees remain available on cyclic scopes. |
| R-Guard | `doc/formal/passes.tex:189-190` | `passes/cq-reduce/src/lib.rs:338-390` | Source step is Guard. | Clones nonreduction steps; recurses into branch schedules. | exact | Reduction does not rewrite Rust payloads. |
| Y-Up | `doc/formal/passes.tex:192-193` | `passes/cq-reduce/src/lib.rs:433-449` | BFS order of join tree. | Parent-with-child edges in reverse BFS. | exact | Upward supports are reduced before use. |
| Y-Down | `doc/formal/passes.tex:195-196` | `passes/cq-reduce/src/lib.rs:443-449` | BFS order of join tree. | Child-with-parent edges in BFS. | exact | Downward support propagates from the root. |
| R-Tree | `doc/formal/passes.tex:198-199` | `passes/cq-reduce/src/lib.rs:264-285,318-334,415-449` | An acyclic join tree exists. | Builds up/down reductions and inserts at the first ready prefix/domain position. | exact | Tree reduction preserves the VO erasure. |
| R-Filtered | `doc/formal/passes.tex:214-217` | `passes/cq-reduce/src/lib.rs:454-550` | Acyclic; support component rank below 3; not range-only against point-filtered target component. | Propagates subtree/outside ranks and filters edges accordingly. | exact | Optional reductions use independent filter evidence. |
| R-Masks | `doc/formal/passes.tex` (`R-Masks`) | `passes/cq-reduce/src/lib.rs` (`mask_edges`, `mask_steps`) | Inner target; filtered child subtree of outer-rooted tree; outer excluded as target and support. | Builds child-first inner reductions and block cuts, excluding cuts through an outer block. | exact | Masks leaves the outer stream outside the reducer. |
| R-Placement | `doc/formal/passes.tex:265-267` | `passes/cq-reduce/src/lib.rs:75-91` (`R-Placement`) | Only independently hoisted blocks are reduction occurrences. | Excludes correlated block edges before hypergraph analysis, retaining written binders for scope. | exact | Prefix-dependent computations cannot become independent reduction operands. |
| CK-Masks | `doc/formal/passes.tex` (`CK-Masks`) | `passes/cq-reduce/src/correspondence.rs` (`check_masks`) | Exact eligible edges once; outer exclusion; ready boundary, cuts first, complete child support, strongest siblings. | Independently orients the tree by distance, inspects cut components, and validates recorded edge coverage and partial order. Does not call mask construction; independent subtree interleavings are accepted. | exact, independently checked | Recorded reductions satisfy Masks premises, not constructor equality. |
| CK-Reduce | `doc/formal/passes.tex:225-227` | `passes/cq-reduce/src/correspondence.rs:10-102,175-290` | Valid source/target VO and ReduceObligations. | Checks CQ/branch preservation, reduction-free source, erasure, edge/support provenance and tree/block obligations. | exact structurally | Reductions preserve all nonreduction source steps. |
| WF-JoinBranch | `doc/formal/ir_vo.tex:44-48` | `languages/cq-lang-vo/src/join_branches.rs:120-210` | Flat relational scope; disjoint connected full groups; exact interfaces; earlier branch-local reductions. | Checks every listed condition. | exact | Independent branch grouping is explicit and exact. |
| S-JoinBranch | `doc/formal/passes.tex:431-433` | `passes/cq-order/src/branches.rs:7-130` | Widest shared-key fact spine and connected dimension ears. | Applies the three-shared-column threshold, pinned filters, and deterministic ear absorption. | exact | Branch selection uses no uniqueness/FK premise. |
| CK-OrderBranches | `doc/formal/passes.tex` (`CK-OrderBranches`) | `passes/cq-order/src/branch_check.rs` (`check`) | Widest spine; pinned probes excluded; connected-ear proofs within recorded groups; final saturation. | Checks every contraction premise over recorded member ownership, then requires each group discharged and no remaining admissible ear. Group record order is irrelevant. | exact, independently checked | Static dimension groups cannot validate themselves through the selector. |

### Elaboration, RA, IX, Physical, and Structures

| Rule | TeX file:line | Code anchor opened | Premises as written | What the code checks/does | Status | INVARIANT |
|---|---|---|---|---|---|---|
| E-Emit | `doc/formal/passes.tex:291-292` | `passes/cq-elaborate/src/lib.rs:1166-1183` | Next unused `rN` in emission order. | Skips reserved names and appends the named definition. | exact | Emitted definitions have fresh identities. |
| E-Relation | `doc/formal/passes.tex:294-297` | `passes/cq-elaborate/src/lib.rs:721-759` | Unemitted occurrence; literal bindings and positional variable mapping. | Reuses cached results or emits the fixed-path rename with constants. | exact | Each source occurrence has fixed bindings. |
| E-Check | `doc/formal/passes.tex:299-300` | `passes/cq-elaborate/src/lib.rs:812-825` | Partial subatom; introduces nothing; prefix exists. | Emits a semijoin against the occurrence’s complete relation. | exact | A partial check preserves correlated occurrence witnesses. |
| E-Project | `doc/formal/passes.tex:302-304` | `passes/cq-elaborate/src/lib.rs:827-835` | Partial subatom not handled by E-Check. | Projects its recorded columns. | exact | Partial introduction retains precisely its subatom heading. |
| B-Root | `doc/formal/passes.tex:306-307` | `passes/cq-elaborate/src/lib.rs:836-838` | No prefix. | Installs the right subatom result. | exact | First occurrence establishes the prefix. |
| B-Join | `doc/formal/passes.tex:309-311` | `passes/cq-elaborate/src/lib.rs:836-843` | Prefix exists; E-Check inapplicable. | Emits a natural join with that prefix. | exact | Ordinary subatoms extend the valuation. |
| C-Guard | `doc/formal/passes.tex:313-315` | `passes/cq-elaborate/src/lib.rs:428-431,1058-1068` | Guard step; unitized prefix. | Copies condition/environment into RA Guard. | exact | Guard payload and lexical scope survive elaboration. |
| C-Bind | `doc/formal/passes.tex:317-319` | `passes/cq-elaborate/src/lib.rs:1071-1086` | Scalar let step, pattern binders, unitized prefix. | Copies scalar binding with no generator token. | exact | Scalar extension is not confused with unnesting. |
| C-For | `doc/formal/passes.tex:321-323` | `passes/cq-elaborate/src/lib.rs:1089-1103` | Generator step, item binders, unitized prefix. | Emits Bind with the `for` token and original RHS/environment. | exact | Set-valued generator semantics survive lowering. |
| E-ReduceWith | `doc/formal/passes.tex:325-327` | `passes/cq-elaborate/src/lib.rs:847-901` | Named target/support reduction. | Matches equality classes; uses semijoin or join/select/project; updates target result. | exact | Reduction retains only the target heading. |
| C-Finish | `doc/formal/passes.tex:329-331` | `passes/cq-elaborate/src/lib.rs:1108-1148` | All VO steps consumed; unitized prefix. | Retains valuation/head dependencies and emits output. | exact | Output bindings agree with the CQ head. |
| E-Constraint | `doc/formal/passes.tex` (`E-Constraint`) | `passes/cq-elaborate/src/lib.rs` | Constraint at recorded VO placement. | Realizes that selection; no independent pushdown. | **unverified** | CQ-to-VO owns predicate placement. |
| E-Domain | `doc/formal/passes.tex` (`E-Domain`) | `languages/cq-syntax/src/constraint.rs` (`implied_domains`); `facade/cq-planning/src/lib.rs` (`closed_disjunction_domains_preserve_literal_reduction_topology`) | Every disjunct restricts the projected variable against literal or query-closed operands. | Extracts common-variable unary domains while preserving closed expression payloads and the exact predicate. | exact, conditional on CQ-Closed | Operand encoding does not change necessary domains or reduction topology. |
| CT-Constraint | `doc/formal/passes.tex:344-347` | `languages/cq-lang-ra/src/structural.rs:621-641` | Exact predicate shape/bindings plus E-Domain closure. | Adds original and implied-domain predicate facts using fixed structural columns. | exact | Rewrites retain exact constraint meaning. |
| CK-Elaborate | `doc/formal/passes.tex` (`CK-Elaborate`) | `passes/cq-elaborate/src/lib.rs`; `passes/cq-elaborate/src/structural.rs`; `languages/cq-lang-ra/src/structural/blocks.rs` (`View::node`) | Fixed facts, Rust payloads/bindings and recorded placements. | Compares support and unchanged Rust without temporal predecessor facts; a block context substitutes all equal prefix-column origins, not only heading representatives. | **unverified** | Elaboration refines the supplied plan; joined-prefix fallback aggregate coverage is in `cq-planning`'s `fallback_aggregate_preserves_all_joined_prefix_origins`. |
| E-JoinBranch | `doc/formal/passes.tex` (`E-JoinBranch`) | `passes/cq-elaborate/src/join_branches.rs` | Recorded members and interface. | Builds their exact projected join. | exact, conditional | Exact branch support. |
| CK-JoinBranch | `doc/formal/passes.tex` (`CK-JoinBranch`) | `passes/cq-elaborate/src/join_branches.rs` | Fixed facts, Rust bindings and projected DAG. | Checks member/interface correspondence. | **unverified** | No Rust-order proof. |
| RA-Unit | `doc/formal/ir_ra.tex:33` | `languages/cq-lang-ra/src/lib.rs:401` | No premise. | Produces an empty heading. | exact | Unit introduces no columns. |
| RA-Rename | `doc/formal/ir_ra.tex:35-38` | `languages/cq-lang-ra/src/lib.rs:402-420,700-771` | Declared relation; mapping/constants partition source columns; injective targets; admissible constants. | Checks declaration, unique occurrence, source partition, target freshness and literal admissibility. | exact | Renames preserve positional source meaning. |
| RA-Join | `doc/formal/ir_ra.tex:40-41` | `languages/cq-lang-ra/src/lib.rs:421-428` | Both operands defined with headings `V,W`. | Requires earlier operands and unions headings. | exact | Join output heading is `V ∪ W`. |
| RA-Semi | `doc/formal/ir_ra.tex:43-44` | `languages/cq-lang-ra/src/lib.rs:445-450` | Left heading and defined right operand. | Keeps the left heading. | exact | Semijoin does not expose support columns. |
| RA-Project | `doc/formal/ir_ra.tex:46-47` | `languages/cq-lang-ra/src/lib.rs:429-444` | Projected attributes form a subset. | Rejects duplicates and unavailable columns. | exact | Projection cannot invent columns. |
| RA-Select | `doc/formal/ir_ra.tex:49-50` | `languages/cq-lang-ra/src/lib.rs:451-496` | Input heading; well-formed theta over visible variables/parameters. | Checks nonempty theta, scope, parameter separation and admissibility. | exact, conditional | Selection retains its input heading. |
| RA-Extend | `doc/formal/ir_ra.tex:52-53` | `languages/cq-lang-ra/src/lib.rs:510-532` | Input exists; RHS visible; introduced names fresh. | Checks environment/uses and freshness. | exact | Bind extends rather than overwrites a row. |
| RA-For | `doc/formal/ir_ra.tex:55-56` | `languages/cq-lang-ra/src/lib.rs:510-532` | Same scope/freshness, with item binders and generator tag. | Uses the shared Bind contract; preserves the tag. | exact | Generator output is a computed extension. |
| RA-Union | `doc/formal/ir_ra.tex:58-60` | `languages/cq-lang-ra/src/lib.rs:310-382` | Equal operand headings. | Requires at least two operands, compatible schemas and valid block provenance. | exact with the surrounding RAHeading contract | Union arms expose one interface. |
| RA-Anti | `doc/formal/ir_ra.tex:62-64` | `languages/cq-lang-ra/src/lib.rs:383-400` | Key subset of common heading. | Checks distinct common columns and admissibility; keeps left heading. | exact | Anti removes witnesses without adding columns. |
| RA-Def | `doc/formal/ir_ra.tex:66-67` | `languages/cq-lang-ra/src/lib.rs:283-294` | Operator well-formed and result name fresh. | Rejects duplicate result names before checking the operator. | exact | RA names denote one immutable result. |
| WF-RA | `doc/formal/ir_ra.tex` (`WF-RA`) | `languages/cq-lang-ra/src/lib.rs` | Headings, reachability and lexical scope. | No temporal Rust-chain obligation. | **unverified** | Free-variable identities are retained. |
| RA-Constraint | `doc/formal/passes.tex` (`RA-Constraint`) | `languages/cq-lang-ra/src/lib.rs` | Visible operands and representation laws. | Checks syntax/closedness, not host types. | **unverified** | Rust traits enforce operations. |
| RA-Group | `doc/formal/backend_ir.tex:1098-1101` | `languages/cq-lang-ra/src/lib.rs:297-309`; `passes/cq-physical-lowering/src/lib.rs:75-99` | Complete distinct named valuation domain; keyed groups, or singleton global domain. | Checks Group inputs/schema; lowering establishes required canonical input boundaries. Realization handles keyed/global cases. | exact, conditional on aggregate monoid laws | Every aggregate consumes the same valuation set. |
| IX-Reuse | `doc/formal/passes.tex:472` | `languages/cq-lang-ix/src/lib.rs:155-163` | Descriptor already present. | Deduplicates signatures in first-use traversal. | exact | Repeated keyed reads share a demand. |
| IX-New | `doc/formal/passes.tex:474-475` | `passes/cq-index-policy/src/correspondence.rs:27-58` | New descriptor and next `iN` identity. | Checks first-use descriptor and exact `iN` naming. | exact | Demand identity follows first use. |
| IX-Join | `doc/formal/passes.tex:477-480` | `languages/cq-lang-ix/src/lib.rs:118-133,149-155` | Join/semi; increasing RHS ordinals for shared columns; stream supply. | Derives RHS intersections; anti uses its explicit key. | exact | Join reads have exact stream demands. |
| IX-Const | `doc/formal/passes.tex:482-486` | `languages/cq-lang-ix/src/lib.rs:134-137` | Point conjunction with constant equality/literal membership. | Recognizes point columns and records const supply. | exact | Constant probes do not imply reusable builds. |
| IX-Range | `doc/formal/passes.tex:488-491` | `languages/cq-lang-ix/src/lib.rs:138-140` | One-column constant/parameter order window. | Records one range-supply ordinal. | exact | Range demand is explicit. |
| IX-Other | `doc/formal/passes.tex:493-495` | `languages/cq-lang-ix/src/lib.rs:141-147` | Operator not introducing a keyed demand. | Adds no descriptor. | exact | Non-keyed operators add no fictitious demands. |
| WF-IX | `doc/formal/ir_ix.tex:18-21` | `languages/cq-lang-ix/src/lib.rs:206-275` | Valid RA; semicolons; unique names/descriptors; valid demanded increasing ordinals. | Checks those conditions; intentionally allows partial coverage. | exact | IX descriptors are locally valid. |
| CK-Index | `doc/formal/passes.tex:497-499` | `passes/cq-index-policy/src/correspondence.rs:10-72` | Both contracts; unchanged RA; complete first-use `iN` descriptors. | Checks RA equality, every expected demand, names, and no extras. | exact | Index-policy output covers every required demand. |
| PH-Pipe | `doc/formal/ir_physical.tex:37-40` | `languages/cq-lang-physical/src/lib.rs:1180-1207,774-777` | Pipe; not literal-bearing rename; reusable host or linear/fused nonkeyed use. | Checks one-use/fusion/keyed exclusions and rejects literal pipe renames. | exact | Derived pipes have one legal stream consumption. |
| PH-Let | `doc/formal/ir_physical.tex:42-43` | `languages/cq-lang-physical/src/lib.rs:1208-1210` | Retained mode and positive read count. | Rejects unused retained results. | exact | Retention serves an actual consumer. |
| PH-Block | `doc/formal/ir_physical.tex` (`PH-Block`) | `languages/cq-lang-physical/src/lib.rs` (`check_rewrite`) | Branch scopes and free-variable identities. | Preserves payloads and recorded decisions. | **unverified** | No partial-expression ordering restriction. |
| PH-HostSet | `doc/formal/ir_physical.tex:86-88` | `languages/cq-lang-physical/src/lib.rs:1388-1400` | Recorded host-set fact and host rename. | Seeds full-heading uniqueness. | exact, conditional on fact discharge/certification | Canonicality follows checked context facts. |
| PH-Range | `doc/formal/passes.tex:513-514` | `languages/cq-lang-physical/src/lib.rs:917-924` | Constant order bounds on one column. | Requires recognized range bounds and records range reads. | exact | Physical ranges match their logical predicate. |
| WF-Physical | `doc/formal/ir_physical.tex:49-50` | `languages/cq-lang-physical/src/lib.rs:585-650,1180-1260` | PHeading and PMode. | Checks declarations/facts, operator graph, modes and choice admissibility. | exact structurally | Physical is independently checkable. |
| CK-Demands | `doc/formal/passes.tex:516-519` | `passes/cq-physical-lowering/src/lib.rs:458-508` | Every Physical keyed read except merge-left has an IX descriptor. | Checks each `(result, ordinals, supply)` against IX. | exact | Every keyed read has a recorded demand. |
| CK-Return | `doc/formal/context.tex:55-57` | `passes/cq-physical-lowering/src/correspondence.rs:31-56` | Inherited unique key, return canon, or canonical consumer. | Infers keys and checks the three-way return condition. | exact, conditional | Returned support has an explicit canonical boundary owner. |
| CK-Physical | `doc/formal/passes.tex:521-524` | `passes/cq-physical-lowering/src/correspondence.rs:11-56` | Both contracts; erasure equals source RA; demands; CK-Return. | Checks all four obligations. | exact structurally | Physical erasure equals source RA. |
| STR-Key | `doc/formal/ir_structures.tex:75-76` | `languages/cq-lang-structures/src/lib.rs:734-799` | Reusable host read or built key prefix. | Checks read coverage, payload depth, and membership alternatives. | exact | Every stream keyed read has a provider. |
| STR-Keyset | `doc/formal/ir_structures.tex:78-80` | `languages/cq-lang-structures/src/positional.rs:386-434` | Keyset, reusable source rows, exact membership capability. | Requires matching demand/source rows; checks known codec layouts; Rust checks provider traits. | exact, conditional | Supplemental keysets do not replace payload indexes. |
| STR-Existential | `doc/formal/ir_structures.tex:82-84` | `languages/cq-lang-structures/src/filtered.rs:139-258` | Filtered projection; observed payload subset of key; one read key; first/semi/anti readers. | Checks projected coverage, constants, reader shapes, and index layout. | exact structurally | Membership replacement loses no observable witness payload. |
| STR-FullKey | `doc/formal/ir_structures.tex` | `passes/cq-structure-selection/src/lib.rs`, `languages/cq-lang-structures/src/lib.rs`, `passes/cq-structure-selection/tests/selection.rs` | Set relation; one streamed read key covers the complete right heading; no ordered reads. | Records and validates raw host/built membership certificates; positive probes contribute no new valuation columns; Rust chooses dense or hash representation by traits. Shared smallest-candidate providers retain their enumeration policy. | exact structurally, conditional on provider laws | Raw-input code sets use bounded bitmap membership without payload indexes; no input uniqueness is assumed. |
| STR-Range | `doc/formal/ir_structures.tex:86-87` | `languages/cq-lang-structures/src/lib.rs:731-747` | Constant window on one column. | Const/range supplies do not require reusable built storage. | exact | Predicate windows can scan/filter without forced indexing. |
| STR-Merge | `doc/formal/ir_structures.tex:89-91` | `languages/cq-lang-structures/src/lib.rs:801-825` | Host/built keyed structure or same-key merge pipe. | Requires corresponding structure or sorted pipe; checks known incompatible types. | exact, conditional on Rust trait laws | Merge inputs are ordered on the merge key. |
| STR-Read | `doc/formal/ir_structures.tex:71-73` | `languages/cq-lang-structures/src/positional.rs:437-530` | Eager semi; earlier exact RHS keyset; matching lineage. | Checks RHS identity/order/key, nonlazy route, source spine, and summary ownership. | exact | Positional summaries describe their own earlier build. |
| STR-JoinFilter | `doc/formal/ir_structures.tex` | `languages/cq-lang-structures/src/join_filter.rs` | Eager covering probe/semi; recorded single-level exact key layout; exclusive spine; total build; marker and conservative capability. | Checks structural coverage and scan/key/marker agreement; rustc enforces provider `Keys` enumeration and conservative-filter traits. | exact, conditional on representation laws | A conservative filter never replaces the covering exact operator; custom providers use the same key-enumeration interface. |
| WF-Structures | `doc/formal/ir_structures.tex:93-95` | `languages/cq-lang-structures/src/lib.rs:410-535,734-923` | Valid entries; all Physical reads answered; CK-Return. | Checks representations, read coverage, retained/dedup types and inferred return canonicality. | exact structurally | Structures discharges Physical read obligations. |
| SEL-Positional | `doc/formal/passes.tex:550-552` | `passes/cq-structure-selection/src/positional.rs:7-90,111-158` | Explicit positional host; lineage; eligible reads. | Restricts to named hosts and supported spines/schema names, then records origins/markers/providers. | exact for the documented initial selector | Positional capability is opt-in and recorded. |
| SEL-JoinFilter | `doc/formal/passes.tex` | `passes/cq-structure-selection/src/positional.rs` (`choose_join_filters`) | Eligible covered scan; filtered payload build; positional host; fixed AdaptiveMembership choice. | Records provider, budget, marker, and covering witness without inspecting key types; membership-only builds need no duplicate filter. | exact | Default filter choice is explicit Structures text. |
| STR-AdaptiveMembership | `doc/formal/ir_structures.tex` | `runtime/cq-storage/src/adaptive.rs`; `runtime/cq-storage/src/dispatch.rs`; `runtime/cq-storage/tests/representation_laws.rs` | DenseOrdinal supplies a total injective ordinal and associated native word; the complete span converts exactly to usize and fits the budget. | Rust capability dispatch chooses dense construction; built-in integers use u64, wide custom ordinals retain u128, and arbitrary Eq + Hash keys or oversized spans retain exact hash membership. Compiler selection never inspects Rust type spelling. | conditional on the capability law | Dense and hash representations denote the same set; no truncation can turn a wide sparse domain into a dense bitmap. |
| CK-Structures | `doc/formal/passes.tex:555-557` | `passes/cq-structure-selection/src/correspondence.rs:7-20` | Both contracts; identical Physical AST and serialization. | Checks both AST and token equality. | exact | Structure selection cannot change Physical semantics. |
| STR-Repr | `doc/formal/backend.tex:31-36` | `languages/cq-lang-structures/src/repr.rs:215-283` | Connected lineage; valid owned columns; unambiguous origins; full heading coverage. | Checks origin matching, injectivity, computed ownership and coverage. | exact structurally | Physical row representation covers every logical column. |
| STR-Witness | `doc/formal/backend.tex:56-60` | `languages/cq-lang-structures/src/repr.rs:35-119` | Matching share/lookup witness; every path consumes it before escape. | Traverses consumers and rejects return/materialization/canon/scoped/unsupported escapes. | exact structurally | Hidden lookup cursors cannot escape their force boundary. |
| ST-Share | `doc/formal/backend.tex:138-144` | `languages/cq-lang-structures/src/share.rs:64-147` | Earlier immutable payload/snapshot; eager semi; later same-key probe; preserved keys. | Checks order, identities, supported layouts and key-preserving route. | exact for documented provider families | Shared lookup and force read the same immutable bucket. |
| ST-Total | `doc/formal/backend.tex` (`ST-Total`) | `languages/cq-lang-structures/src/stage.rs` | Represented operators and producer capabilities. | Batch vocabulary is representation policy. | **unverified** | All Rust is total by premise. |
| ST-Stage | `doc/formal/backend.tex` (`ST-Stage`) | `languages/cq-lang-structures/src/stage.rs` | Indexable producer, positive size and represented segment. | Records bounded batch demand. | **unverified** | Staging preserves the plan. |
| ST-Expand | `doc/formal/backend.tex` (`ST-Expand`) | `languages/cq-lang-structures/src/stage.rs` | Exact range cursor and represented segment. | Records segmented expansion. | **unverified** | Lengths are representation facts. |
| P-For | `doc/formal/passes.tex` (`P-For`) | `passes/cq-physical-realization/src/operators.rs` | Unchanged expression, free variables and finite pure total iteration. | Emits original RHS using flat_map or nested loop. | **unverified** | Generator semantics are preserved. |
| CTX-HostSet | `doc/formal/context.tex:38-39` | `languages/cq-lang-physical/src/lib.rs:589-609` | Host-set fact in context. | Checks a unique used declared host and admissible schema; truth is a certification/discharge premise. | exact, conditional | Host distinctness is not inferred from a Rust type. |
| CTX-Consumer | `doc/formal/context.tex:41-43` | `languages/cq-lang-physical/src/lib.rs:119-123,611-613` | Canonical-consumer fact and consuming context; support invariance. | Records the fact and disallows contradictory local return canon; consumer law remains a context premise. | exact, conditional | Final multiplicity is owned by the invoking consumer. |
| CTX-Discharge | `doc/formal/context.tex:75-78` | `languages/cq-lang-physical/src/lib.rs:218-240` | Fact in argument packet; invoking compiler proves it. | Applies recorded facts; the invoking compiler must establish the proof premise. | exact, conditional | Restricted facts enter through explicit packets. |
| Plan-Prefix | `doc/formal/passes.tex` (`Plan-Prefix`) | `facade/cq-planning/src/lib.rs` (`normalized_advice`, `prefix_valuation`, `compile_prefix`) | Normalize; rebase authored occurrence selectors; Prefix(Connected)/Coarse; optional Masks; advice; elaborate. | Normalization preserves atom/block order; the shared composition maps qualified selectors to normal clause positions before ordinary checks. | exact | Inserted closed restrictions never retarget authored relation occurrences. |
| R-Advice | `doc/formal/passes.tex:713-717` | `facade/mini-linq-macros/src/lib.rs:294-352` | Skip names an existing reduction; distinct top-level atoms share variables. | Resolves selectors, checks edges, removes/adds reductions before the stream, rechecks VO. | exact | Advice changes only admissible recorded reductions. |
| ST-Sequence | `doc/formal/composition.tex:18-21` | `facade/mini-linq-macros/src/lib.rs:407-423,512-539` | Sequence lowering/correspondence; host grading; terminal pull translation. | Uses checked lowering, grading and terminal translation. | exact | Terminal generation consumes checked Sequence. |

## Findings

### 1. Host-reader roles and result identifiers [C] — resolved

- **Former defect:** A single identifier represented either a synthetic role or a Physical consumer, so `scan`, `candidates` and `membership` could collide with legal result names.
- **Resolution:** STR-Reader records `ReaderRef::{Scan,Candidates,Membership,Result(Ident)}`. The grammar requires `host result(name)` for named consumers; roles have separate unambiguous syntax. `Analysis.fallbacks` keeps typed role/name keys, and checker, selection, realization, Sequence rewrites, GPU adapters and authored corpus use that distinction.
- **Regressions:** `passes/cq-structure-selection/tests/selection.rs` exercises role-like result spellings, scan/named-key coexistence, serialization round trips and rejection of obsolete ambiguous syntax. `languages/cq-lang-sequence/tests/sequence_language.rs` asserts that empty-key semi/anti capability obligations consume the named membership provider, never Scan. `features/own-dialect/tests/queries.rs` combines role-named readers with an authored BTreeMap group provider. The requested `cargo test -p mini-linq --test pipeline --test blocks --test openings` passed 59 tests, including all-wildcard negation; broader verification belongs to the integration report.

### 2. Independent policy checking [D/E/F] — resolved

- **Former defect:** CK-Masks invoked `mask_steps`, and CK-Order invoked `branches::choose`, so constructor mistakes could validate themselves.
- **Resolution:** `passes/cq-reduce/src/correspondence.rs` now checks R-Masks edge membership, completeness, outer exclusion, readiness and support-version partial order independently. `passes/cq-order/src/branch_check.rs` checks the S-JoinBranch spine/pinned/ear premises over recorded groups and final saturation, without constructing expected branch metadata.
- **Behavior:** Independent mask-subtree interleavings and reversed branch-record order are accepted. Unsupported, duplicate, missing, late or stale-child masks and strongest-sibling violations are rejected; outer block cuts are excluded. Well-formed groups containing a spine, pinned probe or uncontractible cycle are rejected.
- **Regressions:** `passes/cq-reduce/tests/reduce.rs` and `passes/cq-order/tests/order.rs` contain authored rejection cases and noncanonical-order acceptance cases. Verification results belong to the integration report.


### 3. Keyed aggregate accumulator representation [B] — resolved

- **Former defect:** Realization invented `std::collections::HashMap`, its hasher and key trait obligations.
- **Resolution:** STR-Group requires one `r group = Type;` entry precisely for keyed Group results. SEL-Group records the default `HashMap<_, AccTuple, cq_storage::HashState>`; the engine hash state is part of the representation, not inferred from Rust types. P-GroupState consumes the checked provider using empty `Default`, `entry/or_insert_with` and owned `IntoIterator`; map laws remain provider premises. Global groups retain their single identity accumulator without a table.
- **Regressions:** Selection tests reject missing, duplicate, non-Group and global-Group entries. Realization tests retain authored BTreeMap and standard HashMap providers, including their own hashers. Authored keyed-group fixtures retain their explicit accumulator rather than inheriting a new default. Verification belongs to the integration report.

### 4. Provider capabilities are reconstructed from Rust type spellings in the shared language [C/E] [severity med]

- **Evidence — governing docs:** `doc/DESIGN.md:4` says customization is “a pass A → B, a checked rewrite L → L, or an authored IR.” `doc/formal/ir_structures.tex:64-65` places provider obligations at the Rust boundary:
  > “Rust additionally checks positional host, marker, key, lifetime and provider obligations; a trait implementation must obey the storage abstraction laws.”
  
  The narrower existing rule is explicit: `doc/formal/backend.tex:169-170` identifies SortedRun, PrefixRun, and exact resident sorted hosts as range-cursor representations.
- **Evidence — code:** `languages/cq-lang-structures/src/stage.rs:117-120`:
  ```rust
  if path.path.segments.last().is_some_and(|segment|
      (segment.ident == "SortedRun" && levels.len() == 1 && levels[0] == keys)
          || (segment.ident == "PrefixRun" && prefix_depth(levels, &keys).is_some()))));
  ```
  The same file recognizes target-specific providers at `:128-130`:
  ```rust
  if path.path.segments.last().is_some_and(|segment| segment.ident == "FrozenDeviceIndex" || segment.ident == "LeveledDeviceIndex")
      && path.path.segments.iter().rev().nth(1).is_some_and(|segment| segment.ident == "cq_gpu_runtime"));
  ```
  Lookup sharing repeats provider-name recognition in `languages/cq-lang-structures/src/share.rs:125-131`.
- **Violation:** The shared language has no recorded provider capability descriptor/catalog reference for exact ranges or reusable lookup witnesses. It instead recognizes concrete type names, sometimes only the last path component, and embeds GPU-provider knowledge. The formal rules presently document a closed family, so this is **not** a claim that those implementations violate the current closed-family rule. It is a layering/extensibility problem: **[INFERENCE]** aliases and equivalent custom providers cannot preserve the same planning capabilities merely by satisfying the runtime laws, while unrelated same-named types enter syntactic recognition.
- **Belongs in:** Checked provider/catalog capabilities associated with a Structures entry, leaving Rust types as representation tokens and target-specific providers in target catalogs.

### 5. Recorded VO block placement — resolved [B/F]

- **Rules:** WF-BlockPlacement, O-BlockPlacement, O-WrittenBlocks, E-BlockPlacement, CK-BlockPlacement.
- **Recorded fact:** `languages/cq-lang-vo/src/blocks.rs` requires `block @path(columns) hoisted { ... }` or `correlated`. Parsing, printing and recursive reduction cloning preserve the enum.
- **Choice owner:** ordinary `cq-order::compile` hoists Rust-free blocks and correlates Rustful ones; `cq-order::compile_written` explicitly records correlated arms throughout the exact reference.
- **Consumer:** `cq-elaborate` reads that field for prefix/domain construction. Both public compilation entries preserve it; the written reference rejects hoisted input instead of silently changing it. Correspondence checks matching RA `over` provenance.
- **Coverage:** block-placement fixtures cover nested union/fallback, both elaboration entries, placement mismatches, repeated reads and free-variable scope.
- **Status:** placement is independent of expression syntax or totality classification.

### 6. Resolved contract: one opaque Rust expression

Contract 7 removes arithmetic recognition, wrapping rewrites and a second
Bind/Guard meaning. All Rust is pure-total by premise and emitted unchanged.
Only `where` is relational; closed operands use `Term::Closed(syn::Expr)`.
VO and RA compare payloads and free-variable identities, not written ordinals.
GPU eligibility uses identity representations and closed captures, not types.
These updated obligations have not been verified by running checks in this change.

### 7. The facade mutates the resolved pipeline using an exact pass path and unrelated authored-phase presence [C/E/F] [severity med]

- **Evidence — governing docs:** `doc/formal/composition.tex:6-8` defines entry by:
  ```text
  π = resolve(configuration, defaults)
  ```
  `doc/DESIGN.md:138-140` says:
  > “Omitted single phases use the core defaults; omitted lists are empty.”
- **Evidence — code:** `facade/mini-linq-macros/src/lib.rs:99-107`:
  ```rust
  .position(|pass| pass.path == sequential)
  {
      pipeline.passes.insert(
  ```
  The inserted path is `::mini_linq::passes::plan_sequential`; the gate at `:89-95` includes whether CQ `plan`, `select`, `represent`, `stage`, or rewrite lists were authored.
- **Violation:** Queue resolution is not the complete entry policy. The macro recognizes a particular spelling of the grading pass and conditionally adds a planning pass after resolution. **[INFERENCE]** Changing an unrelated phase from implicit to explicitly equivalent can change the number of planner invocations; an equivalent wrapper/path spelling is not treated as the same grade capability. This bypasses the explicit named-phase model and makes entry behavior harder to recover from the resolved configuration.
- **Belongs in:** `cq-stage` configuration resolution with explicit phase semantics/default decomposition, not a facade post-resolution path search.

### 8. Formal documentation and rule inventory still describe the production planner as Written/Coarse [F] [severity low]

- **Evidence — governing docs:** `doc/formal/passes.tex:785-786`:
  > “`plan` is left-deep in written order and defaults to checked Masks reduction.”
  
  `doc/formal/rules.tsv:94` summarizes Plan-Prefix as:
  > “Normalize, order Written/Coarse…”
  
  The same TeX file already gives the newer, contradictory statement at `doc/formal/passes.tex:114-118`: production uses Prefix(Connected)/Coarse.
- **Evidence — code:** `facade/mini-linq-macros/src/lib.rs:276-279`:
  ```rust
  let ordering = cq_order::Options {
      heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
      split: cq_order::Split::Coarse,
  ```
  Its rank is `passes/cq-order/src/lib.rs:305`:
  ```rust
  (true, shared == 0, width, clause)
  ```
- **Violation:** The default can move a later connected atom ahead of an earlier disconnected atom. **[INFERENCE]** For a Rust-free body shaped `R(a), S(b), T(a)`, the connected heuristic prefers `T` after `R`, not the written `S`. The Plan-Prefix TeX rule at `passes.tex:708-711` agrees with code; the prose and inventory summary do not.
- **Belongs in:** One consistent production-planner specification and rule inventory; keep Written/Coarse explicitly identified as the reference policy.

The dominant front/middle-end gap is not absent checking: most layers have substantial structural contracts, and IX/Physical/Structures preservation obligations are explicit.  
The remaining leaks are untyped identities/capabilities, repeated expression recognition, and planning/representation choices hidden in adapters or entry orchestration.  
Independent policy checking, block placement, typed reader roles and recorded keyed aggregate state are resolved. Provider-capability recording remains a separate layering issue.