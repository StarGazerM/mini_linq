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
        let loader = format_ident!("{}Loader", cq_lang_cq::symbol_name(name));
        quote!(let #name = loaders::#loader::csv(
            directory.join(::cq_workloads::dataset::input_file_name(#i)),
            ::cq_load::CsvOptions::default(),
        ).unwrap();)
    });
    let validation = schema.iter().map(|declaration| {
        let name = &declaration.name;
        let relation_name = cq_lang_cq::symbol_name(name);
        let arity = declaration.arity();
        let fields: Vec<_> = (0..arity).map(syn::Index::from).collect();
        quote!(::cq_workloads::dataset::RelationData::new(#relation_name, #arity,
            #name.iter().map(|row| vec![#(row.#fields,)*]).collect()).unwrap())
    });
    let family_name = family.to_string();
    let surfaces = match family_name.as_str() {
        "surface" => quote!(),
        "union" => quote!(::cq_ext_union::union),
        "stacked" => quote!(::cq_ext_union::union, ::cq_ext_negation::negation),
        "negation" => quote!(::cq_ext_negation::negation),
        "aggregate" => quote!(::cq_ext_aggregate::aggregate),
        "recursion" => quote!(::cq_ext_recursion::recursion),
        "correlated" => quote!(::cq_ext_correlated::correlated),
        _ => return Err(syn::Error::new_spanned(family, "unknown extension family")),
    };
    let authored = match family_name.as_str() {
        "union" | "stacked" => quote!(union! { #source }),
        "aggregate" => quote!(aggregate! { #source }),
        "recursion" => quote!(recursion! { #source }),
        _ => source.clone(),
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
            mod loaders { ::mini_linq::relation_loaders! { #(#schema)* } }
            pub(super) fn execute(case: ::cq_workloads::extended::Case, scenario: ::cq_workloads::Scenario) -> ::std::vec::Vec<::std::vec::Vec<i32>> {
                let directory = case.path("data").join(scenario.as_str()).join("tiny");
                #(#setup)*
                let dataset = ::cq_workloads::dataset::Dataset::from_loaded_schema(
                    &case.schema().unwrap(), &directory, vec![#(#validation,)*],
                ).unwrap();
                dataset.validate_provenance(::cq_workloads::Suite::Extended, case.name, case.config(scenario)).unwrap();
                assert_eq!(dataset, case.generate(scenario).unwrap(), "stale extension fixture: {}", case.name);
                #parameters
                let result = ::mini_linq::compile! { pipeline { surface = [#surfaces]; } #authored }; #output
            }
            #[test]
            fn duckdb_scenarios() {
                let case = ::cq_workloads::extended::case(#name).unwrap();
                for scenario in ::cq_workloads::Scenario::ALL { case.check(scenario, execute(*case, scenario)); }
            }
        }
    })
}
