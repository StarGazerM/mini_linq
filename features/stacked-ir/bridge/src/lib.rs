//! Public protocol macros for the external Trace language.
//!
//! `enter` consumes Physical and produces Trace; `leave` consumes Trace and
//! produces Physical. Both forward the caller's explicit remaining queue.
//! `round_trip` is a checked Physical rewrite for a named facade pipeline.
//! It composes the same two passes without choosing any continuation path.
//! These passes accept no arguments and never choose runtime representations.

use cq_lang_physical as physical;
use proc_macro::TokenStream;

fn no_arguments(arguments: proc_macro2::TokenStream) -> syn::Result<()> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            arguments,
            "Trace passes accept no arguments",
        ))
    }
}

#[proc_macro]
pub fn enter(input: TokenStream) -> TokenStream {
    cq_stage::pass::<physical::Program, cq_lang_trace::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        physical_to_trace::compile(source)
    })
    .into()
}

#[proc_macro]
pub fn leave(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_trace::Program, physical::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        trace_to_physical::compile(source)
    })
    .into()
}

#[proc_macro]
pub fn round_trip(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        let trace = physical_to_trace::compile(source)?;
        trace_to_physical::compile(&trace)
    })
    .into()
}
