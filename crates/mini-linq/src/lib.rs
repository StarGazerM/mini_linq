//! Procedural entry points at the compiler's explicit language boundaries.
//!
//! CQ planning produces RA; IX records index demands and uses; Structures
//! binds every demand to a Rust type implementing the runtime traits, under
//! a caller-supplied, query-independent policy and per-relation advice; Stream realizes every result as a typed stream
//! program directed by those structures; realization erases Stream to Access
//! over the runtime's traits. Caller import aliases bind policy and execution slots.
//! No global storage default or fallback whole-query backend is supplied.

use proc_macro::TokenStream;
#[cfg(any(
    feature = "logical-selection",
    feature = "written-order",
    feature = "clause-prefix",
    feature = "hypergraph",
    feature = "index-demands",
    feature = "structure-selection",
    feature = "stream-lowering",
    feature = "stream-realization",
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
///
/// The caller binds `mini_linq::stream_policy` to a macro that appends its
/// query-independent `policy { … }` block and invokes [`select_structures!`].
#[cfg(feature = "index-demands")]
#[proc_macro]
pub fn index_demands(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::stream_policy)),
        cq_index_policy::compile,
    )
}

/// Bind every IX demand to a runtime structure under the appended policy.
///
/// Input is `IX policy { … }`. Hosts are declared by the policy. Each demand
/// no host serves gets a structure: the type a relation's `#[index(…)]`
/// advice names, else `cq_storage::HashIndex` for joins, `HashSet` for
/// semijoins, `BTreeMap` (both operands) under `join = merge`. The policy is
/// re-appended for [`structures_to_stream!`].
#[cfg(feature = "structure-selection")]
#[proc_macro]
pub fn select_structures(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::structures_to_stream)),
        |source: &entry::Policed<cq_ir::index_requirements::Module, cq_ir::policy::Policy>| {
            let target = cq_structure_selection::compile(&source.language, &source.policy)?;
            let policy = &source.policy;
            Ok(quote::quote! { #target #policy })
        },
    )
}

/// Realize Structures as a Stream program.
///
/// Input is `Structures policy { … }`. Each RA result becomes a typed stream
/// definition whose access rule follows from the chosen structures; modes
/// follow from use. The result continues to [`stream_to_access!`].
#[cfg(feature = "stream-lowering")]
#[proc_macro]
pub fn structures_to_stream(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::stream_to_access)),
        |source: &entry::Policed<cq_ir::structures::Module, cq_ir::policy::Policy>| {
            cq_stream_lowering::compile(&source.language, &source.policy)
        },
    )
}

/// Erase a Stream program to structure statements and one Access region.
///
/// Materialized results become `let` bindings of `cq_storage` structures;
/// the returned result becomes an Access region passed to the caller's
/// `mini_linq::access` binding, inside a block that owns the structures.
#[cfg(feature = "stream-realization")]
#[proc_macro]
pub fn stream_to_access(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::stream::Module| {
        let endpoints = cq_stream_realization::Endpoints {
            support_iter: syn::parse_quote!(::mini_linq::access_to_support_iter),
        };
        let realized = cq_stream_realization::compile(source, &endpoints)?;
        let statements = &realized.statements;
        let access = &realized.access;
        Ok(quote::quote! { { #(#statements)* mini_linq::access! { #access } } })
    })
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
