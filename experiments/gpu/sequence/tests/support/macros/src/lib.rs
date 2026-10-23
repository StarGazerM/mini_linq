use proc_macro::TokenStream;
use quote::ToTokens;

/// Expand an authored checked Sequence fixture without planning decisions.
#[proc_macro]
pub fn sequence(input: TokenStream) -> TokenStream {
    let program = syn::parse_macro_input!(input as cq_lang_sequence::Program);
    let result = cq_lang_sequence::check_grading(&program, &cq_gpu_sequence::machine())
        .and_then(|analysis| cq_gpu_sequence::translate(&program, &analysis));
    match result {
        Ok(expression) => expression.into_token_stream().into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[allow(dead_code)]
#[path = "../../h05_program.rs"]
mod h05_program;

#[allow(dead_code)]
#[path = "../../h05_plans.rs"]
mod h05_plans;

fn fixture(name: syn::Ident) -> syn::Result<cq_lang_sequence::Program> {
    match name.to_string().as_str() {
        "A" => Ok(h05_plans::a()),
        "B" => Ok(h05_plans::b()),
        "R" => Ok(h05_plans::r()),
        _ => Err(syn::Error::new_spanned(
            name,
            "expected H05 plan A, B, or R",
        )),
    }
}

/// Lower the authored Structures choices, then use only generated scheduling.
#[proc_macro]
pub fn planned_h05(input: TokenStream) -> TokenStream {
    let name = syn::parse_macro_input!(input as syn::Ident);
    let result = (|| {
        let authored = fixture(name)?;
        let source = cq_sequence_lowering::compile(&authored.structures)?;
        let machine = cq_gpu_sequence::machine();
        let planned = cq_sequence_plan::plan(&source, &machine, &cq_gpu_sequence::catalog())?;
        let analysis = cq_lang_sequence::check_grading(&planned.program, &machine)?;
        cq_gpu_sequence::translate(&planned.program, &analysis)
    })();
    match result {
        Ok(expression) => expression.into_token_stream().into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Compile the same selected Physical program independently through Sequential.
#[proc_macro]
pub fn h05_physical_reference(input: TokenStream) -> TokenStream {
    let name = syn::parse_macro_input!(input as syn::Ident);
    let result = (|| {
        let authored = fixture(name)?;
        let structures = cq_structure_selection::compile(&authored.structures.physical)?;
        let program = cq_sequence_lowering::compile(&structures)?;
        let machine = cq_lang_sequence::Machine {
            layout: None,
            name: syn::parse_quote!(sequential),
            items: 1,
            root: cq_lang_sequence::LevelNode {
                shared_bytes: None,
                name: syn::parse_quote!(host),
                collaborative: false,
                dynamic: false,
                width: None,
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
            layout: None,
            name: syn::parse_quote!(sequential),
            items: 1,
            root: cq_lang_sequence::LevelNode {
                shared_bytes: None,
                name: syn::parse_quote!(host),
                collaborative: false,
                dynamic: false,
                width: None,
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
