# Physical stage necessity, order and semantic obligations

Status: historical audit of the pre-migration prototype, 12 September 2026.
The defects described below motivated [contextual physical lowering](CONTEXTUAL-PHYSICAL-LOWERING.md),
which declares contextual inputs, restores the IndexRequirements edge, and removes
compiler-owned source parsers. References below identify the audited boundary;
linked source files now contain the migrated implementation. The necessity
analysis remains relevant: moving private `Request` structs alone was insufficient.

## 1. The existing order is the starting point

The declared architecture is:

```text
CQ
  -- logical lowering / checked planning --> RelationalPlan
  -- index selection --------------------> IndexRequirements
  -- physical realization --------------> Rust context [Access regions]
  -- local Access lowering --------------> Rust context [IteratorPipeline regions]
  -- local pipeline lowering ------------> ordinary Rust
```

`IndexRequirements` contains the entire RA module, including named definitions;
it does not replace that module with index records. Its result-key namespace and
column ordinals are already specified. The materialized and lazy physical routes
are alternative implementations with their own admitted domains. The Rust
context is produced before its local Access holes are expanded: it is inaccurate
to model this as an unrelated new whole-program IR after Pipeline.

Sources: [RA](../crates/cq-ir/src/relational_plan.rs),
[IndexRequirements](../crates/cq-ir/src/index_requirements.rs),
[Access](../crates/cq-ir/src/rust_access_plan.rs),
[pass entries](../crates/cq-compiler/src/passes.rs), and the existing
[IX, M, ST rules](formal/physical.tex) and [IP/R rules](formal/staging.tex).

The audited `compile_physical` prototype instead accepts RA directly, performs
its own resource analysis and emits deferred provider/join/semijoin requests.
This both bypasses the existing index-selection boundary and introduces
additional local languages. Those changes require explicit justification. A
macro alias does not supply it.

Any proposed IR must identify its exact incoming and outgoing edges in this
graph. A representation of unresolved physical choices would belong within a
justified refinement of the physical-realization edge, before concrete Access.
It must not duplicate RA planning earlier, introduce storage decisions into IP/R
lowering later, or silently discard IndexRequirements in between.

Keeping or replacing an existing edge also requires honoring its contract.
IndexRequirements currently admits nonempty, ordered key requirements; it is
not already a general language for zero-column existence resources, summaries,
block layouts or construction schedules. A new implementation cannot reinterpret
these requirements as optional hints merely to pass through the right type.
An alternative edge or an extension of that language needs its own preservation
and coverage argument, including the nested-loop and empty-separator cases.

## 2. Necessity has not been demonstrated

The following requirements have witnesses in the existing languages:

| Required behavior | Existing representation |
|---|---|
| Logical natural join and exact semijoin | Existing RA operators and named operands |
| Different providers for different bindings | Physical-pass configuration and Rust binding/type construction |
| Hash versus nested-loop traversal | Alternative physical lowering to existing Rust and Access |
| Descriptor guard before payload traversal | Access `For`, `If`, `For`, `If`, in that order |
| Exact residual after a conservative filter | An existing Access `If` with a lawful Rust predicate |
| Resource construction, borrowing and reuse | Surrounding Rust context plus compiler analysis environments |
| Set collection | Existing Access/Pipeline set-valued endpoint |

For example, zone reduction can already produce this Access control shape:

```text
For descriptor in current descriptors
If conservative metadata test
For handle in selected rows of that descriptor
If exact membership of the handle's key
Output the surviving handle
```

This is a constructive expressibility argument. It establishes that zone pruning
and heterogeneous storage alone do not require another semantic IR. It does not
prove that every useful future physical optimizer can operate on those Rust
expressions, or that the current synthetic Access labels prove RA correspondence.

A new structured representation may be necessary if a separately scheduled pass
must inspect or transform unresolved physical obligations: for example, moving a
resource build across consumers while retaining its source version and ownership
scope. A necessity argument must specify that transformation, its input facts,
the information the existing languages fail to expose, and the preservation
property it needs. The argument is relative to inspectability and stage contracts,
not an assertion that arbitrary Rust cannot compute the answer.

Independent macro replacement requires a declared input protocol, but does not
by itself require another relational operator language. A candidate to evaluate
first is an existing checked source module, a selected definition identity, and
an explicitly specified administrative binding/configuration context. The pass
can derive keys and output fields from the retained RA. At an indexed physical
edge, the retained module is IndexRequirements, with the RA inside it. This
candidate still owes a precise serialization and contextual contract before it
can become a macro-stage API; calling the extra fields "configuration" is not a
substitute for specifying their meaning.

