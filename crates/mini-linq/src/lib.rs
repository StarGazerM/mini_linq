//! Procedural entry points at the compiler's explicit language boundaries.
//!
//! CQ planning produces RA; IX records index demands and uses; Physical
//! fixes one physical operator per result and the mode its reads allow,
//! under a caller-supplied, query-independent policy; Structures answers
//! every keyed read with a host key fact or a Rust type built over the
//! result, with per-relation `#[index]` advice; realization prints one pull
//! program over the runtime's traits. Access and Pipeline remain as
//! hand-staged entry points into the execution endpoints. Caller import
//! aliases bind policy and provider slots. No global storage default or
//! fallback whole-query backend is supplied.

use proc_macro::TokenStream;
#[cfg(any(
    feature = "logical-selection",
    feature = "written-order",
    feature = "clause-prefix",
    feature = "hypergraph",
    feature = "index-demands",
    feature = "structure-selection",
    feature = "physical-lowering",
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
///
/// The caller binds `mini_linq::physical_policy` to a macro that appends its
/// query-independent `policy { … }` block and invokes [`lower_physical!`].
#[cfg(feature = "index-demands")]
#[proc_macro]
pub fn index_demands(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::physical_policy)),
        cq_index_policy::compile,
    )
}

/// Fix one physical operator per RA result under the appended policy.
///
/// Input is `IX policy { … }`. Every natural join becomes a `probe` of the
/// IX-used side (or a `merge` under `join = merge`), every semijoin a
/// `semi`, a select bounding an ordered host key a `range`; modes follow
/// from use; the return is flagged `canon` unless uniqueness is inferred
/// from `canon` hosts. The policy is re-appended for [`select_structures!`].
#[cfg(feature = "physical-lowering")]
#[proc_macro]
pub fn lower_physical(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::select_structures)),
        |source: &entry::Policed<cq_ir::index_requirements::Module, cq_ir::policy::Policy>| {
            let target = cq_physical_lowering::compile(&source.language, &source.policy)?;
            let policy = &source.policy;
            Ok(quote::quote! { #target #policy })
        },
    )
}

/// Bind every keyed read to a value that answers it under the appended policy.
///
/// Input is `Physical policy { … }`. Host facts come from the policy (`R
/// keyed [a] ordered canon`). Each read no host key answers gets a
/// structure: the type the relation's `#[index(…)]` advice names, else
/// `cq_storage::HashIndex` for probes, `HashSet` for semijoins, `BTreeMap`
/// for merges and ranges. Whether a type implements the traits the plan uses
/// is rustc's check. Continues to [`structures_to_rust!`].
#[cfg(feature = "structure-selection")]
#[proc_macro]
pub fn select_structures(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::structures_to_rust)),
        |source: &entry::Policed<cq_ir::physical::Module, cq_ir::policy::Policy>| {
            cq_structure_selection::compile(&source.language, &source.policy)
        },
    )
}

/// Print Structures as one pull program: a block that builds the chosen
/// structures, guards the traits each read needs, and evaluates to an
/// iterator of the answer tuples in head order. A `canon` return
/// deduplicates through the caller's `mini_linq::distinct_state!` provider.
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn structures_to_rust(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::structures::Module| {
        let endpoints = cq_physical_realization::Endpoints {
            distinct_state: syn::parse_quote!(mini_linq::distinct_state),
        };
        cq_physical_realization::compile(source, &endpoints)
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
