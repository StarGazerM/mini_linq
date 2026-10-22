use proc_macro::TokenStream;
use quote::ToTokens;

#[allow(dead_code)]
#[path = "../../h05_program.rs"]
mod h05_program;

/// Expand the same hand-written Program checked by tests/h05.rs. The only
/// query code in the executable comes from the public Sequence translator.
#[proc_macro]
pub fn h05(input: TokenStream) -> TokenStream {
    let capacity = syn::parse_macro_input!(input as syn::LitInt);
    let result = (|| {
        let program = h05_program::program_with_capacity(capacity.base10_parse()?);
        let analysis = cq_lang_sequence::check_grading(&program, &cq_gpu_sequence::machine())?;
        cq_gpu_sequence::translate(&program, &analysis)
    })();
    match result {
        Ok(expression) => expression.into_token_stream().into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Interpret the fixture's Physical program through default CPU structures and
/// Sequential lowering. This is a Physical reference, not the GPU Sequence
/// program with its resident provider types silently replaced.
#[proc_macro]
pub fn h05_sequential_physical_reference(input: TokenStream) -> TokenStream {
    let capacity = syn::parse_macro_input!(input as syn::LitInt);
    let result = (|| {
        let gpu = h05_program::program_with_capacity(capacity.base10_parse()?);
        let structures = cq_structure_selection::compile(&gpu.structures.physical)?;
        let program = cq_sequence_lowering::compile(&structures)?;
        let machine = cq_lang_sequence::Machine {
            name: syn::parse_quote!(sequential),
            root: cq_lang_sequence::LevelNode {
                name: syn::parse_quote!(host),
                collaborative: false,
                dynamic: false,
                children: vec![],
            },
        };
        let analysis = cq_lang_sequence::check_grading(&program, &machine)?;
        cq_sequence_pull::translate(&program, &analysis)
    })();
    match result {
        Ok(expression) => expression.into_token_stream().into(),
        Err(error) => error.into_compile_error().into(),
    }
}
