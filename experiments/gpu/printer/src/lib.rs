//! Checked Structures → CUDA tile streams. Storage choices are never inferred
//! from query names: keyed reads use their recorded SortedRun, PrefixRun, or
//! exact integral membership provider;
//! each canon requires an explicitly recorded `cq_gpu_runtime::DeviceSet<_>`.
//! The result is a typed, owning `cq_gpu_runtime::DeviceColumns<Row, Codec>`:
//! pass it to another compiled query without a transfer, or call `.to_host()`
//! explicitly. A bound `DeviceColumns` supplies its own immutable columns,
//! dictionary and codec facts; a bound Vec or slice retains the upload path.
//! Neither host codecs nor capacity advice establish canonicality.
//! Fixed full-tile scans decode each 1024-value vector into four register
//! values per lane once, then consume those values across the four stripes.
//! Short recorded batches and resumable scans use scalar device decode without
//! materializing a decompressed global column.
//!
//! Derived columns use the codec recorded by their Structures provider.
//! `DeviceSet<_>` and retained `Vec<_>` record Plain; an explicit
//! `retype(dedup => ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::For>)`
//! records packed answers. Retained rows can likewise record
//! `cq_gpu_runtime::DeviceColumns<_, Codec>`. Supported codec markers are
//! `Plain`, `For`, `Delta` and `DictionaryCodec`; unknown aliases are rejected.
//! Unsupported operators and representations fail before CUDA is emitted.

use proc_macro::TokenStream;

mod compiler;
mod options;
mod predicate;
mod prefix;

/// Terminal pipeline pass. Optional control arguments are `tile(256,4)`,
/// `frontier(8)` (zero disables publishing), `directory(rank)`, and
/// `capacity(InputRelation)` for an explicit input-cardinality allocation bound.
/// Capacity advice does not assert key uniqueness or change relational meaning;
/// exceeding it reports an error instead of dropping answers. Without advice,
/// bounded output grows by resuming saved device execution state.
/// Data/evaluation decisions come exclusively from checked Structures.
#[proc_macro]
pub fn print(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_structures::Program, syn::Expr>(input.into(), |source, arguments| {
        compiler::compile(source, syn::parse2(arguments)?)
    })
    .into()
}

/// Explicit Structures rewrite selecting a rank-backed prefix representation.
/// Remaining recorded key levels are exact residual predicates, not discarded.
/// `prefix_runs(Supplier => supplier_key)` explicitly selects a singleton level
/// by base relation or retained result. The selected column's layout index is
/// recorded in PrefixRun's fourth const argument and must match its first level.
#[proc_macro]
pub fn prefix_runs(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), prefix::compile).into()
}

#[cfg(test)]
mod tests;
