# Physical selection and realization

The implemented compiler follows:

```text
RA → IX → Physical → Rust[Access]
```

Read the [short PDF](formal/build/physical_design.pdf),
[complete paper](formal/build/compiler.pdf), or their shared
[formal specification](formal/ix_physical_contract.tex).
The generic driver and a borrowed exact-index instance are implemented. The
old whole-query/configured/Native routes have been removed; broader physical
coverage remains incomplete.

## Boundaries

| IR | Fixed information | Owner |
| --- | --- | --- |
| RA | Operators, operands, exact relation versions | `cq-query-lowering` |
| IX = `(A,D,U)` | Unchanged RA, named index demands, references at indexed join positions | `cq-index-policy` |
| Physical = `(IX,Φ,Λ)` | Unchanged IX, provider per relation/version and index, lowering per RA definition | `cq-physical-selection` |
| Rust[Access] | Concrete borrowed storage/index expressions and selected accesses | `cq-physical-lowering` dispatches external definitions |

`Φ_R` binds all used inputs and definition results. `Φ_I` binds every IX index.
`Λ` binds every RA definition, including Rename and Unit. Physical path spelling
is opaque to the common compiler. The explicit-binding policy checks structure;
external definitions and Rust type/lifetime checking establish their admitted
representation. The compiler does not prove arbitrary provider laws.

Realization has no policy parameter. It prepares inputs, visits definitions in
dependency order, and establishes each written index before its first use.
Unsupported recorded choices reject; realization does not substitute a provider.
The same storage can support different algorithms, which is why `Λ` is recorded
separately from `Φ`.

## Actual macro composition

The caller aliases its policy as `mini_linq::choose_physical`. That policy supplies
concrete assignments, so it contains real configuration rather than an empty wrapper:

```text
index_demands! { RA }
  → mini_linq::choose_physical! { IX }
  → given_physical_bindings! { IX physical { assignments } }
  → mini_linq::realize_physical! { Physical }
  → selected providers/lowerings and their continuations
  → { Rust statements; mini_linq::access! { Access } }
```

The fixture imports `realize_physical` and selects `access_to_set_loops as access`.
Those are stage/execution bindings. The assignments inside Physical independently
select each relation/index/result provider and operator rule.

The [administrative protocol](PROVIDER-PROTOCOL.md) specifies typed `Input`,
`Index`, and `Lower` requests, plus the optional exact family's `Rows`, `Matches`,
and `Output` messages. Continuations disappear during realization; they are not
another retained whole-query IR. The generic driver depends only on `cq-ir`.
Rows/Matches return an iterator expression, its item pattern and logical witness;
Output returns only a witness. The selected RA rule assembles neutral
`=> bind PAT <- (EXPR)` Access clauses. Eager loops and lazy polling are decisions
of the Access consumer. Named Pipeline definitions remain explicit until their
final Rust lowering introduces query-composition adapters such as `flat_map`.
The separate `cq-example-providers` proc-macro crate implements the concrete
slice, hash, sorted, borrowed-output, Rename, indexed-join and terminal-Project definitions.

## Actual expansion and validation

The executable three-input query is in
[physical_realization.rs](../crates/mini-linq/tests/physical_realization.rs).
Its [authored boundary fixtures](../crates/mini-linq/tests/fixtures/physical-realization/)
specify full IX, Physical assignments, and completed Rust[Access]. The
[actual trace](ix-physical/realization-trace/README.md) records every real
proc-macro input/output, including the external provider callbacks. The
[review guide](ix-physical/realization-trace/review.md) separates stage boundaries
from administrative exchanges. The trace acceptance test compares tokens after
parsing, ignoring spans/formatting, and checks frozen Physical payloads.

Runtime tests compare with independent relation enumeration. They exercise
empty inputs, missing matches, duplicate support, multiple matches, non-`Clone`
borrowed fields and their pointer provenance. Compile-fail cases have a passing
control and reject wrong relation/index representations and unavailable indexed
columns. [Variant tests](../crates/mini-linq/tests/physical_variants.rs) cover local
provider rebinding, renamed index ordinals, macro hygiene, and a caller-defined
nested output carrier without common-compiler changes.

The older [selection-only trace](ix-physical/selection-trace/README.md) deliberately
stops at Physical. The local two-output and auxiliary files in `doc/ix-physical`
are authored expectations; neither is relabeled as a successful compiler capture.

The [full CQ query](../crates/mini-linq/tests/physical_query.rs) additionally
traverses CQ → RA and selects an external terminal projection. Its
[separate trace](ix-physical/query-trace/review.md) and authored fixtures are
checked independently, so calls from different queries are not mixed. A second
projection test exercises Rows from stored input. Intermediate projection is
explicitly rejected because discarded Access bindings must not silently constrain
a later reintroduction of those logical variables.

The [separate pull query](ix-physical/pull-trace/review.md) continues through the
existing named Pipeline. Its actual calls 042 and 043 show Access → Pipeline and
Pipeline → Rust separately. Source counters verify deferred evaluation,
short-circuiting and resumption; complete results still match the independent
oracle. The eager and pull reports use independent call numbering.

## Implementation domain

The current complete host-expression realization admits unary borrowed slices,
binary borrowed hash/B-tree maps, direct Rename, exact indexed natural joins,
and terminal projection.
A map can satisfy only its physical first-column index, translated into the
renamed logical heading. Left Access regions compose by inlining their body;
there is no intermediate collection. Final outputs must use normalized logical
heading order. Output carriers may differ while preserving the same logical
field map.

The caller supplies ordinary Rust input expressions and a Rust function signature.
No generated `build/query/run` API is assumed. Access's eager set and iterator
consumers remain independently usable; deciding general push/pull placement is
separate work.

The retired prototypes' following behavior is **not ported**:

- General schemas, composite/empty-column index providers, and building multiple
  indexes over one stored relation.
- Stored intermediates and indexes over reduced/projected relation versions.
- Physical `Unit`, intermediate `Project`, `Select`, `Extend`, and `Semijoin` rules.
- Block/YA access, metadata-before-payload rejection, exact residual checking,
  selected-row provenance and observer counters.
- Deferred index construction/replay and generated host/callable APIs.

These are removed execution routes, not compatibility aliases or fallback paths.
Their earlier tests are not evidence for the new compiler. Independent CQ/RA/YA
oracle/proof tests moved from `cq-compose` to `cq-query-lowering`; Access/Pipeline
and storage-law tests remain in their owning crates. Unsupported intermediate
indexing currently gives an explicit compile error. Extending this domain needs
specified provider/rule behavior and real expansion fixtures first. Any new IR
or protocol syntax requires review before implementation.
