//! Named pipeline adapters for checked GPU Datalog realization.
use proc_macro::TokenStream;

fn no_arguments(args: proc_macro2::TokenStream) -> syn::Result<()> {
    if args.is_empty() { Ok(()) } else {
        Err(syn::Error::new_spanned(args, "GPU Datalog pass accepts no arguments"))
    }
}

#[proc_macro]
pub fn device_storage(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(
        input.into(), cq_gpu_datalog_core::storage::compile_with,
    ).into()
}

#[proc_macro]
pub fn print_rounds(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(input.into(), |source, args| {
        no_arguments(args)?;
        cq_gpu_datalog_core::print::compile(source)
    }).into()
}

#[proc_macro]
pub fn bind_inputs(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), cq_gpu_datalog_core::bind_inputs).into()
}

#[proc_macro]
pub fn bind_round(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(
        input.into(), cq_gpu_datalog_core::bindings::compile_with,
    ).into()
}
