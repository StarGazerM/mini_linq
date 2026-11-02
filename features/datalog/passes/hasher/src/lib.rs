//! Hash-provider selection for Datalog, whose default providers use Fx hashing.
//!
//! `hasher(keyed)` opts Storage's native hash layout maps, including each
//! relation's canonical layout, and its recorded negation key sets into the
//! keyed `cq_storage` hash providers. `hasher(fx)` explicitly selects the
//! default Fx providers. Ordered/custom providers, row buffers, bindings, the
//! canonical choice, maintenance, and all logical/execution decisions are
//! preserved.
//! Selection reads recorded provider tokens (A-126): every instance whose
//! declared token is `hashed` with a spelling function records the chosen
//! `hasher { fx | keyed }` and is respelled from its key/payload schema by its
//! token's spelling function; verbatim (authored/device) tokens stay opaque.
//! No Rust type path or generic argument is inspected. `hash_set`, `hash_map`
//! and `sharded_layout` are hashed, so Storage selection may precede or follow
//! sharded Storage selection.
//!
//! Storage records each negation key set's provider and hasher; fork copies
//! them into Round, so there is no separate Round hasher rewrite.
//!
//! CQ holes discharge result canonicalization to admission and negation key
//! sets, so they have no separate answer-dedup provider to select. To opt these
//! canonical consumers and storage into keyed hashing, depend directly on
//! `cq-storage` and use:
//!
//! ```ignore
//! pipeline {
//!     storage_rewrites = [cq_datalog_hasher::hasher(keyed)];
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
