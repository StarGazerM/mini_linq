use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::Parse;

/// Input relation names bind Native macros in the caller's `inputs` namespace.
/// Keeping this separate from stage bindings leaves all RA names available.
/// This constructs symbolic paths; Rust resolves their implementations.
#[cfg(feature = "borrowed-nested")]
pub(crate) fn relation_macros(
    source: &cq_ir::relational_plan::Module,
) -> std::collections::BTreeMap<String, syn::Path> {
    source
        .program
        .inputs()
        .map(|input| {
            let name = &input.name;
            (
                cq_ir::symbol_name(name),
                syn::parse_quote!(mini_linq::inputs::#name),
            )
        })
        .collect()
}

pub(crate) fn stage<S: Parse, T: ToTokens>(
    input: TokenStream,
    next: Option<syn::Path>,
    lower: impl FnOnce(&S) -> syn::Result<T>,
) -> TokenStream {
    let source = syn::parse_macro_input!(input as S);
    match lower(&source) {
        Ok(target) => match next {
            Some(next) => quote!(#next! { #target }).into(),
            None => quote!(#target).into(),
        },
        Err(error) => error.into_compile_error().into(),
    }
}
