use proc_macro::TokenStream;
#[proc_macro]
pub fn accept_sort(input: TokenStream) -> TokenStream {
    cq_stage::Structures::<cq_lang_structures::physical::Program, cq_lang_structures::Program>::rewrite(input.into(), |physical, source, arguments| {
        if arguments.to_string() != "a + (b * c)" { return Err(syn::Error::new_spanned(arguments, "opaque arguments changed")); }
        let analysis = cq_lang_structures::contract::check(physical, source)?;
        for canonical in analysis.canonical.values() {
            let cq_lang_structures::CanonSource::Rows(provider) = canonical else {
                panic!("expected a sort-backed canonical rows index");
            };
            assert_eq!(provider.token, "sort");
            assert_eq!(&provider.ty, &syn::parse_quote!(::cq_storage::dedup::Sort));
        }
        for provider in analysis.head_dedup.values() {
            assert_eq!(provider.token, "sort");
            assert_eq!(&provider.ty, &syn::parse_quote!(::cq_storage::dedup::Sort));
        }
        Ok((physical.clone(), source.clone()))
    }).into()
}
