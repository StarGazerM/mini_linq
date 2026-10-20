//! Test-only admissible alternatives, never selected by the default pipeline.
//! Every rewrite uses the ordinary checked `cq_stage::rewrite` boundary.
//! `eval_source!` is an independent typed reference, not a production printer.
mod reference;
mod rewrites;

fn no_arguments(arguments: proc_macro2::TokenStream) -> syn::Result<()> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            arguments,
            "this test pass takes no arguments",
        ))
    }
}

/// Eliminate identical rules, preserving their first occurrence and interface.
#[proc_macro]
pub fn deduplicate_source(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_source::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        rewrites::deduplicate_source(source)
    })
    .into()
}

/// Merge adjacent independent components without changing the immediate source.
#[proc_macro]
pub fn merge_strata(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_strata::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        rewrites::merge_strata(source)
    })
    .into()
}

/// Maintain an alternate key-zero permutation and use it for a full read.
#[proc_macro]
pub fn alternate_storage(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        rewrites::alternate_storage(source)
    })
    .into()
}

/// Evaluate checked DL-Source using native Rust and a naive set fixpoint.
/// Returns a HashSet for one output, or a tuple of HashSets for many outputs.
#[proc_macro]
pub fn eval_source(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match syn::parse::<cq_lang_datalog_source::Program>(input).and_then(|p| reference::emit(&p)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}
