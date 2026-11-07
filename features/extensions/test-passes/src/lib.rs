//! Conformance Sequence terminals for continuation propagation.
//!
//! `canonical(once)` preserves the core canonical return contract, including
//! multiplicity-sensitive aggregate consumers. `repeated(twice)` deliberately
//! violates that contract and is used only for extensional set-boundary tests.
use quote::quote;

#[proc_macro]
pub fn repeated(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::Sequence::<cq_lang_sequence::structures::physical::Program, cq_lang_sequence::structures::Program, cq_lang_sequence::Program>::pass::<syn::Expr>(input.into(), |physical, structures, source, arguments| {
        let argument: syn::Ident = syn::parse2(arguments)?;
        if argument != "twice" {
            return Err(syn::Error::new_spanned(argument, "expected twice"));
        }
        let analysis = cq_lang_sequence::check_grading(physical, structures, source, &cq_sequence_pull::machine())?;
        let printed = cq_sequence_pull::translate(physical, structures, source, &analysis)?;
        syn::parse2(quote!({
            let rows = (#printed).collect::<::std::vec::Vec<_>>();
            rows.clone().into_iter().chain(rows)
        }))
    })
    .into()
}

#[proc_macro]
pub fn canonical(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::Sequence::<cq_lang_sequence::structures::physical::Program, cq_lang_sequence::structures::Program, cq_lang_sequence::Program>::pass::<syn::Expr>(input.into(), |physical, structures, source, arguments| {
        let argument: syn::Ident = syn::parse2(arguments)?;
        if argument != "once" {
            return Err(syn::Error::new_spanned(argument, "expected once"));
        }
        let analysis = cq_lang_sequence::check_grading(physical, structures, source, &cq_sequence_pull::machine())?;
        cq_sequence_pull::translate(physical, structures, source, &analysis)
    })
    .into()
}
