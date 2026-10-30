use proc_macro::TokenStream;
#[proc_macro]
pub fn accept_sort(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, arguments| {
        if arguments.to_string() != "a + (b * c)" { return Err(syn::Error::new_spanned(arguments, "opaque arguments changed")); }
        for entry in &source.structures.entries {
            if let cq_lang_structures::Structure::Dedup { strategy, .. }
            | cq_lang_structures::Structure::HeadDedup { strategy, .. } = &entry.structure {
                assert!(matches!(strategy, cq_lang_structures::CanonStrategy::Sort(_)));
            }
        }
        Ok(source.clone())
    }).into()
}
