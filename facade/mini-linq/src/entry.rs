use proc_macro::TokenStream;
use quote::ToTokens;
use syn::parse::Parse;

pub(crate) use cq_stage::Policed;

// The proc-macro ABI conversion is the facade's only entry-point adapter.
pub(crate) fn stage<S: Parse, T: ToTokens>(
    input: TokenStream,
    next: Option<syn::Path>,
    lower: impl FnOnce(&S) -> syn::Result<T>,
) -> TokenStream {
    cq_stage::stage(input.into(), next, lower).into()
}
