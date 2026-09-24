use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::Parse;

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

/// A language payload followed by the caller's appended policy block.
pub(crate) struct Policed<L, P> {
    pub language: L,
    pub policy: P,
}

impl<L: Parse, P: Parse> Parse for Policed<L, P> {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            language: input.parse()?,
            policy: input.parse()?,
        })
    }
}
