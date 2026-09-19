use cq_ir::{provider_protocol as protocol, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Ident, Path};

use crate::{
    exact, layout,
    syntax::{ProviderRequest, ProviderTask},
};

pub fn slice(tokens: TokenStream) -> syn::Result<TokenStream> {
    let request: ProviderRequest = syn::parse2(tokens)?;
    match &request.request {
        ProviderTask::Input(protocol::Task::Input { name, source, .. }) => {
            let types = layout::physical_types(&request.plan, name)?;
            let [element] = types.as_slice() else {
                return Err(syn::Error::new_spanned(
                    name,
                    "slice_rows stores exactly one logical column",
                ));
            };
            let value = format_ident!("__cq_store_{}", name, span = Span::mixed_site());
            let operations = layout::relation_provider(&request.plan, name)?;
            let callback = &request.reply_to;
            let state = &request.state;
            Ok(quote! { #callback! {
                resume { #state }
                emitted { let #value: &[#element] = #source; }
                result { stored #name at (#value) operations #operations }
            } })
        }
        ProviderTask::Rows(_) => exact::slice_rows(&request),
        _ => Err(syn::Error::new_spanned(
            &request.request,
            "slice_rows supports Input and Rows; an index requires a separately selected implementation",
        )),
    }
}

pub fn hash(tokens: TokenStream) -> syn::Result<TokenStream> {
    map(
        syn::parse2(tokens)?,
        syn::parse_quote!(::std::collections::HashMap),
    )
}

pub fn sorted(tokens: TokenStream) -> syn::Result<TokenStream> {
    map(
        syn::parse2(tokens)?,
        syn::parse_quote!(::std::collections::BTreeMap),
    )
}

/// Shared Rust emission for the two external map examples. The common
/// realizer does not call this helper or know either collection type.
fn map(request: ProviderRequest, collection: Path) -> syn::Result<TokenStream> {
    match &request.request {
        ProviderTask::Input(protocol::Task::Input { name, source, .. }) => {
            let types = layout::physical_types(&request.plan, name)?;
            let [column, payload] = types.as_slice() else {
                return Err(syn::Error::new_spanned(
                    name,
                    "this map provider stores exactly two logical columns",
                ));
            };
            let value = format_ident!("__cq_store_{}", name, span = Span::mixed_site());
            let operations = layout::relation_provider(&request.plan, name)?;
            let callback = &request.reply_to;
            let state = &request.state;
            Ok(quote! { #callback! {
                resume { #state }
                emitted { let #value: &#collection<#column, ::std::vec::Vec<#payload>> = #source; }
                result { stored #name at (#value) operations #operations }
            } })
        }
        ProviderTask::Index(protocol::Task::Index { name, .. }) => {
            let index = map_index(&request, name)?;
            let relation = &index.requirement.relation;
            let (source, _) = layout::stored(&request.environment, relation)?;
            let types = layout::physical_types(&request.plan, relation)?;
            let [column, payload] = types.as_slice() else {
                unreachable!("map_index checked arity")
            };
            let value = format_ident!("__cq_index_{}", name, span = Span::mixed_site());
            let operations = layout::index_provider(&request.plan, name)?;
            let callback = &request.reply_to;
            let state = &request.state;
            Ok(quote! { #callback! {
                resume { #state }
                emitted { let #value: &#collection<#column, ::std::vec::Vec<#payload>> = #source; }
                result { indexed #name at (#value) operations #operations }
            } })
        }
        ProviderTask::Rows(_) => exact::map_rows(&request),
        ProviderTask::Matches(_) => exact::map_matches(&request),
        _ => Err(syn::Error::new_spanned(
            &request.request,
            "this map provider supports Input, Index, Rows, and Matches",
        )),
    }
}

pub fn map_index<'a>(
    request: &'a ProviderRequest,
    name: &Ident,
) -> syn::Result<&'a cq_ir::index_requirements::IndexDefinition> {
    let index = request
        .plan
        .index_requirements
        .indexes
        .iter()
        .find(|i| symbol_name(&i.name) == symbol_name(name))
        .ok_or_else(|| syn::Error::new_spanned(name, "unknown IX index identity"))?;
    let relation = &index.requirement.relation;
    let physical = layout::physical_columns(&request.plan, relation)?;
    let [column, _payload] = physical.as_slice() else {
        return Err(syn::Error::new_spanned(
            relation,
            "this map provider stores exactly two logical columns",
        ));
    };
    let heading = layout::heading(&request.plan, relation)?;
    let position = heading
        .iter()
        .position(|h| symbol_name(h) == symbol_name(column))
        .unwrap();
    if index.requirement.columns.len() != 1
        || index.requirement.columns[0].index as usize != position
    {
        return Err(syn::Error::new_spanned(
            &index.requirement,
            "selected map index can reuse only its physical first-column index, translated through Rename; select another provider for this IX demand",
        ));
    }
    Ok(index)
}
