//! The default RA route chooses its index policy and physical continuation once.
use cq_ir::{index_requirements as ix, relational_plan as ra};
use proc_macro2::TokenStream;
use quote::quote;

/// Select the default index policy and invoke its corresponding continuation.
/// Callers can inspect IX before invoking a selected typed or macro continuation.
pub fn with_default_indexes<T>(
    source: &ra::Module,
    input: impl FnOnce(ix::Module) -> syn::Result<T>,
    lazy: impl FnOnce(ix::Module) -> syn::Result<T>,
) -> syn::Result<T> {
    ra::contract::check(source)?;
    let result_keys = !source.plan.output.head.is_normalized()
        || source.plan.definitions.iter().any(|d| {
            matches!(
                d.operator,
                ra::Operator::Select(_) | ra::Operator::Extend(_) | ra::Operator::Unit(_)
            )
        });
    if result_keys {
        lazy(cq_index::result::compile(source)?)
    } else {
        input(cq_index::input::compile(source)?)
    }
}

/// Expand the selected index policy with its caller-supplied physical continuation.
/// The resulting native macro invocation carries IX, with no serialized route tag.
pub fn expand_indexed(
    source: &ra::Module,
    input: &syn::Path,
    lazy: &syn::Path,
) -> syn::Result<TokenStream> {
    with_default_indexes(
        source,
        |indexes| Ok(quote!(#input! { #indexes })),
        |indexes| Ok(quote!(#lazy! { #indexes })),
    )
}

/// Compile the same default pair for typed callers, retaining IX for inspection.
/// Independently authored IX is compiled through an explicit physical entry.
pub fn compile_indexed(
    source: &ra::Module,
    access: &syn::Path,
) -> syn::Result<(ix::Module, syn::File)> {
    with_default_indexes(
        source,
        |indexes| {
            let program = cq_physical::input::compile_input_indexed(&indexes, access)?;
            Ok((indexes, program))
        },
        |indexes| {
            let program = cq_physical::lazy::compile_lazy_indexed(&indexes, access)?;
            Ok((indexes, program))
        },
    )
}
