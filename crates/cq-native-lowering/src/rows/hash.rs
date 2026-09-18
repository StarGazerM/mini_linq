//! Native HashSet row storage and its complete operation protocol.
use cq_ir::native::Operation;
use proc_macro2::TokenStream;
use quote::quote;

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub work: Option<syn::Expr>,
}

pub fn compile(operation: &Operation, options: &Options) -> syn::Result<TokenStream> {
    let set = quote!(::std::collections::HashSet);
    if let Some(syntax) = super::schema::array(operation, &set)? {
        return Ok(syntax);
    }
    super::set_ops::compile(operation, set, options.work.as_ref())
}

#[cfg(test)]
mod tests {
    crate::rows::set_ops_tests::check_set_provider!();
}
