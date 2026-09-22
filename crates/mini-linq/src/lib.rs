//! Procedural entry points at the compiler's explicit language boundaries.
//!
//! CQ planning produces RA; IX records index definitions and uses; Physical
//! records per-relation/index providers and per-operation lowering rules.
//! Realization invokes those external definitions and returns Rust[Access].
//! Caller import aliases bind policy and execution slots. No global storage
//! default or fallback whole-query backend is supplied.

use proc_macro::TokenStream;
#[cfg(any(
    feature = "logical-selection",
    feature = "written-order",
    feature = "clause-prefix",
    feature = "hypergraph",
    feature = "index-demands",
    feature = "physical-selection",
    feature = "physical-realization",
    feature = "access-to-pipeline",
    feature = "access-to-set-loops",
    feature = "pipeline-distinct-iter",
    feature = "pipeline-set",
    feature = "pipeline-support-iter",
    feature = "pipeline-exists"
))]
mod entry;
mod workload;

/// Capture a typed CQ for the workload catalog without compiling a host API.
#[proc_macro]
pub fn workload_query(input: TokenStream) -> TokenStream {
    workload::expand(input)
}

/// Select a logical planner without bypassing its caller-resolved macro binding.
#[cfg(feature = "logical-selection")]
#[proc_macro]
pub fn mini_linq(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::cq::Module| {
        cq_ir::cq::contract::check(source)?;
        Ok(if source.program.query.is_basic() {
            quote::quote!(mini_linq::cq_to_ra_written_order! { #source })
        } else {
            quote::quote!(mini_linq::cq_to_ra_clause_prefix! { #source })
        })
    })
}

/// Canonical CQ-to-RA rules for variable-only conjunctive queries.
#[cfg(feature = "written-order")]
#[proc_macro]
pub fn cq_to_ra_written_order(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_index_policy)),
        cq_query_lowering::written_order::compile,
    )
}

/// Source-order clause-prefix CQ lowering, selected independently of source shape.
#[cfg(feature = "clause-prefix")]
#[proc_macro]
pub fn cq_to_ra_clause_prefix(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_index_policy)),
        cq_query_lowering::clause_prefix::compile,
    )
}

/// Graph planning, including YA reduction when the query is acyclic.
#[cfg(feature = "hypergraph")]
#[proc_macro]
pub fn cq_to_ra_hypergraph(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_index_policy)),
        cq_query_lowering::hypergraph::compile,
    )
}

/// Select graph or expression-preserving prefix planning through caller bindings.
/// Evaluation demand belongs to the downstream composition, not this selector.
#[cfg(feature = "logical-selection")]
#[proc_macro]
pub fn choose_graph_or_prefix(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::cq::Module| {
        cq_ir::cq::contract::check(source)?;
        let facts = cq_query_lowering::dependencies::ClauseDependencies::analyze(source)?;
        Ok(if facts.supports_graph_rules {
            quote::quote!(mini_linq::cq_to_ra_hypergraph! { #source })
        } else {
            quote::quote!(mini_linq::cq_to_ra_clause_prefix! { #source })
        })
    })
}

/// Construct exact-version index definitions and references from every join use.
#[cfg(feature = "index-demands")]
#[proc_macro]
pub fn index_demands(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_physical)),
        cq_index_policy::compile,
    )
}

/// Attach the explicit policy's complete provider and lowering assignments.
///
/// The parsed table is supplied policy configuration. This entry checks its
/// structure before exposing Physical; external provider admission is separate.
#[cfg(feature = "physical-selection")]
#[proc_macro]
pub fn given_physical_bindings(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::realize_physical)),
        |source: &cq_ir::physical::Module| {
            cq_physical_selection::given_bindings(
                source.index_requirements.clone(),
                source.assignments.clone(),
            )
        },
    )
}

/// Realize the recorded Physical program inside the caller's Rust expression context.
///
/// Input identifiers refer to host bindings in that context. The caller binds
/// `mini_linq::access` to the consumer of the completed Access quotation.
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn realize_physical(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::physical::Module| {
        cq_physical_lowering::begin(
            source.clone(),
            syn::parse_quote!(::mini_linq::resume_physical),
            syn::parse_quote!(mini_linq::access),
        )
    })
}

/// Resume a selected external provider's compile-time reply.
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn resume_physical(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |reply: &cq_ir::provider_protocol::Reply| {
        cq_physical_lowering::resume(reply.clone())
    })
}

