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

/// Fill unauthored decomposition using the GPU's exported machine and catalog.
#[proc_macro]
pub fn plan(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        Ok(cq_sequence_plan::plan(
            source,
            &cq_gpu_sequence_core::machine(),
            &cq_gpu_sequence_core::catalog(),
        )?
        .program)
    })
    .into()
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

/// Publish raw candidate rows into the explicitly supplied mutable arena.
///
/// Use `translate_arena(&mut arena)` as the terminal pipeline pass. The
/// recorded output protocol still controls counting and capacity retries.
#[proc_macro]
pub fn translate_arena(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_sequence::Program, syn::Expr>(input.into(), |source, arguments| {
        if arguments.is_empty() {
            return Err(syn::Error::new(proc_macro2::Span::call_site(),
                "GPU Sequence translate_arena requires an explicit arena expression"));
        }
        let arena = syn::parse2::<syn::Expr>(arguments)?;
        let analysis = cq_lang_sequence::check_grading(source, &cq_gpu_sequence_core::machine())?;
        cq_gpu_sequence_core::translate_arena(source, &analysis, &arena)
    })
    .into()
}

/// Bind declared native run inputs before lowering Structures to Sequence.
#[proc_macro]
pub fn bind_runs(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        cq_gpu_sequence_core::runs::bind(source)
    })
    .into()
}

/// Expose literal atom arguments as checked guards before provider selection.
#[proc_macro]
pub fn capture_constants(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::physical::Program>(input.into(), |source, arguments| {
        no_arguments(arguments)?;
        cq_gpu_sequence_core::constants::compile(source)
    }).into()
}
