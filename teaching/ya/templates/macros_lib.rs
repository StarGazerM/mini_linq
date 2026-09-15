//! Rust macro imports select the continuations between existing IRs.
use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};

fn stage<S: Parse, T: ToTokens>(
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

#[proc_macro]
pub fn planned_query(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::planned_relational_plan)),
        cq_compiler::compile_cq_graph,
    )
}

#[proc_macro]
pub fn planned_relational_plan(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::materialized)),
        cq_compiler::compile_result_indexes,
    )
}

mod kw {
    syn::custom_keyword!(access);
}
struct AccessInput<S> {
    access: syn::Path,
    source: S,
}
impl<S: Parse> Parse for AccessInput<S> {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let access = if input.peek(kw::access) {
            input.parse::<kw::access>()?;
            let path = input.parse()?;
            input.parse::<syn::Token![;]>()?;
            path
        } else {
            syn::parse_quote!(mini_linq::collect_access)
        };
        Ok(Self {
            access,
            source: input.parse()?,
        })
    }
}

#[proc_macro]
pub fn materialized(input: TokenStream) -> TokenStream {
    stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_compiler::compile_materialized_with(&input.source, &input.access)
        },
    )
}

/// The same selected computation inside an anonymous Rust closure.
#[proc_macro]
pub fn materialized_query(input: TokenStream) -> TokenStream {
    stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_compiler::compile_materialized_callable(&input.source, &input.access)
        },
    )
}

#[proc_macro]
pub fn pull(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::iterator_pipeline)),
        cq_compiler::compile_pull,
    )
}

#[proc_macro]
pub fn collect_access(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::collect_pipeline)),
        cq_compiler::compile_pull,
    )
}

#[proc_macro]
pub fn iterator_pipeline(input: TokenStream) -> TokenStream {
    stage(input, None, cq_compiler::compile_iterator_pipeline)
}

#[proc_macro]
pub fn collect_pipeline(input: TokenStream) -> TokenStream {
    stage(input, None, cq_compiler::compile_collect_pipeline)
}
