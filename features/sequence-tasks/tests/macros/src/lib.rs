use proc_macro::TokenStream;
use quote::quote;

/// Compile both interpretations of exactly the same checked Sequence program.
#[proc_macro]
pub fn both(input: TokenStream) -> TokenStream {
    let mut program = syn::parse_macro_input!(input as cq_lang_sequence::Program);
    let result = (|| {
        let providers = program.structures.structures.entries.clone();
        program.structures = cq_structure_selection::compile(&program.structures.physical)?;
        for provider in providers {
            program.structures.structures.entries.retain(|entry| {
                entry.name != provider.name
                    || std::mem::discriminant(&entry.structure)
                        != std::mem::discriminant(&provider.structure)
            });
            program.structures.structures.entries.push(provider);
        }
        let analysis = cq_lang_sequence::check_grading(&program, &cq_sequence_tasks::machine())?;
        let mut sequential = program.clone();
        sequential.graded = None;
        let sequential_analysis = cq_lang_sequence::contract::check(&sequential)?;
        let pull = cq_sequence_pull::translate(&sequential, &sequential_analysis)?;
        let tasks = cq_sequence_tasks::translate(&program, &analysis)?;
        Ok::<_, syn::Error>(quote! {
            (::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#pull), #tasks)
        })
    })();
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}
