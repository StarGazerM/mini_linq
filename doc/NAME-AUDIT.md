# Compiler names and macro ownership

Names identify syntax, a typed transformation, or an executable implementation.
A name alone does not establish semantic compatibility. The
[crate/module map](COMPILER-STRUCTURE.md) gives ownership; the tables here list
the current exports and import slots.

## Caller slots are not extra wrapper macros

| Name emitted by a pass | Role | Example binding |
|---|---|---|
| `mini_linq` | Caller facade module name | `use wiring as mini_linq` |
| `choose_index_policy!` | RA → IX slot | `use ::mini_linq::index_demands as choose_index_policy` |
| `choose_physical!` | IX → Physical policy slot | Caller policy appends explicit assignments and invokes `given_physical_bindings!` |
| `realize_physical!` | Physical realization slot | Import the facade implementation of the same name |
| `access!` | Completed Access consumer slot | Import the desired Access endpoint |

`query!` is not a library export. A caller can use that name as an ordinary
import alias. Per-relation and per-index provider paths live in Physical; they
are not a single global `provider!` slot. Plain aliasing needs no forwarding
macro. A macro that adds a physical table performs policy work.

## `mini-linq` procedural macros

All entries below are defined in `crates/mini-linq/src/lib.rs`.

| Exports | Meaning / typed implementation |
|---|---|
| `workload_query!` | Capture a typed CQ factory for the workload catalog; `mini-linq::workload` |
| `mini_linq!`, `choose_graph_or_prefix!` | Frontend planner selectors; dispatch through caller-relative planner paths |
| `cq_to_ra_written_order!` | CQ → RA; `cq-query-lowering::written_order::compile` |
| `cq_to_ra_clause_prefix!` | CQ → RA; `cq-query-lowering::clause_prefix::compile` |
| `cq_to_ra_hypergraph!` | CQ → planned RA, including logical YA; `cq-query-lowering::hypergraph::compile` |
| `index_demands!` | RA → IX; `cq-index-policy::compile` |
| `given_physical_bindings!` | IX plus supplied assignments → Physical; `cq-physical-selection::given_bindings` |
| `realize_physical!`, `resume_physical!` | Begin/resume one Physical realization; `cq-physical-lowering::begin/resume` |
| `access_to_distinct_iter!`, `access_to_set!`, `access_to_support_iter!`, `access_to_exists!` | Existing Access → Pipeline implementation with the named endpoint continuation; `cq-access-lowering::to_pipeline` |
| `access_to_set_loops!` | Existing Access → eager Rust set loops; `cq-access-lowering::to_set_loops` |
| `pipeline_to_distinct_iter!`, `pipeline_to_set!`, `pipeline_to_support_iter!`, `pipeline_to_exists!` | Pipeline → Rust; `cq-pipeline-lowering::{distinct_iter,collect_set,support_iter,exists}` |

`resume_physical!` is an administrative callback. It is not a separate semantic
stage or another public query notation. The `access_to_*` names are existing
execution endpoints, not storage-selection or join-planning macros.

These selected physical rules are also exported by `mini-linq`, under
`physical-realization`; their callbacks are hidden implementation entries.

| Exports | Implementation module | Admitted role |
|---|---|---|
| `rename!` | `cq-physical-lowering::rename::compile` | Selected direct-input Rename rule; asks the result provider to establish its representation |
| `indexed_join!` | `cq-physical-lowering::indexed_join::compile` | Selected exact binary NaturalJoin rule; consumes IX and operand operations |
| `project!` | `cq-physical-lowering::project::lower` | Selected terminal Project rule; retains clauses and asks the recorded result provider for Output |
| `project_rows!`, `project_output!` | `cq-physical-lowering::project::{rows,output}` | Internal callbacks of terminal projection |
| `indexed_join_rows!`, `indexed_join_matches!`, `indexed_join_output!` | `cq-physical-lowering::indexed_join::{rows,matches,output}` | Internal callbacks of that selected rule |


## `cq-example-providers` procedural macros

All exports are defined in `crates/cq-example-providers/src/lib.rs`.

| Exports | Implementation module | Admitted role |
|---|---|---|
| `slice_rows!` | `storage::slice`, `exact::slice_rows` | Borrow one-column slices; enumerate Rows |
| `hash_relation!` | `storage::hash`, `exact` | Borrow a two-column HashMap relation; reuse its index; Rows/Matches |
| `sorted_relation!` | `storage::sorted`, `exact` | Borrow a two-column BTreeMap relation; reuse its index; Rows/Matches |
| `borrowed_fields!` | `exact::borrowed_fields` | Construct a tuple of interpreted borrowed fields for Output |

The same provider crate also exports `hash_distinct!`, `ordered_distinct!`,
`hash_set_sink!`, and `ordered_set_sink!`, implemented in `state`. Callers bind
them to the `distinct_state` or `set_sink` slot. Their typed expression requests
are defined in `cq-ir::provider_protocol::state`.

The shared envelope and exact-operation family are typed in
`cq-ir::provider_protocol`; operator continuation records are private to
`cq-physical-lowering::syntax`; `cq-example-providers::syntax` parses only
representation requests. `RustPath` in the paper means a Rust module/name
path to an external macro. It is neither a provider tag nor an enum case.

The former whole-query hash/configured lowerers, Native macros, and `cq-compose`
are removed. Their names are not compatibility aliases. Independent storage
traits remain in `cq-storage`; the current map/slice example uses ordinary
standard-library collections directly.
