//! A conformance Sequence terminal for continuation propagation.
//!
//! Its `twice` argument is mandatory and realized as two copies of the complete
//! stream in the emitted Rust. Duplicated witnesses denote the same finite set.
//! This fixture requires Clone answers and validates extensional extension
//! boundaries when a caller's downstream printer repeats witnesses.
use quote::quote;

#[proc_macro]
pub fn repeated(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::pass::<cq_lang_sequence::Program, syn::Expr>(input.into(), |source, arguments| {
        let argument: syn::Ident = syn::parse2(arguments)?;
        if argument != "twice" {
            return Err(syn::Error::new_spanned(argument, "expected twice"));
        }
        let analysis = cq_lang_sequence::check_grading(
            source,
            &cq_lang_sequence::Machine { key_partition_threshold: 65536, layout: None,
            name: syn::parse_quote!(sequential),
            items: 1,
            root: cq_lang_sequence::LevelNode {
                shared_bytes: None,
                name: syn::parse_quote!(host),
                collaborative: false,
                dynamic: false,
                width: None,
                children: vec![],
            }, },
        )?;
        let printed = cq_sequence_pull::translate(source, &analysis)?;
        syn::parse2(quote!({
            let rows = (#printed).collect::<::std::vec::Vec<_>>();
            rows.clone().into_iter().chain(rows)
        }))
    })
    .into()
}