## 3. What the current requests erase

`joins.rs::Request` describes an equijoin and an output projection, but retains
neither its source RA definition nor its logical headings. Equal key arity is
insufficient. For `L(k,x) = {(1,10)}` and `R(k,y) = {(1,99)}`, natural join has one
answer. Pairing physical column 1 of L with column 0 of R has the same key arity
and valid Rust types, but produces no answer. A wrong output permutation can
also remain well typed because every column is `i64`.

Likewise, a resource built from version `r0` with keys `{1,2}` cannot serve a
reduction against later version `r1` with keys `{2}`. A same-shaped Rust resource
does not encode that distinction. And an implementation can read every payload
before evaluating its range guards, return the correct set, and violate pruning.

The audited source-to-invocation checker reconstructs some missing facts from
the original RA. That is useful evidence, but it does not give the private
request a standalone contract or certify arbitrary replacement macro bodies.

The audited implementation had six manual `Parse` implementations in five compiler files, plus two
provider argument-parser closures: join input/request, semijoin request, provider
request, physical input directives and checker materialization syntax. This
contradicts [the compiler crate boundary](../crates/cq-compiler/src/lib.rs).
Construction of an already declared AST with `parse_quote!` is a different case;
the problem is owning a new source grammar inside its lowering implementation.

## 4. Formal contract over existing semantic objects

The following are proposed contextual lowering obligations, not new AST nodes,
runtime relation objects, capability tags or registered compiler rules.

Let the checked RA environment assign each immutable named result a heading
`V_v` and denotation `D(v) ⊆ Tuple(V_v)`. A compiler environment records

\[
\rho(v)=(P_v,e_v,\ell_v),\qquad
\operatorname{Rep}_{P_v,\ell_v}(e_v,D(v)).
\]

`P_v` is a selected provider, `e_v` a Rust binding and `ell_v` a bijection from
physical field positions to the logical heading. `Rep` is a mathematical
relation: decoding the provider's selected rows yields exactly `D(v)`.
The implementation need not expose this relation as a universal Rust trait.

A resource environment records an origin and interpretation:

\[
\Xi(q)=(v,\kappa,b,k,e_q),\qquad
\operatorname{Keys}(e_q)=\pi_\kappa D(v).
\]

Here `kappa` is an ordered logical key, `b` the selected builder, and `k` its
exact-membership implementation. Exact membership means an iff, not possible
membership. A valid builder may borrow an existing structure; these equations
do not require allocation or scanning. Path equality alone cannot establish the
resource law or justify reuse of an effectful replacement.

Extend the existing M-Def style of contextual judgment:

\[
\Sigma;\Delta;\rho;\Xi\vdash(t=o)
\Downarrow C[A_1,\ldots,A_n]\dashv\rho';\Xi'.
\]

The source definition uses an existing RA operator. Each `A_i` is existing Access
syntax; `C` is ordinary Rust syntax owning setup, storage and collection. When
this judgment belongs to the indexed physical edge, the source also supplies
the checked IndexRequirements and the selected realization must discharge their
coverage obligations. Suppressing those premises in notation does not remove them.

For a binary operator define

\[
\kappa=\operatorname{ord}(V_l\cap V_r),\quad
i_l=\ell_l^{-1}(\kappa),\quad i_r=\ell_r^{-1}(\kappa).
\]

Every physical key map must implement this same ordered logical key. Output
layout is a bijection onto the RA result heading. These conditions are derived
from retained source information, not inferred from the fact that array indexing
type-checks.

## 5. Rule obligations and their algorithmic meaning

These labels identify proof obligations in this audit only. They are not added
to the implemented rule inventory or claimed to be implemented pass contracts.