/// Lower Rename using its selected representation.
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn rename(input: TokenStream) -> TokenStream {
    cq_physical_lowering::rename::compile(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Lower NaturalJoin through exact Rows/Matches/Output operations.
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn indexed_join(input: TokenStream) -> TokenStream {
    cq_physical_lowering::indexed_join::compile(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Lower terminal Project using its selected output representation.
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn project(input: TokenStream) -> TokenStream {
    cq_physical_lowering::project::lower(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[doc(hidden)]
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn indexed_join_rows(input: TokenStream) -> TokenStream {
    cq_physical_lowering::indexed_join::rows(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[doc(hidden)]
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn indexed_join_matches(input: TokenStream) -> TokenStream {
    cq_physical_lowering::indexed_join::matches(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[doc(hidden)]
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn indexed_join_output(input: TokenStream) -> TokenStream {
    cq_physical_lowering::indexed_join::output(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[doc(hidden)]
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn project_rows(input: TokenStream) -> TokenStream {
    cq_physical_lowering::project::rows(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[doc(hidden)]
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn project_output(input: TokenStream) -> TokenStream {
    cq_physical_lowering::project::output(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Execute an Access plan with eager nested loops and an exact-set sink.
/// This execution implementation does not pass through IteratorPipeline.
#[cfg(feature = "access-to-set-loops")]
#[proc_macro]
pub fn access_to_set_loops(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_access_lowering::to_set_loops::compile)
}

/// Expose the left-deep iterator operators of an annotated Rust access plan.
///
/// This stage produces an independently invocable [`pipeline_to_distinct_iter!`]
/// macro rather than emitting Rust directly, so each binary join and its
/// intermediate binding remain visible in macro expansion.
/// Let accesses introduce fresh logical names, including in direct quotations:
///
/// ```compile_fail
/// let _rows = mini_linq::access_to_distinct_iter! {
///     out(x) => ((x,)) :-
///     R(x) => bind (x,) <- ([(1,)]),
///     let x = 2 => bind (x,) <- ([(2,)]);
/// };
/// ```
#[cfg(feature = "access-to-pipeline")]
#[proc_macro]
pub fn access_to_distinct_iter(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::pipeline_to_distinct_iter)),
        cq_access_lowering::to_pipeline::compile,
    )
}

/// Consume an Access plan into the selected exact-set sink through access_to_distinct_iter
/// compiler and the independently selectable `pipeline_to_set` continuation.
#[cfg(feature = "access-to-pipeline")]
#[proc_macro]
pub fn access_to_set(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::pipeline_to_set)),
        cq_access_lowering::to_pipeline::compile,
    )
}

/// Consume a named pipeline into its selected exact-set sink, folding terminal
/// distinctness into the set sink. The source language is IteratorPipeline.
/// Requires an explicit `mini_linq::set_sink!` binding satisfying the exact-set law.
/// An otherwise valid program without that binding fails:
///
/// ```compile_fail
/// let _ = mini_linq::pipeline_to_set! {
///     a = scan x in ([1, 1]) yield (x,);
///     b = distinct a;
///     return b;
/// };
/// ```
#[cfg(feature = "pipeline-set")]
#[proc_macro]
pub fn pipeline_to_set(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_pipeline_lowering::collect_set::compile)
}

/// Lower an explicit named physical plan to its actual ordinary-Rust AST.
///
/// The compiler writes one standard iterator-adapter local per named operator.
/// The expression contains caller-resolved `mini_linq::distinct_state!` requests.
/// The selected provider expands state operations; there is no default.
///
/// ```compile_fail
/// let _ = mini_linq::pipeline_to_distinct_iter! {
///     a = scan x in ([1, 1]) yield (x,);
///     b = distinct a;
///     return b;
/// };
/// ```
#[cfg(feature = "pipeline-distinct-iter")]
#[proc_macro]
pub fn pipeline_to_distinct_iter(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_pipeline_lowering::distinct_iter::compile)
}

/// The Access-to-Pipeline edge with a support-stream continuation.
#[cfg(feature = "access-to-pipeline")]
#[proc_macro]
pub fn access_to_support_iter(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::pipeline_to_support_iter)),
        cq_access_lowering::to_pipeline::compile,
    )
}

/// Enumerate Pipeline support; the enclosing consumer owns distinctness.
#[cfg(feature = "pipeline-support-iter")]
#[proc_macro]
pub fn pipeline_to_support_iter(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_pipeline_lowering::support_iter::compile)
}

/// Lower Access through Pipeline to an existential consumer.
#[cfg(feature = "access-to-pipeline")]
#[proc_macro]
pub fn access_to_exists(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::pipeline_to_exists)),
        cq_access_lowering::to_pipeline::compile,
    )
}

/// Consume a checked Pipeline only until its first witness.
#[cfg(feature = "pipeline-exists")]
#[proc_macro]
pub fn pipeline_to_exists(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_pipeline_lowering::exists::compile)
}

#[cfg(test)]
mod native_input_tests;

#[cfg(test)]
mod relation_input_tests;
