//! Target-owned planning on the checked Sequence pipeline protocol.
//!
//! Use `mini_linq::passes::plan(cq_sequence_tasks_macros::plan)` after authored
//! advice in `decompose`, or invoke `plan` directly in a checked pass queue.

use proc_macro::TokenStream;

/// Fill only unauthored decisions using the Tasks machine and catalog.
#[proc_macro]
pub fn plan(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, arguments| {
        if !arguments.is_empty() {
            return Err(syn::Error::new_spanned(
                arguments,
                "Tasks planning accepts no arguments; record advice before plan",
            ));
        }
        Ok(cq_sequence_plan::plan(
            source,
            &cq_sequence_tasks::machine(),
            &cq_sequence_tasks::catalog(),
        )?
        .program)
    })
    .into()
}

/// Check the target-owned schedule before interpretation.
#[proc_macro]
pub fn tasks(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, _| {
        cq_lang_sequence::check_grading(source, &cq_sequence_tasks::machine())?;
        Ok(source.clone())
    }).into()
}

/// Return joined owned answers through the ordinary Pull iterator ABI.
#[proc_macro]
pub fn translate_tasks(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_sequence::Program, syn::Expr>(input.into(), |source, _| {
        let analysis = cq_lang_sequence::check_grading(source, &cq_sequence_tasks::machine())?;
        cq_sequence_tasks::translate(source, &analysis)
    }).into()
}
