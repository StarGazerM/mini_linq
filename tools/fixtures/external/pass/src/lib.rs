use proc_macro::TokenStream;
#[proc_macro]
pub fn ordered(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, arguments| {
        if arguments.to_string() != "a + (b * c)" { return Err(syn::Error::new_spanned(arguments, "opaque arguments changed")); }
        let mut target = source.clone();
        for entry in &mut target.structures.entries {
            match &mut entry.structure {
                cq_lang_structures::Structure::Dedup { ty, .. } | cq_lang_structures::Structure::HeadDedup { ty, .. } => *ty = syn::parse_quote!(::std::collections::BTreeSet<_>),
                _ => {}
            }
        }
        Ok(target)
    }).into()
}
