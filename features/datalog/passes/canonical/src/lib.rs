//! Canonical-layout order selection for ordered index kinds.
//!
//! Storage's default canonical layout suits hash layouts: a derived, returned
//! or negated relation without a read binding every column receives one more
//! layout, keyed by every column in declaration order, for admission alone.
//! A hash layout answers membership only through its own key, so that layout
//! cannot be avoided there.
//!
//! An ordered index kind stores every full layout as the complete rows in the
//! layout's column order (sorted runs, as on the device). Any such layout
//! answers membership, so the canonical layout need not have an order of its
//! own. `canonical(ordered)` gives a canonical layout that no occurrence reads
//! the column order of one of the relation's read layouts, preferring one
//! read as a full version inside the relation's own component (the order a
//! recursive component keeps maintained anyway). A realization that shares
//! one state between layouts of equal order then stores nothing for it.
//!
//! Only the canonical layout's column order and its respelled provider type
//! change. Layout count, keys, reads, maintenance and every other relation
//! are preserved; canonical consumers already read through the recorded
//! permutation. The rewrite is admissible for hash layouts too, where it is
//! merely useless.
//!
//! ```ignore
//! pipeline {
//!     storage_rewrites = [cq_datalog_canonical::canonical(ordered), /* realization */];
//! }
//! ```
mod rewrite;

/// Select each unread canonical layout's order among the relation's read
/// orders (rule DL-CanonicalOrder). The only index kind is `ordered`.
#[proc_macro]
pub fn canonical(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(input.into(), |source, args| {
        rewrite::storage(source, syn::parse2(args)?)
    })
    .into()
}
