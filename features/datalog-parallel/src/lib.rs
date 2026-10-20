//! Select with `round = [cq_datalog_parallel::partition];`
//! and `print = cq_datalog_parallel::print_rounds;` in the Datalog pipeline.
//! The rewrite records slice-one execution before any CQ hole is expanded.
use proc_macro::TokenStream;

#[proc_macro]
pub fn partition(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(input.into(), |source, arguments| {
        let target = cq_datalog_partition::compile_with(source, arguments)?;
        cq_datalog_partition::check(source, &target)?;
        Ok(target)
    })
    .into()
}

#[proc_macro]
pub fn print_rounds(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(
        input.into(),
        |source, arguments| {
            if !arguments.is_empty() {
                return Err(syn::Error::new_spanned(
                    arguments,
                    "print_rounds takes no arguments",
                ));
            }
            cq_datalog_rayon_print::compile(source)
        },
    )
    .into()
}
