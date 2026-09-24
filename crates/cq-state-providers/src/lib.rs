//! Selectable distinct-state and set-sink implementations.
//!
//! Execution endpoints request `new`, `insert`, and `finish` through the
//! [`cq_ir::state_protocol`]; each macro here expands one request to a Rust
//! expression over a standard collection. Selecting a path is the caller's
//! choice; the compiler supplies no default.

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

#[proc_macro]
pub fn hash_set_sink(input: TokenStream) -> TokenStream {
    expand(input, state::hash_set_sink)
}

#[proc_macro]
pub fn ordered_set_sink(input: TokenStream) -> TokenStream {
    expand(input, state::ordered_set_sink)
}
