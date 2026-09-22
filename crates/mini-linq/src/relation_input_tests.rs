//! Recovered tests from crates/mini-linq/src/relation_input.rs.
use crate::relation_input::*;
use cq_ir::{relational_plan as ra, symbol_name};
use quote::ToTokens;
use std::collections::BTreeMap;
#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn source() -> ra::Module {
        syn::parse_quote! {
            pub struct P;
            relation R(k:i64);
            answer(k) :- R(k);
            relational {
                r = rename R {k -> k};
                output r as answer(k);
            }
        }
    }

    fn required() -> proc_macro2::TokenStream {
        quote! {
            access crate::access;
            result_rows crate::rows;
            candidates crate::reduce;
            keys crate::Keys;
            pairs crate::pairs;
        }
    }

    #[test]
    fn configured_ra_round_trips_without_an_intermediate_language() {
        let source = source();
        let tokens = quote! {
            inputs { r#R => crate::rows; }
            pairs crate::pairs;
            result_bindings {}
            keys crate::Keys;
            access crate::access;
            result_rows crate::rows;
            candidates crate::reduce;
            #source
        };
        let input: RelationInput = syn::parse2(tokens.clone()).unwrap();
        input.options().unwrap();
        assert_eq!(input.to_token_stream().to_string(), tokens.to_string());
    }

    #[test]
    fn unused_defaults_are_absent_from_administrative_configuration() {
        let source = source();
        let input: RelationInput = syn::parse_quote!(#source);
        let options = input.options().unwrap();
        assert!(options.access.is_none());
        assert!(options.result_rows.is_none());
        assert!(options.candidates.is_none());
        assert!(options.keys.is_none());
        assert!(options.pairs.is_none());
    }

    #[test]
    fn unknown_directives_report_the_declared_keyword_alternatives() {
        let source = source();
        let required = required();
        let error = syn::parse2::<RelationInput>(quote! {
            #required mystery crate::other; #source
        })
        .unwrap_err();
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("expected one of"), "{diagnostic}");
        for keyword in [
            "counter_storage",
            "inputs",
            "candidate_bindings",
            "key_sets",
            "pair_bindings",
        ] {
            assert!(diagnostic.contains(keyword), "{diagnostic}");
        }
    }

    #[test]
    fn duplicate_scalar_and_scope_directives_are_rejected() {
        let source = source();
        let required = required();
        for duplicate in [
            quote!(access other::access;),
            quote!(inputs {} inputs {}),
            quote!(pair_bindings {} pair_bindings {}),
        ] {
            let input: RelationInput = syn::parse2(quote!(#required #duplicate #source)).unwrap();
            let error = input.options().err().unwrap();
            assert!(error.to_string().contains("duplicate physical directive"));
        }
    }

    #[test]
    fn observer_configuration_is_owned_by_its_host() {
        let source = source();
        let required = required();
        let plain: RelationInput = syn::parse_quote!(#required #source);
        assert!(plain.options().is_ok());
        assert!(plain.observed_options().is_err());
        let observed: RelationInput =
            syn::parse_quote!(counter_storage crate::counters; #required #source);
        assert!(observed.options().is_err());
        assert!(observed.observed_options().is_ok());
    }

    #[test]
    fn binding_duplicates_are_normalized_within_each_scope() {
        let source = source();
        let required = required();
        let duplicate: RelationInput = syn::parse_quote! {
            #required inputs { R => a::rows; r#R => b::rows; } #source
        };
        assert!(
            duplicate
                .options()
                .err()
                .unwrap()
                .to_string()
                .contains("duplicate physical binding")
        );

        let separate: RelationInput = syn::parse_quote! {
            #required
            inputs { R => a::rows; }
            result_bindings { R => b::rows; }
            #source
        };
        let options = separate.options().unwrap();
        assert!(options.inputs.contains_key("R"));
        assert!(options.result_bindings.contains_key("R"));
        assert!(options.candidate_bindings.is_empty());
        assert!(options.key_sets.is_empty());
        assert!(options.pair_bindings.is_empty());
    }
}
