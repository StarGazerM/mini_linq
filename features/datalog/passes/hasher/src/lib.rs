//! Opt-in hash-provider selection; the default Datalog pipeline is unchanged.
//!
//! `hasher(fx)` rewrites Storage's native hash layout maps, IDB seen sets,
//! and EDB canonical sets. `hasher(default)` restores the base `cq_storage`
//! hash choices. Unused relation-kind fields, ordered/custom providers, row
//! buffers, bindings, maintenance, and all logical/execution decisions are
//! preserved. Only qualified `std::collections::HashMap`,
//! `std::collections::HashSet`, `cq_storage::HashSet`, and
//! `cq_datalog_parallel_runtime::ShardedSeen` types are recognized (an initial
//! `::` is optional); aliases and nested types are deliberately opaque.
//!
//! `ShardedSeen<T>` and `ShardedSeen<T, S>` are supported, so Storage selection
//! may precede or follow sharded Storage selection. Default selection restores
//! `ShardedSeen<T>`. The runtime must support its generic hasher parameter.
//!
//! Negation key sets are recorded only after fork. Pair the Storage rewrite
//! with `hasher_round(fx)` in the Round queue to select those sets as well.
//! Apply it before converting Round to a specialized parallel language.
//! `hasher_round(default)` restores their base provider, without changing
//! the embedded Storage program or the CQ hole pipelines.
//!
//! CQ hole dedup providers are a separate Structures decision. To select all
//! three layers, use the following explicit configuration and depend directly
//! on `rustc-hash = "=2.1.3"` and `cq-rewrites` in the consuming crate:
//!
//! ```ignore
//! pipeline {
//!     storage_rewrites = [cq_datalog_hasher::hasher(fx)];
//!     round = [cq_datalog_hasher::hasher_round(fx)];
//!     cq = pipeline {
//!         structures = [cq_rewrites::retype(dedup =>
//!             ::std::collections::HashSet<_,
//!                 ::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>>)];
//!     };
//! }
//! ```
//!
//! Fx hashing is deterministic and not collision-attack resistant: use it for
//! trusted workloads, not adversarial keys. Equality/deduplication semantics
//! remain unchanged; hash iteration order is not an ordering guarantee.
mod rewrite;

/// Select `fx` or `default` hash providers at the checked Storage boundary.
#[proc_macro]
pub fn hasher(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(input.into(), |source, args| {
        rewrite::storage(source, syn::parse2(args)?)
    })
    .into()
}

/// Select `fx` or `default` negation key sets at the checked Round boundary.
#[proc_macro]
pub fn hasher_round(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(input.into(), |source, args| {
        rewrite::round(source, syn::parse2(args)?)
    })
    .into()
}
