//! Test-only admissible alternatives, never selected by the default pipeline.
//! Every rewrite uses the ordinary checked `cq_stage::rewrite` boundary.
//! `eval_source!` is an independent typed reference, not a production printer.
mod reference;
mod reference_incremental;
mod reference_round;
#[cfg(test)]
mod reference_tests;
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

/// Select ordered-map layout providers without changing their read bindings.
#[proc_macro]
pub fn ordered_storage(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        rewrites::ordered_storage(source)
    })
    .into()
}

/// Read a later delta occurrence directly instead of through its grouped layout.
#[proc_macro]
pub fn direct_later_delta(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        rewrites::direct_later_delta(source)
    })
    .into()
}

/// Publish component members in the opposite admissible order.
#[proc_macro]
pub fn reverse_publication(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        rewrites::reverse_publication(source)
    })
    .into()
}

/// Independently select two rule holes: `(batch_hole, positions_hole)`.
/// The first uses `stage(batch(64))`; the second uses `represent(positions)`.
#[proc_macro]
pub fn per_hole_pipeline(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(input.into(), rewrites::per_hole_pipeline)
        .into()
}

/// Opt every pull hole into the registered, partitioned pull terminal.
#[proc_macro]
pub fn partition_holes(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        rewrites::partition_holes(source)
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

/// Execute the recorded semi-naive Incremental program as typed set relations.
#[proc_macro]
pub fn eval_incremental(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::pass::<cq_lang_datalog_incremental::Program, syn::Expr>(
        input.into(),
        |program, args| {
            no_arguments(args)?;
            syn::parse2(reference_incremental::emit(program)?)
        },
    )
    .into()
}

/// Interpret recorded Round phases and embedded CQ holes, without a printer.
#[proc_macro]
pub fn eval_round(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(input.into(), |program, args| {
        no_arguments(args)?;
        syn::parse2(reference_round::emit(program)?)
    })
    .into()
}

/// Evaluate the Incremental meaning retained inside a checked Round program.
/// This terminal permits identical corpus inputs at each pipeline boundary.
#[proc_macro]
pub fn eval_incremental_from_round(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(input.into(), |program, args| {
        no_arguments(args)?;
        cq_lang_datalog_round::contract::check(program)?;
        syn::parse2(reference_incremental::emit(&program.storage.incremental)?)
    })
    .into()
}

/// Evaluate the Source meaning retained inside a checked Round program.
#[proc_macro]
pub fn eval_source_from_round(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(input.into(), |program, args| {
        no_arguments(args)?;
        cq_lang_datalog_round::contract::check(program)?;
        syn::parse2(reference::emit(&program.storage.incremental.strata.source)?)
    })
    .into()
}

/// Evaluate the recorded component order retained inside checked Round.
#[proc_macro]
pub fn eval_strata_from_round(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(input.into(), |program, args| {
        no_arguments(args)?;
        cq_lang_datalog_round::contract::check(program)?;
        syn::parse2(reference::emit_strata(&program.storage.incremental.strata)?)
    })
    .into()
}
