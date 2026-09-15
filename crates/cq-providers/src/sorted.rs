//! Native BTreeSet row storage and its complete operation protocol.
use cq_ir::physical_provider::Operation;
use proc_macro2::TokenStream;
use quote::quote;

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub work: Option<syn::Expr>,
}

pub fn compile(operation: &Operation, options: &Options) -> syn::Result<TokenStream> {
    super::native::compile(
        operation,
        quote!(::std::collections::BTreeSet),
        options.work.as_ref(),
    )
}

#[cfg(test)]
mod tests {
    crate::native_tests::check_native_provider!();
}
