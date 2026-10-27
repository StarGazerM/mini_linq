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
        let machine = cq_sequence_pull::machine();
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
        let machine = cq_sequence_pull::machine();
        let analysis = cq_lang_sequence::check_grading(&program, &machine)?;
        cq_sequence_pull::translate(&program, &analysis)
    })();
    match result {
        Ok(expression) => expression.into_token_stream().into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[path = "../../run_holes.rs"]
mod run_holes;

fn arena_hole(
    name: &syn::Ident,
    output: cq_lang_sequence::OutputProtocol,
) -> syn::Result<syn::Expr> {
    let mut structures = run_holes::fixture(&name.to_string());
    let normalized = cq_gpu_sequence_core::constants::compile(&structures.physical)?;
    if normalized != structures.physical {
        let before = cq_lang_structures::physical::contract::check(&structures.physical)?;
        let inputs: std::collections::BTreeMap<_, _> = structures.structures.entries.iter()
            .filter_map(|entry| match &entry.structure {
                cq_lang_structures::Structure::Host { reader, keyed, .. } if reader == "scan" =>
                    Some((before.hosts[&entry.name.to_string()].clone(), keyed.ty.clone())),
                _ => None,
            }).collect();
        structures = cq_structure_selection::compile(&normalized)?;
        let after = cq_lang_structures::contract::check(&structures)?;
        for entry in &mut structures.structures.entries {
            if let cq_lang_structures::Structure::Host { reader, keyed, .. } = &mut entry.structure {
                if reader == "scan" {
                    if let Some(ty) = inputs.get(&after.physical.hosts[&entry.name.to_string()]) {
                        keyed.ty = ty.clone();
                    }
                }
            }
        }
    }
    let structures = cq_gpu_sequence_core::runs::bind(&structures)?;
    let source = cq_sequence_lowering::compile(&structures)?;
    let machine = cq_gpu_sequence::machine();
    let mut planned = cq_sequence_plan::plan(&source, &machine, &cq_gpu_sequence::catalog())?;
    let terminal = planned.program.sequence.definitions.last_mut().unwrap();
    let cq_lang_sequence::Term::Store { capacity, .. } = &mut terminal.term else {
        return Err(syn::Error::new_spanned(name, "expected a planned terminal store"));
    };
    if matches!(output, cq_lang_sequence::OutputProtocol::Reserve) {
        // Keep the fixed-reservation fixture deliberately under-sized so fanout
        // exercises metadata retry independently of previous-count reservation.
        *capacity = Some(1);
    }
    planned.program.sequence.output = Some(output);
    let analysis = cq_lang_sequence::check_grading(&planned.program, &machine)?;
    cq_gpu_sequence_core::translate_arena(
        &planned.program,
        &analysis,
        &syn::parse_quote!(&mut arena),
    )
}

/// Exercise the production arena pipeline with a fixed capacity-one reservation.
#[proc_macro]
pub fn planned_arena_hole(input: TokenStream) -> TokenStream {
    let name = syn::parse_macro_input!(input as syn::Ident);
    match arena_hole(&name, cq_lang_sequence::OutputProtocol::Reserve) {
        Ok(expression) => expression.into_token_stream().into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Expose the exact first-hole expansion for the acceptance source artifact.
#[proc_macro]
pub fn arena_hole_source(input: TokenStream) -> TokenStream {
    let name = syn::parse_macro_input!(input as syn::Ident);
    match arena_hole(&name, cq_lang_sequence::OutputProtocol::Reserve) {
        Ok(expression) => syn::LitStr::new(
            &expression.into_token_stream().to_string(),
            name.span(),
        ).into_token_stream().into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Exercise arena holes using their last validated count as the initial capacity.
#[proc_macro]
pub fn planned_arena_hole_previous(input: TokenStream) -> TokenStream {
    let name = syn::parse_macro_input!(input as syn::Ident);
    match arena_hole(&name, cq_lang_sequence::OutputProtocol::ReservePrevious) {
        Ok(expression) => expression.into_token_stream().into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Expose the previous-count reservation expansion for source-level assertions.
#[proc_macro]
pub fn arena_hole_previous_source(input: TokenStream) -> TokenStream {
    let name = syn::parse_macro_input!(input as syn::Ident);
    match arena_hole(&name, cq_lang_sequence::OutputProtocol::ReservePrevious) {
        Ok(expression) => syn::LitStr::new(
            &expression.into_token_stream().to_string(),
            name.span(),
        ).into_token_stream().into(),
        Err(error) => error.into_compile_error().into(),
    }
}