| Obligation | Premises | Conclusion / generated algorithm |
|---|---|---|
| Exact resource | `Rep` holds for source version `v`; key map implements `kappa`; builder is adequate | Fresh `q` has exactly `π_kappa D(v)`; record its origin in `Xi` |
| Resource reuse | Same exact source version, ordered key and lawful builder/key composition; lifetime and repeated-read laws hold | Use the existing binding; no extra construction |
| Exact semijoin | Source is `t = l ⋉ r`; current left selection is exact; resource keys equal `π_kappa D(r)` | Traverse current left handles, keep exactly matching keys, construct the selected result |
| Zone semijoin | Exact-semijoin premises plus sound descriptor bounds, complete current descriptor coverage and sound conservative rejection | Metadata traversal → guard → selected payload traversal → exact residual → selection |
| Nested-loop join | Source is `t = l ⋈ r`; both bindings represented; paired key maps and result layout are correct | Enumerate all compatible pairs and collect their merged tuples |
| Hash join | Same source/layout premises; buckets contain exactly the right rows for each key | Build or obtain exact buckets, scan left, enumerate precisely matching right rows |
| Materialization | Access collection denotes the intended set; constructor preserves it | Fresh target binding satisfies `Rep`; previous bindings remain represented |

Unit, Rename and Project retain their existing logical meanings. Set
normalization is necessary for projection and pair enumeration; no new logical
operator is introduced by these alternatives. Source expression evaluation
multiplicity and Rust lifetime/scope must be stated by the stage contract.
Generated recipes preserve complete provider handles when filtering, reordering
or deduplicating them; they do not invent or recombine identity/address/row fields.

For a descriptor occurrence `d`, let `B_d` be its currently selected rows and
`K = π_kappa D(r)`. The conservative premise is

\[
(\exists x\in B_d.\ x|_\kappa\in K)
\Longrightarrow \operatorname{mayMatch}(q,d).
\]

Consequently the two semijoin rules both represent

\[
D(t)=\{x\in D(l)\mid x|_\kappa\in K\}.
\]

The empty key case distinguishes `K = ∅` from `K = {()}`. Summary false positives
are permitted; missing the exact residual is not. A stale key source violates
the exact-resource premise even if the generated Rust is well typed.

Result preservation and access effects are separate obligations. For each zone
consumer occurrence, every payload-open event must follow that occurrence's
successful metadata guard; a rejected descriptor occurrence has no payload-open
event. Descriptor enumeration and bounds inspection must themselves avoid opening
payload. These conditions do not forbid a different consumer, including a resource
builder, from accessing the same underlying storage for its own operation.
Work counters and execution order can differ between correct recipes; equality
of answer sets does not claim equality of their complete observable traces.

Conditional preservation follows by induction over the existing named RA
definitions: exact membership proves reduction, conservative rejection plus the
residual proves zone reduction, pair/bucket adequacy proves join, and fresh
materialization establishes the next `Rep` invariant. Resource reuse requires
its origin and lifetime premises. This argument additionally assumes lawful
host operations and the existing Access/Pipeline/collection preservation results.
It is a proof plan over abstract algorithms, not a proof of the current emitters.

## 6. Admission and implementation order

The next representation decision must produce all of the following before a
compiler implementation consumes it:

1. A concrete transformation or independent scheduling requirement, and a
   comparison with existing source-plus-context staging. If that comparison
   finds no gap, retain existing IRs and specify the contextual pass interface.
2. The exact position in the declared graph, including retained source
   information, preserved IndexRequirements obligations, and the facts fixed
   versus still unresolved at that point. Alternative edges need explicit
   contracts; a folder name does not create one.
3. For any genuinely new language: a minimal grammar, semantic domains,
   well-formedness and rejection judgments, inference rules, and incoming/outgoing
   preservation statements. Every field and operation must serve a stated premise
   or observable requirement. No duplicate operator/key/output vocabulary merely
   to transport an implementation's local variables.
4. Representation first in the syntax-owning crate: follow the existing
   syntax-preserving `syn_derive::Parse`/`ToTokens` convention, reuse existing
   column/list/RA types, and add local contract and round-trip/rejection tests.
   Administrative macro parameters remain distinct from query semantics.
5. Typed producer and consumer pass entries following `passes.rs`, with separate
   source/target correspondence checks. Macro entries parse; implementation
   modules receive typed inputs. Each algorithm block maps to named rules and
   concrete handlers. The checker uses the declared syntax instead of inventing
   a second parser, while independently checking the semantic correspondence.
6. Only then migrate the prototype emitters and remove their private grammars.
   Preserve the existing acceptance witnesses: per-binding representations,
   borrowed key builders, selected versions, block pruning, alternate summaries,
   set-valued endpoints and independent macro choices. These tests accompany
   structural and semantic checks; they do not replace them.

Audit conclusion: the prototype has an architectural defect, and another
physical IR has not yet earned its place. The formal contextual obligations
above establish what a corrected boundary must preserve. They do not authorize
inserting a new stage, reordering existing stages, or treating the current private
request structs as an already designed IR.
