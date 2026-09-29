//! Concrete equality-state representations.

use cq_protocol::{Request, Task};
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
    }
}
