//! Concrete equality-state and output-set representations.

use cq_ir::provider_protocol::state::{Request, Task};
use proc_macro2::TokenStream;
use quote::quote;

pub fn hash_distinct(tokens: TokenStream) -> syn::Result<TokenStream> {
    distinct(
        syn::parse2(tokens)?,
        syn::parse_quote!(::std::collections::HashSet),
    )
}

pub fn ordered_distinct(tokens: TokenStream) -> syn::Result<TokenStream> {
    distinct(
        syn::parse2(tokens)?,
        syn::parse_quote!(::std::collections::BTreeSet),
    )
}

fn distinct(request: Request, collection: syn::Path) -> syn::Result<TokenStream> {
    match request.task {
        Task::New { .. } => Ok(quote!(#collection::new())),
        Task::Insert { state, value, .. } => {
            Ok(quote!((#state).insert(::core::clone::Clone::clone(#value))))
        }
        task => Err(syn::Error::new_spanned(
            task,
            "distinct state supports only new and insert",
        )),
    }
}

pub fn hash_set_sink(tokens: TokenStream) -> syn::Result<TokenStream> {
    sink(
        syn::parse2(tokens)?,
        syn::parse_quote!(::std::collections::HashSet),
    )
}

pub fn ordered_set_sink(tokens: TokenStream) -> syn::Result<TokenStream> {
    sink(
        syn::parse2(tokens)?,
        syn::parse_quote!(::std::collections::BTreeSet),
    )
}

fn sink(request: Request, collection: syn::Path) -> syn::Result<TokenStream> {
    Ok(match request.task {
        Task::New { .. } => quote!(#collection::new()),
        Task::Insert { state, value, .. } => quote!({ (#state).insert(#value); }),
        Task::Finish { state, .. } => quote!(#state),
    })
}
