//! Named pipeline adapters for checked GPU Datalog realization.
use proc_macro::TokenStream;

/// Emit domain codes directly into the arena's measured output layout.
#[proc_macro]
pub fn translate_domain_arena(input: TokenStream) -> TokenStream {
    cq_stage::Sequence::<cq_lang_structures::physical::Program, cq_lang_structures::Program, cq_lang_sequence::Program>::pass::<syn::Expr>(
        input.into(), |physical, structures, source, arguments| {
            let arena: syn::Expr = syn::parse2(arguments)?;
            let source = cq_gpu_sequence_core::runs::record_representation(physical, structures, source)?;
            let analysis = cq_lang_sequence::check_grading(physical, structures, &source, &cq_gpu_sequence_core::machine())?;
            cq_gpu_sequence_core::arena::translate_emission(
                physical, structures, &source, &analysis,
                &cq_gpu_sequence_core::arena::ArenaEmission {
                    arena: arena.clone(),
                    layout: Some(syn::parse_quote!((#arena).output_layout()?)),
                },
            )
        },
    ).into()
}

fn no_arguments(args: proc_macro2::TokenStream) -> syn::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            args,
            "GPU Datalog pass accepts no arguments",
        ))
    }
}

#[proc_macro]
pub fn device_storage(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(
        input.into(),
        cq_gpu_datalog_core::storage::compile_with,
    )
    .into()
}

#[proc_macro]
pub fn domain_storage(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(
        input.into(), cq_gpu_datalog_core::storage::compile_domains_with,
    ).into()
}

#[proc_macro]
pub fn domain_schema(input: TokenStream) -> TokenStream {
    cq_gpu_datalog_core::domains::schema(input.into())
        .unwrap_or_else(syn::Error::into_compile_error).into()
}

#[proc_macro]
pub fn encode_domains(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::cq::Program, cq_lang_datalog_round::cq::Program>(
        input.into(), cq_gpu_datalog_core::encode::compile,
    ).into()
}

#[proc_macro]
pub fn print_rounds(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(input.into(), |source, args| {
        if cq_gpu_datalog_core::storage::uses_domains(&source.storage) {
            let catalog: syn::Expr = syn::parse2(args)?;
            let body = cq_gpu_datalog_core::print::compile(source)?;
            Ok(syn::parse_quote!({ let __cq_gpu_domains = #catalog; #body }))
        } else {
            no_arguments(args)?;
            cq_gpu_datalog_core::print::compile(source)
        }
    })
    .into()
}

#[proc_macro]
pub fn bind_inputs(input: TokenStream) -> TokenStream {
    cq_stage::Structures::<cq_lang_structures::physical::Program, cq_lang_structures::Program>::rewrite(
        input.into(),
        |physical: &cq_lang_structures::physical::Program, source: &cq_lang_structures::Program, args| {
            let target = cq_gpu_datalog_core::bind_inputs(physical, source, args)?;
            Ok((physical.clone(), target))
        },
    )
    .into()
}

#[proc_macro]
pub fn bind_round(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(
        input.into(),
        cq_gpu_datalog_core::bindings::compile_with,
    )
    .into()
}

/// `lanes(streams = K)`: record DL-WaveLanes for every multi-hole wave.
#[proc_macro]
pub fn lanes(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(
        input.into(),
        cq_gpu_datalog_core::lanes::compile_with,
    )
    .into()
}
