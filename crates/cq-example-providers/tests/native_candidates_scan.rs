//! Recovered tests from crates/cq-native-lowering/src/candidates/scan.rs.
use cq_native_lowering::candidates::scan::*;

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
