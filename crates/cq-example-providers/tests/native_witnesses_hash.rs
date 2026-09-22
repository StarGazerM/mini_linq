//! Recovered tests from crates/cq-native-lowering/src/witnesses/hash.rs.
use cq_native_lowering::witnesses::hash::*;

#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn builder_validates_access_without_mandatory_observer() {
        let options = Options {
            access_iter: Some(syn::parse_quote!(crate::rows)),
            work: None,
        };
        let valid =
            syn::parse_quote!(build { out(x) => (([x], row)) :- Rows(x) => for (x,) in (rows); });
        let output = compile(&valid, &options).unwrap();
        assert!(!output.to_string().contains("__cq_work"));
        syn::parse2::<syn::Expr>(output).unwrap();
        let invalid = syn::parse_quote!(build { out(missing) => (missing) :- Rows(x) => for (x,) in (rows); });
        assert!(compile(&invalid, &options).is_err());
        assert!(compile(&valid, &Options::default()).is_err());
    }
}
