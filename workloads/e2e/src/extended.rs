use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, LitInt, LitStr, Token, parse::Parse};

mod kw {
    syn::custom_keyword!(parameters);
}
struct Request {
    name: LitStr,
    family: Ident,
    arity: LitInt,
    parameters: TokenStream,
    source: TokenStream,
}
impl Parse for Request {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let header;
        syn::bracketed!(header in input);
        let name = header.parse()?;
        header.parse::<Token![;]>()?;
        let family = header.parse()?;
        header.parse::<Token![;]>()?;
        let arity = header.parse()?;
        header.parse::<Token![;]>()?;
        header.parse::<kw::parameters>()?;
        let parameters;
        syn::braced!(parameters in header);
        let parameters = parameters.parse()?;
        if !header.is_empty() {
            return Err(header.error("unexpected extension header field"));
        }
        Ok(Self {
            name,
            family,
            arity,
            parameters,
            source: input.parse()?,
        })
    }
}
pub fn expand(tokens: TokenStream) -> syn::Result<TokenStream> {
    let Request {
        name,
        family,
        arity,
        parameters,
        source,
    } = syn::parse2(tokens)?;
    let module = format_ident!("{}", name.value());
    let schema = cq_workloads::extended::schema(source.clone())?;
    let setup = schema.iter().enumerate().map(|(i, declaration)| {
        let name = &declaration.name;
        let fields: Vec<_> = (0..declaration.arity()).collect();
        quote!(let #name: ::std::vec::Vec<_> = dataset.relations[#i].rows.iter().map(|row| (#(row[#fields],)*)).collect();)
    });
    let family_name = family.to_string();
    let extension: syn::Path = match family_name.as_str() {
        "union" | "stacked" => syn::parse_quote!(::cq_ext_union::union),
        "negation" => syn::parse_quote!(::cq_ext_negation::negation),
        "aggregate" => syn::parse_quote!(::cq_ext_aggregate::aggregate),
        "recursion" => syn::parse_quote!(::cq_ext_recursion::recursion),
        "correlated" => syn::parse_quote!(::cq_ext_correlated::correlated),
        _ => return Err(syn::Error::new_spanned(family, "unknown extension family")),
    };
    let union_slot = if family_name == "stacked" {
        quote!(
            pub(crate) use ::cq_ext_negation::negation as union_hole;
        )
    } else {
        quote!(
            pub(crate) use ::mini_linq::mini_linq as union_hole;
        )
    };
    let output_arity = arity.base10_parse::<usize>()?;
    let output = if family_name == "aggregate" {
        let parse_keys = |input: syn::parse::ParseStream<'_>| -> syn::Result<usize> {
            let _: Ident = input.parse()?;
            let keys;
            syn::parenthesized!(keys in input);
            let count =
                syn::punctuated::Punctuated::<Ident, Token![,]>::parse_terminated(&keys)?.len();
            let _: TokenStream = input.parse()?;
            Ok(count)
        };
        let key_count = syn::parse::Parser::parse2(parse_keys, source.clone())?;
        let keys: Vec<_> = (0..key_count).map(syn::Index::from).collect();
        let values: Vec<_> = (0..output_arity - key_count)
            .map(syn::Index::from)
            .collect();
        quote!(result.iter().flat_map(|(key, payload)| payload.iter().map(move |value| ::std::vec![#(::core::convert::TryFrom::try_from(key.#keys).unwrap(),)* #(::core::convert::TryFrom::try_from(value.#values).unwrap(),)*])).collect())
    } else {
        let columns: Vec<_> = (0..output_arity)
            .map(|i| format_ident!("column_{i}"))
            .collect();
        quote!(result.map(|(#(#columns,)*)| ::std::vec![#(#columns,)*]).collect())
    };
    Ok(quote! {
        #[allow(non_snake_case)]
        mod #module {
            macro_rules! policy { ($($ix:tt)*) => { ::mini_linq::lower_physical! { $($ix)* policy {} } }; }
            pub(crate) use policy as physical_policy;
            #[allow(unused_imports)]
            mod mini_linq {
                pub(crate) use super::physical_policy;
                pub(crate) use ::mini_linq::{cq_to_ra, index_demands as choose_index_policy, select_structures, structures_to_rust};
                pub(crate) use ::mini_linq::mini_linq as negation_hole;
                pub(crate) use ::mini_linq::mini_linq as aggregate_hole;
                pub(crate) use ::mini_linq::mini_linq as recursion_hole;
                pub(crate) use ::mini_linq::mini_linq as correlated_hole;
                #union_slot
                pub(crate) use ::cq_state_providers::hash_distinct as distinct_state;
                pub(crate) use ::cq_state_providers::default_indexes as index_provider;
            }
            pub(super) fn execute(dataset: &::cq_workloads::dataset::Dataset) -> ::std::vec::Vec<::std::vec::Vec<i32>> {
                #(#setup)* #parameters
                let result = #extension! { #source }; #output
            }
            #[test]
            fn duckdb_scenarios() {
                let case = ::cq_workloads::extended::case(#name).unwrap();
                for scenario in ::cq_workloads::Scenario::ALL { case.check(scenario, execute(&case.load(scenario).unwrap())); }
            }
        }
    })
}
