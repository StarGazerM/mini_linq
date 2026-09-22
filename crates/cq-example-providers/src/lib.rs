//! External exact-index and output-representation providers.
//!
//! This package is a client of the physical realization protocol. The common
//! realizer does not depend on it or interpret the names exported here.
//! Inputs, indexes, and output fields borrow their host data throughout.

use proc_macro::TokenStream;

mod exact;
mod layout;
mod state;
mod storage;
mod syntax;

#[cfg(test)]
mod tests;

fn expand(
    input: TokenStream,
    f: impl FnOnce(proc_macro2::TokenStream) -> syn::Result<proc_macro2::TokenStream>,
) -> TokenStream {
    f(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn slice_rows(input: TokenStream) -> TokenStream {
    expand(input, storage::slice)
}

#[proc_macro]
pub fn hash_relation(input: TokenStream) -> TokenStream {
    expand(input, storage::hash)
}

#[proc_macro]
pub fn sorted_relation(input: TokenStream) -> TokenStream {
    expand(input, storage::sorted)
}

#[proc_macro]
pub fn borrowed_fields(input: TokenStream) -> TokenStream {
    expand(input, exact::borrowed_fields)
}

#[proc_macro]
pub fn hash_distinct(input: TokenStream) -> TokenStream {
    expand(input, state::hash_distinct)
}

#[proc_macro]
pub fn ordered_distinct(input: TokenStream) -> TokenStream {
    expand(input, state::ordered_distinct)
}

#[proc_macro]
pub fn hash_set_sink(input: TokenStream) -> TokenStream {
    expand(input, state::hash_set_sink)
}

#[proc_macro]
pub fn ordered_set_sink(input: TokenStream) -> TokenStream {
    expand(input, state::ordered_set_sink)
}
