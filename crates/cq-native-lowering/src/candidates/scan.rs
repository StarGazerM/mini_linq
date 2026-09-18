//! Enumerate current handles without evaluating metadata-only operands.
use cq_ir::native::Operation;
use proc_macro2::TokenStream;
use quote::quote;

pub fn compile(operation: &Operation) -> syn::Result<TokenStream> {
    let Operation::Candidates {
        provider, value, ..
    } = operation
    else {
        return Err(syn::Error::new_spanned(
            operation,
            "row scan requires candidates",
        ));
    };
    Ok(quote!(#provider! { rows (#value) }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn unused_metadata_is_not_emitted() {
        let operation = syn::parse_quote! {
            candidates crate::rows { unique_view() } (compile_error!("unused predicate"))
        };
        let output = super::compile(&operation).unwrap().to_string();
        assert_eq!(output.matches("unique_view").count(), 1);
        assert!(!output.contains("compile_error"));
    }
}
