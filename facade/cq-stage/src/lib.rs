//! Token-stream composition for compiler stages, usable by libraries and proc macros.
//!
//! [`stage`] parses one payload, runs a fallible transformation, then emits
//! either the result or a call to the caller-selected continuation macro.
//! Parsing and transformation errors become Rust `compile_error!` tokens.
//! No runtime, language, or compiler pass is selected by this crate.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::Parse;

/// Parse `S`, transform it to `T`, and optionally invoke `next` with `T`.
///
/// Proc-macro entry points convert their input and the returned stream using
/// `.into()`. Ordinary libraries can call this directly with `quote!` output.
///
/// ```
/// let output = cq_stage::stage::<syn::Expr, syn::Expr>(
///     quote::quote!(1 + 2), None, |expression| Ok(expression.clone()),
/// );
/// assert_eq!(output.to_string(), "1 + 2");
/// ```
pub fn stage<S: Parse, T: ToTokens>(
    input: TokenStream,
    next: Option<syn::Path>,
    lower: impl FnOnce(&S) -> syn::Result<T>,
) -> TokenStream {
    let source = match syn::parse2::<S>(input) {
        Ok(source) => source,
        Err(error) => return error.into_compile_error(),
    };
    match lower(&source) {
        Ok(target) => match next {
            Some(next) => quote!(#next! { #target }),
            None => quote!(#target),
        },
        Err(error) => error.into_compile_error(),
    }
}

/// A language payload followed by the caller's appended policy block.
pub struct Policed<L, P> {
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
