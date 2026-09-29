//! Selectable distinct-state implementations.
//!
//! The pull printer requests `new` and `insert` through `cq-protocol` when
//! deduplicating canonical plan streams. Inferred-canonical values and
//! materialized canonical structures need not invoke a distinct-state provider.
//! Each macro expands one request to a Rust expression over a standard collection.
//! Selecting a provider is the caller's choice; the compiler supplies no
//! default.

use proc_macro::TokenStream;

mod state;

fn expand(
    input: TokenStream,
    f: impl FnOnce(proc_macro2::TokenStream) -> syn::Result<proc_macro2::TokenStream>,
) -> TokenStream {
    f(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn hash_distinct(input: TokenStream) -> TokenStream {
    expand(input, state::hash_distinct)
}

#[proc_macro]
pub fn ordered_distinct(input: TokenStream) -> TokenStream {
    expand(input, state::ordered_distinct)
}

/// Default built-index types for the caller-bound `index_provider!` slot.
#[proc_macro]
pub fn default_indexes(input: TokenStream) -> TokenStream {
    expand(input, |tokens| {
        use syn::parse::Parser;
        let parser = |input: syn::parse::ParseStream<'_>| -> syn::Result<proc_macro2::TokenStream> {
            let kind: syn::Ident = input.parse()?;
            let ty = match kind.to_string().as_str() {
                "probe" => quote::quote!(::cq_storage::HashIndex<'_, _, _>),
                "semi" => quote::quote!(::std::collections::HashSet<_>),
                "ordered" => quote::quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>),
                "trie" => {
                    let inner: syn::Type = input.parse()?;
                    quote::quote!(::cq_storage::Trie<'_, _, #inner>)
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        kind,
                        "expected probe, semi, ordered, or trie Type",
                    ));
                }
            };
            if !input.is_empty() {
                return Err(input.error("unexpected provider arguments"));
            }
            Ok(ty)
        };
        parser.parse2(tokens)
    })
}
