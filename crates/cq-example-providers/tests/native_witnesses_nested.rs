//! Recovered tests from crates/cq-native-lowering/src/witnesses/nested.rs.
use cq_native_lowering::witnesses::nested::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn construction_defers_traversal_and_does_not_evaluate_observer() {
        let operation = syn::parse_quote!(build { out(x) => (([x], row)) :- Rows(x) => for (x,) in (unique_scan()); });
        let options = Options {
            access_iter: Some(syn::parse_quote!(crate::rows)),
            work: Some(syn::parse_quote!(compile_error!("unused work"))),
        };
        let output = compile(&operation, &options).unwrap();
        assert!(matches!(
            syn::parse2::<syn::Expr>(output.clone()).unwrap(),
            syn::Expr::Closure(_)
        ));
        let text = output.to_string();
        assert!(text.contains("unique_scan"));
        for absent in ["collect", "HashMap", "index_rows", "unused work"] {
            assert!(!text.contains(absent), "{text}");
        }
        let invalid = syn::parse_quote!(build { out(missing) => (missing) :- Rows(x) => for (x,) in (rows); });
        assert!(compile(&invalid, &options).is_err());
        assert!(compile(&operation, &Options::default()).is_err());
    }
    #[test]
    fn probe_needs_no_build_iterator_and_evaluates_once() {
        let operation = syn::parse_quote!(probe(unique_resource())(unique_key()));
        let options = Options {
            access_iter: None,
            work: Some(syn::parse_quote!(unique_work())),
        };
        let output = compile(&operation, &options).unwrap().to_string();
        for expression in ["unique_resource", "unique_key", "unique_work"] {
            assert_eq!(output.matches(expression).count(), 1);
        }
    }
}
