use proc_macro::TokenStream;
#[proc_macro]
pub fn accept_sort(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, arguments| {
        if arguments.to_string() != "a + (b * c)" { return Err(syn::Error::new_spanned(arguments, "opaque arguments changed")); }
        let analysis = cq_lang_structures::contract::check(&source)?;
        for canonical in analysis.canonical.values() {
            let cq_lang_structures::CanonSource::Rows(ty) = canonical else {
                panic!("expected a sort-backed canonical rows index");
            };
            assert_eq!(ty, &syn::parse_quote!(::cq_storage::dedup::Sort));
        }
        for ty in analysis.head_dedup.values() {
            assert_eq!(ty, &syn::parse_quote!(::cq_storage::dedup::Sort));
        }
        Ok(source.clone())
    }).into()
}
