//! Replaceable refinements of native leaves in the existing Access language.
//!
//! Row candidates are justified together with their enclosing exact residual
//! predicate. Resources are constructed from an Access plan; no RA definition,
//! logical environment, or query executor is serialized into these leaves.

mod keys;
mod scan;
mod witnesses;

pub use keys::{compile_exact_keys, compile_zone_keys};
pub use scan::{compile_prune_rows, compile_scan_rows};
pub use witnesses::{compile_hash_witnesses, compile_nested_witnesses};

use proc_macro2::TokenStream;
use quote::quote;

fn work_binding(work: Option<&syn::Expr>) -> Option<TokenStream> {
    work.map(|work| quote!(let __cq_work = (#work);))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical::options::AccessOptions;
    use cq_ir::physical_provider::Operation;
    use quote::quote;

    type Compile = fn(&Operation, &AccessOptions) -> syn::Result<TokenStream>;

    fn options(work: Option<syn::Expr>) -> AccessOptions {
        AccessOptions {
            storage: syn::parse_quote!(crate::data),
            iterator: syn::parse_quote!(crate::iterator),
            keys: Some(syn::parse_quote!(::std::collections::HashSet)),
            work,
        }
    }

    #[test]
    fn plain_scan_does_not_expand_unused_metadata_predicate_or_instrumentation() {
        let operation = syn::parse_quote! {
            candidates crate::rows { unique_view() }
                (compile_error!("unused predicate"))
        };
        let output = compile_scan_rows(
            &operation,
            &options(Some(syn::parse_quote!(compile_error!("unused work")))),
        )
        .unwrap();
        assert_eq!(output.to_string().matches("unique_view").count(), 1);
        assert!(!output.to_string().contains("compile_error"));
        syn::parse2::<syn::Expr>(output).unwrap();
    }

    #[test]
    fn resource_builders_reject_invalid_access_before_emission() {
        let operation = syn::parse_quote! {
            build { out(missing) => (missing) :- Rows(x) => for (x,) in (rows); }
        };
        for compile in [
            compile_exact_keys,
            compile_zone_keys,
            compile_hash_witnesses,
            compile_nested_witnesses,
        ] {
            assert!(compile(&operation, &options(None)).is_err());
        }
    }

    #[test]
    fn native_access_operations_produce_expressions_without_mandatory_counters() {
        let key_plan: Operation = syn::parse_quote! {
            build { out(x) => ([x]) :- Rows(x) => for (x,) in (rows); }
        };
        let witness_plan: Operation = syn::parse_quote! {
            build { out(x) => (([x], row)) :- Rows(x) => for (x,) in (rows); }
        };
        let candidates: Operation = syn::parse_quote! {
            candidates crate::rows { &view } (|bounds| accepts(bounds))
        };
        for (compile, operation) in [
            (compile_exact_keys as Compile, &key_plan),
            (compile_zone_keys, &key_plan),
            (compile_hash_witnesses, &witness_plan),
            (compile_nested_witnesses, &witness_plan),
            (compile_scan_rows, &candidates),
            (compile_prune_rows, &candidates),
        ] {
            let output = compile(operation, &options(None)).unwrap();
            let text = output.to_string();
            assert!(!text.contains("__cq_work"), "{text}");
            assert!(!text.contains(". add"), "{text}");
            syn::parse2::<syn::Expr>(output).unwrap();
        }
    }

    #[test]
    fn nested_construction_defers_traversal_until_each_probe() {
        let operation = syn::parse_quote! {
            build { out(x) => (([x], row)) :- Rows(x) => for (x,) in (unique_scan()); }
        };
        let output = compile_nested_witnesses(
            &operation,
            &options(Some(syn::parse_quote!(compile_error!("unused work")))),
        )
        .unwrap();
        let expression: syn::Expr = syn::parse2(output.clone()).unwrap();
        assert!(matches!(expression, syn::Expr::Closure(_)));
        let text = output.to_string();
        assert!(text.contains("unique_scan"));
        assert!(!text.contains("collect"));
        assert!(!text.contains("HashMap"));
        assert!(!text.contains("index_rows"));
        assert!(!text.contains("unused work"));
    }

    #[test]
    fn probe_and_membership_evaluate_arguments_once() {
        for (compile, operation) in [
            (
                compile_exact_keys as Compile,
                quote!(contains(unique_resource())(unique_key())),
            ),
            (
                compile_zone_keys,
                quote!(contains(unique_resource())(unique_key())),
            ),
            (
                compile_hash_witnesses,
                quote!(probe(unique_resource())(unique_key())),
            ),
            (
                compile_nested_witnesses,
                quote!(probe(unique_resource())(unique_key())),
            ),
        ] {
            let operation = syn::parse2(operation).unwrap();
            let output = compile(&operation, &options(Some(syn::parse_quote!(unique_work()))))
                .unwrap()
                .to_string();
            for expression in ["unique_resource", "unique_key", "unique_work"] {
                assert_eq!(output.matches(expression).count(), 1, "{output}");
            }
        }
    }
}
