//! Recovered tests from crates/cq-native-lowering/src/candidates/prune.rs.
use cq_native_lowering::candidates::prune::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traversal_evaluates_operands_once_without_storage_configuration() {
        let operation =
            syn::parse_quote!(candidates crate::rows { unique_view() } (unique_predicate()));
        let options = Options {
            access_iter: syn::parse_quote!(crate::iterate),
            work: Some(syn::parse_quote!(unique_work())),
        };
        let output = compile(&operation, &options).unwrap().to_string();
        for expression in ["unique_view", "unique_predicate", "unique_work"] {
            assert_eq!(output.matches(expression).count(), 1);
        }
    }
    #[test]
    fn plain_traversal_contains_no_observer() {
        let operation =
            syn::parse_quote!(candidates crate::rows { &view } (|bounds| accepts(bounds)));
        let options = Options {
            access_iter: syn::parse_quote!(crate::iterate),
            work: None,
        };
        let output = compile(&operation, &options).unwrap();
        assert!(!output.to_string().contains("__cq_work"));
        syn::parse2::<syn::Expr>(output).unwrap();
    }
}
