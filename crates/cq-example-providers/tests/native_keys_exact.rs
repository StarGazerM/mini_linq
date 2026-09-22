//! Recovered tests from crates/cq-native-lowering/src/keys/exact.rs.
use cq_native_lowering::keys::exact::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_requires_no_construction_configuration_and_evaluates_once() {
        let operation = syn::parse_quote!(contains(unique_resource())(unique_key()));
        let options = Options {
            storage: Some(syn::parse_quote!(crate::data)),
            work: Some(syn::parse_quote!(unique_work())),
            ..Options::default()
        };
        let output = compile(&operation, &options).unwrap().to_string();
        for expression in ["unique_resource", "unique_key", "unique_work"] {
            assert_eq!(output.matches(expression).count(), 1, "{output}");
        }
    }

    #[test]
    fn build_requires_only_its_dependencies_and_checks_access() {
        let options = Options {
            access_iter: Some(syn::parse_quote!(crate::rows)),
            keys: Some(syn::parse_quote!(::std::collections::HashSet)),
            ..Options::default()
        };
        let valid = syn::parse_quote!(build { out(x) => ([x]) :- Rows(x) => for (x,) in (rows); });
        let output = compile(&valid, &options).unwrap();
        assert!(!output.to_string().contains("__cq_work"));
        syn::parse2::<syn::Expr>(output).unwrap();
        let invalid = syn::parse_quote!(build { out(missing) => (missing) :- Rows(x) => for (x,) in (rows); });
        assert!(compile(&invalid, &options).is_err());
        assert!(compile(&valid, &Options::default()).is_err());
    }
}
