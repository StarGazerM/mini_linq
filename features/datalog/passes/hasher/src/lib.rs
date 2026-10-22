//! Hash-provider selection for Datalog, whose default providers use Fx hashing.
//!
//! `hasher(keyed)` opts Storage's native hash layout maps, IDB seen sets,
//! and EDB canonical sets into the keyed `cq_storage` hash providers.
//! `hasher(fx)` explicitly selects the default Fx providers. Unused
//! relation-kind fields, ordered/custom providers, row buffers, bindings,
//! maintenance, and all logical/execution decisions are preserved.
//! Only qualified `std::collections::HashMap`, `std::collections::HashSet`,
//! `cq_storage::HashSet`, and `cq_datalog_parallel_runtime::ShardedSeen`
//! types are recognized (an initial `::` is optional); aliases and nested
//! types are deliberately opaque.
//!
//! `ShardedSeen<T>` and `ShardedSeen<T, S>` are supported, so Storage selection
//! may precede or follow sharded Storage selection. Keyed selection produces
//! `ShardedSeen<T>`. The runtime must support its generic hasher parameter.
//!
//! Negation key sets are recorded only after fork. Pair the Storage rewrite
//! with `hasher_round(keyed)` in the Round queue to opt those sets out of Fx
//! as well. Apply it before converting Round to a specialized parallel
//! language. `hasher_round(fx)` explicitly selects Fx without changing the
//! embedded Storage program or the CQ hole pipelines.
//!
//! CQ holes discharge result canonicalization to admission and negation key
//! sets, so they have no separate answer-dedup provider to select. To opt these
//! canonical consumers and storage into keyed hashing, depend directly on
//! `cq-storage` and use:
//!
//! ```ignore
//! pipeline {
//!     storage_rewrites = [cq_datalog_hasher::hasher(keyed)];
//!     round = [cq_datalog_hasher::hasher_round(keyed)];
//! }
//! ```
//!
//! Fx hashing is deterministic and not collision-attack resistant: the default
//! is suitable for trusted workloads, not adversarial keys. Fx types use the
//! `cq_datalog::rustc_hash` facade reexport. Equality and dedup
//! semantics remain unchanged; hash iteration order is not an ordering guarantee.
mod rewrite;

/// Select `fx` or `keyed` hash providers at the checked Storage boundary.
#[proc_macro]
pub fn hasher(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(input.into(), |source, args| {
        rewrite::storage(source, syn::parse2(args)?)
    })
    .into()
}

/// Select `fx` or `keyed` negation key sets at the checked Round boundary.
#[proc_macro]
pub fn hasher_round(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(input.into(), |source, args| {
        rewrite::round(source, syn::parse2(args)?)
    })
    .into()
}
