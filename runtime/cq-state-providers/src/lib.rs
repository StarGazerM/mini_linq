//! Selectable distinct-state implementations.
//!
//! The pull printer requests `new` and `insert` through `cq-protocol` when
//! deduplicating canon pipe or alias streams. Inferred-canonical values and
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
