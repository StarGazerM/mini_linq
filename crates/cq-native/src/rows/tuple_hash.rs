//! Heterogeneous Rust tuple rows in a native HashSet.
use cq_ir::native::Operation;
use proc_macro2::TokenStream;
use quote::quote;

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub work: Option<syn::Expr>,
}

pub fn compile(operation: &Operation, options: &Options) -> syn::Result<TokenStream> {
    let set = quote!(::std::collections::HashSet);
    if let Some(syntax) = super::schema::tuple(operation, &set) {
        return Ok(syntax);
    }
    super::native::compile(operation, set, options.work.as_ref())
}
