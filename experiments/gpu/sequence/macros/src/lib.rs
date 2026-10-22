//! Named pipeline adapters for the checked GPU Sequence compiler.

use proc_macro::TokenStream;

fn no_arguments(arguments: proc_macro2::TokenStream) -> syn::Result<()> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            arguments,
            "GPU Sequence passes accept no arguments; record decisions in decompose",
        ))
    }
}

/// Check the authored grades against the GPU machine without choosing grades.
#[proc_macro]
pub fn machine_grade(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        cq_lang_sequence::check_grading(source, &cq_gpu_sequence_core::machine())?;
        Ok(source.clone())
    })
    .into()
}

/// Translate a checked, explicitly graded Sequence program to CUDA.
#[proc_macro]
pub fn translate(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_sequence::Program, syn::Expr>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        let analysis = cq_lang_sequence::check_grading(source, &cq_gpu_sequence_core::machine())?;
        cq_gpu_sequence_core::translate(source, &analysis)
    })
    .into()
}
