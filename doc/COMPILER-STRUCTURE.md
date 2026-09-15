# Compiler ownership by macro boundary

The compiler is organized into **11 compiler/storage packages**, excluding the
root course package, workloads and tooling. A package owns a coherent semantic
boundary family; modules own its independently selectable implementations. The
[formal snapshot](formal/snapshot.txt) records validation for reviewed sources.

The replacement unit is an implementation of a semantic macro boundary. Its
module owns the algorithm, admitted domain, configuration, correspondence checks
and tests. Shared helpers stay within that family unless they express
independently useful facts about an existing language.

## Boundaries and obligations

The language graph remains:

```text
CQ → RA → IndexRequirements → Rust[Access] → Rust[Pipeline] → Rust
     RA ── configured direct physical lowering ──→ Rust[Access]
```

For an implementation `a`, write `compile_a : L -> Result<T_a>`. The target
`T_a` is an existing IR or a Rust fragment containing holes `Rust[H1, …, Hn]`.
`L` and the holes `Hj` are existing languages or native Rust fragments. Each
implementation has an admitted domain and an observation to preserve. Its selected
continuation must accept the emitted hole and satisfy the required binding,
ownership and observation laws. A common token grammar or resolving macro name
does not prove composition correct. These are mathematical obligations, not a
Rust `Pass` trait, capability registry, route tag or serialized IR.

| Boundary | Implementation modules | Responsibility |
| --- | --- | --- |
| CQ → RA | canonical, prefix, graph | Scheduling, rules, domain, derivation/replay and correspondence |
| RA → IndexRequirements | input, result | Key-demand policy, namespace and independent correspondence |
| IndexRequirements → Rust with Access | input, lazy, materialized | Representation, construction, lifetime and generated computation |
| Configured RA → Rust with Access | direct | Per-binding layouts, current result versions and resource lifetime |
| Candidate access | scan, prune | Enumeration and its conditional exact-residual obligation |
| Key resource | exact, zone | Construction, exact membership and, for zone, conservative range tests |
| Witness resource | hash, nested | Construction and every probe using that representation |
| Row provider | hash, sorted, block | Types, input views, handles, selection, materialization and supported block operations |
| Access → Pipeline | pull | Binding/control translation independent of endpoint |
| Pipeline → Rust | iterator, collect, stream | Endpoint realization and its set/sequence/demand observation |

A resource protocol is indivisible for replacement: zone construction and the
operations interpreting its resource must agree; hash construction cannot pair
with nested probing. Candidate and key-resource choices remain separate, with
their joint obligation established by the enclosing exact semijoin.

Exported spellings do not create additional implementations. `pull`,
`collect_access` and `stream_access` use one translation with different
continuations. `result_indexes` and `planned_relational_plan` use one index policy.
Materialized program and callable entries share a computation with different
hosts. `workload_query` is a catalog frontend.

## Packages and modules

| Package | Modules / responsibility | Permitted local dependencies |
| --- | --- | --- |
| `cq-ir` | Existing languages, local judgments and factual syntax/heading queries | None |
| `cq-storage` | Data structures, operation traits, selections, summaries and passive counters | None |
| `cq-plan` | `canonical`, `prefix`, `graph`; scoped dependency/proof support | `cq-ir` |
| `cq-index` | `input`, `result`, each with its domain and checker | `cq-ir` |
| `cq-physical` | `input`, `lazy`, `materialized`, `direct`; private Rust/check helpers | `cq-ir` |
| `cq-access` | `candidates::{scan,prune}`, `keys::{exact,zone}`, `witnesses::{hash,nested}` | `cq-ir` |
| `cq-providers` | `hash`, `sorted`, `block`; complete representation protocols | `cq-ir` |
| `cq-pull` | Access → Pipeline implementation and checker | `cq-ir` |
| `cq-pipeline` | `iterator`, `collect`, `stream` and endpoint checks | `cq-ir` |
| `cq-compose` | Named default compositions and source-dependent route selection | Selected compiler families and `cq-ir` |
| `mini-linq` | Thin procedural-macro entries and administrative parsing | Selected families, optional `cq-compose`, `cq-ir`; storage for execution tests |

