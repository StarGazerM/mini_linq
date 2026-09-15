# Compiler structure and decision ownership

The compiler separates language definitions, checked transformations, and their
composition. Each selectable implementation owns its admitted domain and calls
an independent checker. Defaults choose among these entries in one place;
implementations do not call the default composition.

| Owner | Responsibility |
| --- | --- |
| `cq-ir` | CQ, named RA, index requirements, Access, Pipeline and provider-fragment syntax; local well-formedness |
| `mini-linq` | Parse the selected language and call a typed entry; emit the next macro invocation |
| `composition.rs` | Default CQ, index and physical routes; planned graph/prefix dispatch; complete program composition |
| `analysis/{linear,indexes}.rs` | Shared source facts: linear operands, headings and equality-key coverage; no route choice or generated Rust |
| `cq_to_relational/{canonical,prefix,graph}.rs` | Explicit CQ-to-RA implementations; each rejects inputs outside its own domain |
| `cq_to_relational/{rules,clause_rules}.rs` | Checked administrative transitions and their RA emission |
| `cq_to_relational/{hypergraph,strategy,ya}.rs` | Join-tree analysis, join choice and the YA up/down schedule |
| `cq_to_relational/check/` | Independent canonical, ordered-prefix, prefix-derivation and graph-derivation checks |
| `relational_to_indexes/{input,result}.rs` | Explicit input-column or named-result index selection |
| `relational_to_indexes/check/` | Independent checks for each index-selection edge |
| `physical/{input_indexed,lazy}.rs` | Explicit input-indexed and lazy result-indexed storage/access implementations |
| `physical/materialized.rs` | Checked materialized entries and native Rust host adapters |
| `physical/materialized/query.rs` | Shared indexed set computation, local Access regions and optional counter emission |
| `physical/materialized/semijoin.rs` | Exact semijoin scan and witness guard for the selected hash realization |
| `physical/check/` | Separate input-indexed, lazy and materialized correspondence traversals; shared Rust-syntax inspection |
| `physical/direct/` | Configured RA-to-Access lowering, layouts, versioned resources, wiring and its independent checker |
| `physical/access/{scan,keys,witnesses}.rs` | Candidate scans, exact/zone resources and join witnesses |
| `physical/providers/` | Native-set and block operations through public storage traits |
| `pull.rs`, `pull/check.rs` | Access-to-Pipeline implementation and independent correspondence |
| `pipeline.rs`, `pipeline/check.rs` | Pipeline-to-Rust endpoints and independent correspondence |
| `cq-storage` | Data structures, operation traits, masks, summaries and passive counters; no query execution |

Compiler paths are relative to `crates/cq-compiler/src/`. Crate names refer to
`crates/`. Checkers inspect actual targets without calling the emitter to obtain
an expected answer. Their placement beside the transformation does not remove
that independence. Compiler options and derivation witnesses in
`cq_to_relational/model.rs` are ordinary analysis data, not another IR.

## Explicit edges and default composition

`compile_cq_canonical`, `compile_cq_prefix` and `compile_cq_graph` select actual
implementations. Likewise, `compile_input_indexes` and `compile_result_indexes`
choose an index namespace; `compile_input_indexed`, `compile_lazy_indexed` and
`compile_materialized_with` choose physical realizations. The default entries
retain their convenience behavior in `composition.rs`. An explicit entry checks
its domain rather than switching to another implementation.

Macro continuation paths are relative to the invocation's `mini_linq` binding.
A module alias can therefore change a downstream edge or host. The selected
implementation must still satisfy the source, output, binding and observation
contracts of that position. A resolving macro path alone does not establish them.

For example, this composition selects the graph planner, result indexes and a
callable materialized host independently, using the same boundary names:

```rust
mod wiring {
    pub use ::mini_linq::{
        graph_query as mini_linq,
        result_indexes as relational_plan,
        materialized_query as index_requirements,
        collect_access, collect_pipeline,
    };
}
use wiring as mini_linq;
```

The [composition test](../crates/mini-linq/tests/composition.rs) runs the same CQ
through five such routes over all 256 two-relation databases on its small domain,
including duplicate-input variants. The [typed-boundary test](../crates/cq-compiler/tests/composition.rs)
also verifies explicit rejection of incompatible routes and independent replay
of graph and prefix derivations for the same source.

## Query computation and its host

`compile_materialized_query(&indexed, &access, &inputs)` returns a checked native
Rust expression. The input expressions are evaluated once in declaration order;
each supplies owned tuples through `IntoIterator`. Borrowed inputs can be bound
as `full.iter().cloned()` or `delta.iter().cloned()`. The expression normalizes
inputs, builds required indexes, computes named sets and returns an owned
`HashSet` of answer tuples. It emits no observation counters or generated types.

The same computation supplies the existing observed `Program`/`Storage` API.
`compile_materialized_callable` wraps the checked expression in an anonymous
Rust closure. The `materialized_query!` macro exposes that host using the existing
indexed-language input. Ordinary Rust calls supply its inputs; there is no
additional binding grammar or query IR. Each closure value has one inferred type
per input parameter. Its captures and repeatability follow the generated body's
ordinary Rust rules. The anonymous host introduces no item name that could shadow
functions referenced directly or through opaque query macros. Its wrapper and
query body have independent source-to-target checks.

For example, an existing planned query becomes a callable expression through this
one host binding:

```rust
mod wiring {
    pub use ::mini_linq::*;
    pub use ::mini_linq::materialized_query as materialized;
}
use wiring as mini_linq;
```

The [macro integration test](../crates/mini-linq/tests/reusable_query.rs) compiles
the same CQ in both hosts and checks repeated borrowed full/delta calls. The
[compiler tests](../crates/cq-compiler/tests/reusable_query.rs) additionally check
exact input evaluation, empty/composite semijoin keys and named non-`Copy` values.
The plain host requires no output ordering; sorted standalone materialization
adds its own `Ord` requirement. Reusing this computation does not yet implement
recursive program semantics, and each call still rebuilds this realization's
local sets and indexes.

## Extraction and remaining boundaries

The [YA teaching profile](../teaching/ya/README.md) selects a concrete dependency
closure and provides two separately replaceable algorithm files: YA scheduling
and semijoin-to-Access lowering. Its standalone build checks that unrelated
physical implementations are not required. Assignment boundaries select existing
implementations rather than introducing homework modules into the compiler.

The configured direct physical edge remains distinct. Provider choice is per
input/result, candidate and key choice per semijoin, and witness choice per join.
Its compiler-local Γ emits existing `Rust[Access]`; its providers retain different
operation laws. The reusable materialized host does not claim to abstract those
representations or to provide trie traversal or recursive rule-program semantics.
A new IR requires a new meaning, grammar and lowering obligation that existing
boundaries cannot express.

[Access refinement](ACCESS-REFINEMENT.md) gives the physical rules;
[the boundary review](IR-STAGE-REVIEW.md) inventories the languages and limits.
