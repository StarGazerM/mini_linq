//! Consumer-owned macro composition, using the same stage ABI as mini-linq.
use cq_lang_physical as physical;
use cq_stage::Policed;
use proc_macro::TokenStream;
use quote::quote;

#[proc_macro]
pub fn enter(input: TokenStream) -> TokenStream {
    cq_stage::stage(
        input.into(),
        Some(syn::parse_quote!(trace_bridge::leave)),
        |source: &Policed<physical::Program, physical::policy::Policy>| {
            let trace = physical_to_trace::compile(&source.language)?;
            let policy = &source.policy;
            Ok(quote!(#trace #policy))
        },
    )
    .into()
}
#[proc_macro]
pub fn leave(input: TokenStream) -> TokenStream {
    cq_stage::stage(
        input.into(),
        Some(syn::parse_quote!(::mini_linq::select_structures)),
        |source: &Policed<cq_lang_trace::Program, physical::policy::Policy>| {
            let target = trace_to_physical::compile(&source.language)?;
            let policy = &source.policy;
            Ok(quote!(#target #policy))
        },
    )
    .into()
}
