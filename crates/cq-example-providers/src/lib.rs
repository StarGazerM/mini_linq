//! External exact-index providers and binary-join lowering rules.
//!
//! This package is a client of the physical realization protocol. The common
//! realizer does not depend on it or interpret the names exported here.
//! Inputs, indexes, and output fields borrow their host data throughout.

use proc_macro::TokenStream;

mod composition;
mod exact;
mod layout;
mod project;
mod rules;
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
pub fn rename(input: TokenStream) -> TokenStream {
    expand(input, rules::rename)
}

#[proc_macro]
pub fn indexed_join(input: TokenStream) -> TokenStream {
    expand(input, rules::indexed_join)
}

/// Project logical columns through the explicitly selected output provider.
#[proc_macro]
pub fn project(input: TokenStream) -> TokenStream {
    expand(input, project::lower)
}

#[doc(hidden)]
#[proc_macro]
pub fn project_rows(input: TokenStream) -> TokenStream {
    expand(input, project::rows)
}

#[doc(hidden)]
#[proc_macro]
pub fn project_output(input: TokenStream) -> TokenStream {
    expand(input, project::output)
}

#[doc(hidden)]
#[proc_macro]
pub fn indexed_join_rows(input: TokenStream) -> TokenStream {
    expand(input, rules::indexed_join_rows)
}

#[doc(hidden)]
#[proc_macro]
pub fn indexed_join_matches(input: TokenStream) -> TokenStream {
    expand(input, rules::indexed_join_matches)
}

#[doc(hidden)]
#[proc_macro]
pub fn indexed_join_output(input: TokenStream) -> TokenStream {
    expand(input, rules::indexed_join_output)
}
