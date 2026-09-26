# Compiler ownership and composition

The accepted path is `CQ → RA → IX → Physical → Rust[Access]`. The implemented
post-IX domain is an open Rust expression over caller-owned inputs. See the
[design contract](IX-PHYSICAL-REDESIGN.md) and
[provider protocol](PROVIDER-PROTOCOL.md) for syntax and obligations.

## Crates and modules

| Crate | Responsibility | Modules / entry functions |
|---|---|---|
| `cq-ir` | Typed syntax and local validity | `cq`, `relational_plan`, `index_requirements`, `physical`, `provider_protocol`, `rust_access_plan`, `iterator_pipeline` |
| `cq-query-lowering` | CQ planning and RA construction | `written_order`, `clause_prefix`, `hypergraph`; each exports `compile`; `dependencies`, `model`, `proof` support planning |
| `cq-index-policy` | RA → IX; derive required join-column indexes and their uses | `compile` in `lib.rs`; `correspondence` checks the edge |
| `cq-physical-selection` | IX plus explicit choices → Physical | `given_bindings` in `lib.rs`; no provider-name dispatch |
| `cq-physical-lowering` | Realization driver and independently selected operator rules | `begin`, `resume` schedule calls; `rename`, `indexed_join`, `project` lower operators through selected providers; no concrete provider dependency |
| `cq-example-providers` | External storage, output and execution-state adapters | `storage` establishes slice/maps; `exact` emits Rows/Matches/Output fragments; `layout` decodes declared fields; `state` implements hash/ordered distinct state and exact-set sinks; no operator lowering |
| `cq-access-lowering` | Existing Access execution implementations | `to_pipeline`, `to_set_loops` |
| `cq-pipeline-lowering` | Existing Pipeline → Rust endpoints | `distinct_iter`, `collect_set`, `support_iter`, `exists`; shared `operators` emits adapters |
| `cq-storage` | Runtime data structures and operation traits | Blocks, selections, summaries, hash indexes, replay and counters; no RA interpreter or provider policy |
| `mini-linq` | Procedural macro facade and caller-resolved continuations | `lib.rs` exports entries; `entry` transports typed stages; `workload` captures catalog CQ |

`mini-linq` and `cq-example-providers` are procedural macro crates. The lowering
crates are ordinary Rust libraries with typed APIs; an individual pass does not
need its own proc-macro crate. The realizer and concrete providers both depend
on the protocol in `cq-ir`; the realizer does not depend on the provider crate.
The provider is a development dependency of the facade for integration tests,
not a production dependency that selects a backend. Provider protocol tests use
the physical-lowering library as a test-only dependency. Shared read-only AST
lookups live in `cq-ir::physical::lookup`; concrete field decoding stays in providers.

The `mini-linq` facade exports `rename!`, `indexed_join!`, `project!` and their
callbacks under `physical-realization`. Their implementations live in
`cq-physical-lowering`. They are explicitly selected rules, not driver defaults;
other rules can still be supplied by external macro paths.

`mini-linq-course` is the workspace/example root, and `cq-workloads` is a typed
query/data catalog. Neither is a compilation stage. The separate `xtask`
workspace contains development tooling.

## Explicit composition boundaries

| Caller-resolved path | Boundary | Current binding example |
|---|---|---|
| `mini_linq::choose_index_policy!` | RA → IX | Import `index_demands` under this name |
| `mini_linq::choose_physical!` | IX → Physical | Caller policy supplies assignments to `given_physical_bindings!` |
| `mini_linq::realize_physical!` | Physical → Rust[Access] | Import the facade's `realize_physical` implementation |
| `mini_linq::access!` | Consume the completed Access region | Explicitly import `access_to_set_loops`, or a compatible selected endpoint |

A policy that appends a table is actual selection logic. A plain renaming uses
`use ... as ...`; it requires no forwarding macro. The names of relation/index
providers and RA lowerers are independently recorded in Physical. Selecting one
provider for `r` does not change the provider for `s`.

Realization uses driver Input/Index/Lower requests and typed replies. Its
`resume_physical!` callback is an administrative continuation, not another IR
stage. The selected join rule may call its operands' Rows/Matches operations
and the selected result provider's Output operation. These calls disappear into
Rust/Access during realization; there is no retained Native language.

## Current execution domain

The [composition example](../examples/composition/README.md) supplies a complete
RA plan over a slice, `HashMap<K, Vec<V>>`, and `BTreeMap<K, Vec<V>>`. Providers
check declared Rust types, establish borrowed aliases, and reuse the maps'
physical first-column indexes after translating Rename's logical ordinals.
Join results are Access syntax with borrowed field expressions. A left result
region is composed into its consumer; it is not treated as stored rows.

The final region must expose distinct logical names in sorted heading order,
and current output metadata must use that same order. Its physical carrier can
be a tuple of borrowed fields or another explicitly selected lawful carrier.
A selected terminal Project retains its source Access clauses and asks the
selected output provider to construct the projected carrier. Intermediate
projection rejects because it needs explicit handling of dropped logical names.
No implicit collection, clone, scan fallback, or alternate provider is inserted.

The RA/IX languages are broader than this provider family. YA/block pruning,
index construction over intermediate regions, general materialization,
Select/Extend, nonterminal projection, and generated host APIs remain migration work. The
placement of push/pull selection is still an open design question. Existing
Access/Pipeline endpoints remain independently testable without deciding it.

The [full-CQ test](../crates/mini-linq/tests/physical_query.rs) exercises the
written-order planner through its terminal Project; its
[query trace](ix-physical/query-trace/review.md) is separate from the
[direct-RA trace](ix-physical/realization-trace/review.md).

## Execution state selection

Distinct and output collection use caller-bound `mini_linq::distinct_state!` and
`mini_linq::set_sink!` slots. There is no HashSet fallback in Access/Pipeline
lowering. Each request carries its local operator name; a caller policy can
choose separately for each occurrence. The semantic Pipeline is unchanged.
State provider syntax and laws live in `cq-ir::provider_protocol::state`.

`cq-example-providers` supplies hash and ordered implementations. Distinct
providers own retained copies; set sinks consume values and need no Clone.
The execution lowerer owns polling and loop order. Support/existence omit the
terminal Distinct and need neither slot. No execution strategy copies a
binding: a value bound by an outer source is used inside its dependent joins
and inner loops as the value the source produced. Hosts and built structures
yield references, so those are free; an owned value (a computed column) that
one outer row must hand to several inner rows is a Rust move error, and the
head expression alone decides what the result owns. The remaining copies are
the ones a type imposes: probing a host map keyed by owned tuples constructs
the key, and a distinct provider retains its own copy of each row.
