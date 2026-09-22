//! Recovered tests from crates/cq-native-lowering/src/keys/zone.rs.
use cq_native_lowering::keys::zone::*;

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> Options {
        Options {
            storage: syn::parse_quote!(crate::data),
            access_iter: None,
            keys: None,
            work: None,
        }
    }
    #[test]
    fn tests_need_no_construction_configuration_and_evaluate_arguments_once() {
        let options = options();
        let operation = syn::parse_quote!(may_match(unique_resource())(unique_bounds())(
            unique_columns()
        ));
        let output = compile(&operation, &options).unwrap().to_string();
        for expression in ["unique_resource", "unique_bounds", "unique_columns"] {
            assert_eq!(output.matches(expression).count(), 1);
        }
        let operation = syn::parse_quote!(contains(unique_resource())(unique_key()));
        let mut options = options;
        options.work = Some(syn::parse_quote!(unique_work()));
        let output = compile(&operation, &options).unwrap().to_string();
        for expression in ["unique_resource", "unique_key", "unique_work"] {
            assert_eq!(output.matches(expression).count(), 1);
        }
    }
    #[test]
    fn builder_checks_access_and_requires_construction_dependencies() {
        let mut options = options();
        let valid = syn::parse_quote!(build { out(x) => ([x]) :- Rows(x) => for (x,) in (rows); });
        assert!(compile(&valid, &options).is_err());
        options.access_iter = Some(syn::parse_quote!(crate::rows));
        options.keys = Some(syn::parse_quote!(::std::collections::HashSet));
        let output = compile(&valid, &options).unwrap();
        assert!(!output.to_string().contains("__cq_work"));
        syn::parse2::<syn::Expr>(output).unwrap();
        let invalid = syn::parse_quote!(build { out(missing) => (missing) :- Rows(x) => for (x,) in (rows); });
        assert!(compile(&invalid, &options).is_err());
    }
}