The dependency guard checks normal, development, build, optional, target-specific
and renamed dependencies. An implementation family cannot import another family
or the composition layer. Cross-family execution tests belong to composition or
frontend consumers. Generated Rust may use caller-selected storage paths without
the compiler library linking the runtime storage crate.

Implementations are ordinary Rust libraries because typed clients use `compile`,
`check`, `explain` and `replay`. The frontend root exports thin macros; its parsing
helpers are private modules. Rust requires exported procedural macros at the root
of a `proc-macro` crate, but not their implementation code.
[Rust Reference](https://doc.rust-lang.org/reference/procedural-macros.html#macro.proc.def)

For example:

```text
cq-access/src/
  lib.rs
  candidates/{mod.rs,scan.rs,prune.rs}
  keys/{mod.rs,exact.rs,zone.rs}
  witnesses/{mod.rs,hash.rs,nested.rs}
```

Small implementations need no mandatory file template; larger ones split private
files. Parent modules declare alternatives and narrowly scoped shared helpers.
They do not select a recognized backend through Boolean or enum dispatch. The
native-set helper takes the selected type syntax from its owning hash/sorted
module; it does not decide between them.

Features independently include implementation modules. Frontend `prune-rows`
enables only `cq-access/candidate-prune`; it does not enable zone keys, block
storage, pull or a Pipeline endpoint. Those are separately supplied choices.
Family dependencies use `default-features = false`; optional convenience bundles
belong to composition. A selected macro has the same meaning when another
implementation is enabled. Cargo features are additive and can unify across
consumers.
[Cargo Book](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification)

**Enforcement limit:** Cargo isolates families, not sibling modules. Reviewed
source inventories, import checks and actual source-removal builds check sibling
independence. This is not one-package-per-algorithm isolation. An implementation
needing its own dependency stack or release lifecycle can later become a crate
without changing its macro protocol.

## Shared code and analysis ownership

| Fact or helper | Owner |
| --- | --- |
| Planner dependency facts, options and derivation/report support | `cq-plan::{dependencies,model,proof}` |
| Native Rust expression aliases and fresh locals | Private `cq-physical::rust_bindings`, used by emitters |
| Independent syntax observations and hash/set recognition | Private `cq-physical::check_support`, used by checkers |
| Input access reconstruction and ownership facts | Private `cq-physical::input::analysis` |
| Lazy linear-prefix domain | `cq-physical::lazy::domain` |
| Input-index domain and expected canonical demands | `cq-index::input::{domain,correspondence}` |
| Sufficient index coverage for a physical realization | Each consuming physical implementation |
| Namespace-preserving index signature | `cq-ir::index_requirements::signature` |
| Shared-heading coordinates | `cq-ir::relational_plan::shared_columns` |

The input-index emitter derives demands from RA headings and Rename coordinates.
Its checker independently derives expected demands. Physical checks verify
sufficient supplied coverage; they do not invoke the selector or require its
preferred ordering.

For `R(a,b) ⋈ S(b,c)`, a factual query can report `{b}` and its coordinate in `S`.
It cannot prescribe a right-result hash index for every backend. Base-input and
named-result namespaces remain distinct, including self-joins and updated RA
versions. Route restrictions never become RA validity.

Planner implementations share source facts and proof vocabulary; each owns its
decisions. Graph planning never calls prefix lowering. Independent checkers may
share structural observations with other checkers, but never call an emitter to
manufacture an expected target. Small independent traversals preserve this
separation without an analysis framework or another IR.

## Configuration belongs to the selected implementation

Native configuration follows the selected implementation and operation:

| Implementation | Required configuration, where used |
| --- | --- |
| Candidate scan | None |
| Candidate prune | Iterator continuation |
| Exact keys | Build: iterator and collection; contains: exact-membership trait path |
| Zone keys | Build: iterator, collection and summary/trait path; tests: relevant trait path |
| Hash/nested witnesses | Build: iterator; probe needs no iterator configuration |
| Native hash/sorted rows | None |
| Block rows | Storage/trait path; materialization additionally needs packing size |

Only operations that observe work use an optional counter expression. A binding
can close over configuration for its whole protocol; a direct membership call
needs no construction iterator. Derived frontend parsing separates administrative
directives from existing IR operands. Implementation-specific option values and
checks determine which directives are accepted and required.

Configured bindings are ordinary lexical Rust macros. For example:

```rust
macro_rules! r_rows {
    ($($op:tt)*) => {
        raw::block_rows! { storage ::cq_storage; $($op)* }
    };
}
macro_rules! r_candidates {
    ($($op:tt)*) => {
        raw::prune_rows! { iterator raw::stream_access; $($op)* }
    };
}
macro_rules! r_keys {
    ($($op:tt)*) => {
        raw::zone_keys! {
            storage ::cq_storage;
            iterator raw::stream_access;
            keys ::std::collections::BTreeSet;
            $($op)*
        }
    };
}
```

`raw` aliases the macro frontend. `r_rows` covers borrowed input access; a binding
used for block materialization also closes over `packing (expression);`. Direct
physical lowering receives configured paths per input/result, candidate, key
resource and witness resource, plus its own Access continuation. It does not
inject a leaf's storage, collection or iterator choices. The same configured
provider supplies types and operations. One query can use R=block, S=hash and
T=sorted independently.

The native materialization operation is `materialize owner value (rows)`.
Packing size belongs only to block-provider configuration. Rows evaluate first,
then packing size, each once per actual construction. The provider represents the
supplied rows exactly and keeps its handles within valid storage lifetimes.
Generated helper bindings must preserve caller expression resolution, including
names hidden inside macros.

The direct edge has plain and observed item/callable hosts. Item hosts expose
`run` without a global packing argument; bound expressions must be legal in that
item's scope. `physical_query` and `observed_physical_query` return closures over
borrowed inputs in declaration order. Configured expressions can capture caller
values, including dynamic packing sizes. Both return owned answers; internal
handles cannot escape their input/storage lifetime. All four hosts share the
same RA definition lowering.

The observed host owns counters and per-result checkpoints. It forwards the
actual counter expression through `work (expression);` to configured operation
bindings. The stats storage path does not choose relation representations.
Type-position provider calls use the plain binding without an observer prefix.
Paired plain/observed bindings must use the same representation.

For example, an observed pruning adapter binds its iterator and forwards work:

```rust
macro_rules! observed_candidates {
    (work ($work:expr); $($op:tt)*) => {
        raw::prune_rows! {
            iterator raw::stream_access;
            work ($work);
            $($op)*
        }
    };
}
```

An observed scan adapter may discard that prefix; raw scan also accepts it
without emitting or evaluating the unused expression. Plain-only adapters need
no observer support. This is an explicit observer convention, not an extra
relational operation or a universal runtime API.

Two binding invariants govern direct lowering:

- Key-resource reuse requires the same current right-hand RA result, ordered key
  coordinates and configured macro binding. Different aliases cannot share
  merely because they wrap the same raw implementation. Path spelling within a
  fixed lexical scope is a conservative identity, not equivalence discovery.
- Wrappers forward operand tokens without expanding them. Scan discards its
  unused metadata predicate, so unsupported `may_match` tokens do not expand.
  Pruning expands them and rejects exact-only keys or providers without block
  operations through selected expansion or Rust typing.

## Decision ownership and composition

| Decision | Owner |
| --- | --- |
| Explicit algorithm / next stage | Caller aliases and configured bindings |
| Source-dependent default route | Named composition in `cq-compose` |
| Legal operator order | Selected planner implementation |
| Indexed realization domain and coverage | Selected physical implementation |
| Storage per input/result | Direct physical binding choices |
| Zone/exact, scan/prune, hash/nested | Selected native implementation modules |
| Block packing and key collection | Selected provider/resource binding |
| Input-data-dependent execution choice | An implementation emitting both valid Rust branches |

`cq-compose::with_default_indexes` chooses the default index policy and physical
continuation together at the RA boundary. `expand_indexed` emits the resulting IX
inside the selected caller-supplied macro path. `compile_indexed` uses the same
branch for typed complete-program compilation. The path is administrative wiring;
IX carries no route identifier. Composition never names the frontend crate.

Frontend defaults use invocation-relative paths such as
`mini_linq::input_indexed`, permitting lexical aliases and renamed dependencies.
Explicit index entries still emit the caller-resolved `index_requirements`
continuation. That spelling can name a selected physical macro in a wiring
module; there is no standalone automatic selector that guesses an earlier index
choice. Independently authored IX invokes an explicit physical entry and receives
that implementation's diagnostics.

Composition does not infer block support from provider names or replace a user's
hash storage. Successful expansion does not establish provider semantic laws.

## Verification and teaching extraction

[The dependency/source-removal gate](../tools/check_dependencies.py) checks:

- The package dependency table, exact frontend feature forwarding and rejected
  forbidden-dependency mutations.
- Twenty-four implementation/foundation source closures with siblings physically
  omitted. Only declarations and feature metadata are reduced; algorithm sources
  are copied unchanged. Family tests also run with all implementations enabled.
- Twenty-two external frontend probes with renamed dependencies and actual macro
  calls. Replacement continuations work while built-in next stages are disabled.

```sh
python3 tools/check_dependencies.py --isolate
```

The [package feature matrix](../tools/check_features.py) separately runs ordinary
`cargo test -p mini-linq` commands with no defaults, each completed feature,
default/completed bundles, and the feature combinations declared by test/example
targets. Cargo prerequisites gate multi-stage tests; native boundary tests keep
their individual feature gates, so scan-only builds execute the scan regression.
Cases are derived from Cargo metadata, including transitive feature dependencies.
The unfinished `hw4` exercise is excluded. This matrix checks supported package
workflows, not every possible subset of features.

```sh
python3 tools/check_features.py
python3 tools/check_features.py --only scan-rows
```

Execution and correspondence remain separate gates. They cover independently
authored RA/IX/Access/Pipeline, mutated targets, operation-specific configuration,
discarded operands, once-only evaluation, caller name resolution, dynamic packing,
owned outputs, heterogeneous blocked-YA, current masks, stale versions,
differently configured resources and observed work counts. See the
[composition tests](../crates/mini-linq/tests/composition.rs),
[standalone example](../examples/composition/README.md),
[blocked-YA example](../examples/blocked-ya/main.rs) and
[native boundary tests](../crates/mini-linq/tests/native_boundaries.rs).

[YA extraction](../teaching/ya/README.md) selects graph, result-index,
materialized, pull and collection implementations into seven packages, including
IR and the frontend. Other implementations are physically absent. The current
profile inventories 80 delivered files and separates two student algorithms:
the YA scheduler and semijoin lowering. Verification checks the supplied
foundation, each missing exercise separately, 75 reference-overlay tests and
both examples, then restores the stubs and checks every inventory hash. No parent
checkout, storage runtime or unrelated compiler family is required.

The formal rule/source maps and [snapshot](formal/snapshot.txt) record reviewed
implementation correspondence and completed validation. Dependency isolation,
module extraction and semantic correctness are distinct claims. These checks do
not prove arbitrary provider laws, arbitrary compositions, full Rust elaboration,
recursive program semantics or a general physical optimizer.
