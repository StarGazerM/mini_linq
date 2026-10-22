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
