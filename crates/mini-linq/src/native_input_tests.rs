//! Recovered tests from crates/mini-linq/src/native_input.rs.
use crate::native_input::*;
use quote::ToTokens;
use syn::{Token, parse::Parse, parse::ParseStream, token};
#[cfg(test)]
mod tests {
    use super::*;
    use cq_ir::native::Operation;
    use quote::quote;

    #[test]
    fn only_implementation_owned_configuration_is_accepted() {
        let input: Input<Operation> =
            syn::parse_quote!(candidates crate::rows { &rows } (predicate));
        input.scan_source().unwrap();
        let input: Input<Operation> = syn::parse_quote!(rows(&rows));
        input.hash_row_options().unwrap();
        let input: Input<Operation> = syn::parse_quote!(storage crate::data; rows(&rows));
        assert!(input.hash_row_options().is_err());
        input.block_options().unwrap();
        let input: Input<Operation> = syn::parse_quote!(storage crate::data; contains(&keys)([1]));
        let options = input.exact_options().unwrap();
        assert!(options.access_iter.is_none());
        cq_native_lowering::keys::exact::compile(input.source(), &options).unwrap();
    }

    #[test]
    fn duplicate_directives_and_unknown_options_are_rejected() {
        for duplicate in [
            quote!(storage b;),
            quote!(access_iter b;),
            quote!(keys b;),
            quote!(work (b);),
            quote!(packing (b);),
        ] {
            let input: Input<Operation> = syn::parse2(quote! {
                storage a; access_iter a; keys a; work (a); packing (a); #duplicate rows(&rows)
            })
            .unwrap();
            assert!(
                input
                    .configuration()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("duplicate")
            );
        }
        assert!(syn::parse2::<Input<Operation>>(quote!(unknown a; rows(&rows))).is_err());
    }

    #[test]
    fn packing_is_administrative_and_round_trips_in_either_order() {
        let source: Operation = syn::parse_quote!(materialize owner value (rows));
        for input in [
            quote!(storage crate::data; packing (unique_size()); #source),
            quote!(packing (unique_size()); storage crate::data; #source),
        ] {
            let parsed: Input<Operation> = syn::parse2(input.clone()).unwrap();
            assert_eq!(parsed.source(), &source);
            assert_eq!(parsed.to_token_stream().to_string(), input.to_string());
            assert!(parsed.block_options().unwrap().packing.is_some());
        }
    }

    #[test]
    fn directive_words_remain_available_as_source_definition_names() {
        #[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
        struct Definition {
            name: syn::Ident,
            equals: Token![=],
            value: syn::Expr,
            semi: Token![;],
        }
        for name in ["storage", "access_iter", "keys", "work", "packing"] {
            let name: syn::Ident = syn::parse_str(name).unwrap();
            let input: Input<Definition> =
                syn::parse2(quote!(storage crate::data; #name = 1;)).unwrap();
            assert_eq!(input.source().name, name);
            input.block_options().unwrap();
        }
    }
}
