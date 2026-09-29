//! Consumer-owned macro composition, using the same stage ABI as mini-linq.
use cq_lang_physical as physical;
use cq_lang_trace::{Section, Staged};
use cq_stage::Policed;
use proc_macro::TokenStream;
use quote::quote;

#[proc_macro]
pub fn enter(input: TokenStream) -> TokenStream {
    cq_stage::stage(
        input.into(),
        Some(syn::parse_quote!(trace_bridge::leave)),
        |source: &Policed<physical::Module, physical::policy::Policy>| {
            let trace = physical_to_trace::compile(&source.language.section)?;
            let prefix = &source.language;
            let policy = &source.policy;
            Ok(quote!(#prefix #trace #policy))
        },
    )
    .into()
}
#[proc_macro]
pub fn leave(input: TokenStream) -> TokenStream {
    cq_stage::stage(
        input.into(),
        Some(syn::parse_quote!(::mini_linq::select_structures)),
        |source: &Policed<Staged<physical::Module, Section>, physical::policy::Policy>| {
            let staged = Staged {
                prefix: source.language.prefix.section.clone(),
                section: source.language.section.clone(),
            };
            let section = trace_to_physical::compile(&staged)?;
            let target =
                physical::Module::new(source.language.prefix.index_requirements.clone(), section);
            let policy = &source.policy;
            Ok(quote!(#target #policy))
        },
    )
    .into()
}
