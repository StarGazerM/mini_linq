//! User-facing macros for the MiniLinq teaching compiler.
//!
//! Each macro parses its exact source IR and routes that typed value to the
//! corresponding compiler pass. Syntax errors become Rust compile errors;
//! typed compilation entries check semantic well-formedness in every build.
//!
//! Language entries parse their own source language. Generated continuations use
//! `mini_linq::stage!`, resolved in the invocation scope. By default that names
//! this crate; `use wiring as mini_linq` can select a module of macro re-exports.
//! `materialized_hash_program!` also accepts `access macro_path;` for its set-valued Access holes.
//! Replacements owe the same observation and binding contracts as the defaults;
//! a resolving alias and compatible tokens alone do not establish those laws.

use proc_macro::TokenStream;
#[cfg(feature = "materialized-hash")]
mod access_input;
#[cfg(any(
    feature = "defaults",
    feature = "written-order",
    feature = "clause-prefix",
    feature = "hypergraph",
    feature = "base-index-demands",
    feature = "result-index-demands",
    feature = "base-hash",
    feature = "prefix-hash",
    feature = "materialized-hash",
    feature = "configured",
    feature = "borrowed-nested",
    feature = "no-index-demands",
    feature = "replay-resource",
    feature = "pipeline-exists",
    feature = "scan-candidates",
    feature = "prune-block-candidates",
    feature = "exact-keys",
    feature = "summarized-exact-keys",
    feature = "hash-witnesses",
    feature = "nested-witnesses",
    feature = "hash-pairs",
    feature = "nested-pairs",
    feature = "sort-merge-pairs",
    feature = "array-hash-rows",
    feature = "tuple-hash-rows",
    feature = "tuple-sorted-rows",
    feature = "array-sorted-rows",
    feature = "array-block-rows",
    feature = "access-to-pipeline",
    feature = "access-to-set-loops",
    feature = "pipeline-distinct-iter",
    feature = "pipeline-set",
    feature = "pipeline-support-iter",
))]
mod entry;
mod workload;
#[cfg(feature = "materialized-hash")]
use access_input::AccessInput;

/// Own one workload query as real Rust macro input.
///
/// The default expansion exposes only the typed-CQ factory used by the catalog,
/// so the workload crate remains usable before students complete R1. The
/// `compiled-workloads` feature additionally emits the complete generated query
/// program through the same caller-resolved macro route as [`mini_linq`].
#[proc_macro]
pub fn workload_query(input: TokenStream) -> TokenStream {
    workload::expand(input)
}

/// Select a logical planner without bypassing its caller-resolved macro binding.
#[cfg(feature = "defaults")]
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
#[cfg(feature = "defaults")]
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

/// Indexed RA-to-staged-Rust pass; local holes return exact sets.
/// The default `access_to_set` continuation combines terminal distinctness and
/// collection. An `access path;` override must satisfy that same set contract.
#[cfg(feature = "materialized-hash")]
#[proc_macro]
pub fn materialized_hash_program(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_relation_lowering::materialized_hash::compile(&input.source, &input.access)
        },
    )
}

/// Lower indexed RA to an anonymous Rust closure returning an exact set.
/// Call it with one iterator per input declaration, in order. Each closure value
/// has Rust-inferred input types and ordinary capture semantics; no generated
/// item name enters the query expressions' scope.
#[cfg(feature = "materialized-hash")]
#[proc_macro]
pub fn materialized_hash_callable(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_relation_lowering::materialized_hash::compile_callable(&input.source, &input.access)
        },
    )
}

/// Select base-input indexes for the admitted linear RA domain.
#[cfg(feature = "base-index-demands")]
#[proc_macro]
pub fn base_index_demands(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_relation_lowering)),
        cq_index_policy::base_demands::compile,
    )
}

/// Select indexes on named RA results without selecting a physical implementation.
#[cfg(feature = "result-index-demands")]
#[proc_macro]
pub fn result_index_demands(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_relation_lowering)),
        cq_index_policy::result_demands::compile,
    )
}

/// Explicit base-input indexed physical implementation.
#[cfg(feature = "base-hash")]
#[proc_macro]
pub fn lower_base_hash(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source| {
        cq_relation_lowering::base_hash::compile(
            source,
            &syn::parse_quote!(mini_linq::access_to_distinct_iter),
        )
    })
}

/// Explicit lazy physical implementation for a linear relational prefix.
#[cfg(feature = "prefix-hash")]
#[proc_macro]
pub fn lower_prefix_hash(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source| {
        cq_relation_lowering::prefix_hash::compile(
            source,
            &syn::parse_quote!(mini_linq::access_to_distinct_iter),
        )
    })
}

/// Select an index policy without bypassing its caller-resolved macro binding.
#[cfg(feature = "defaults")]
#[proc_macro]
pub fn choose_index_policy(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source| {
        Ok(if cq_compose::use_input_indexes(source)? {
            quote::quote!(mini_linq::base_index_demands! { #source })
        } else {
            quote::quote!(mini_linq::result_index_demands! { #source })
        })
    })
}

/// Select a realization from the actual plan and written index requirements.
/// No earlier policy choice is encoded as a route tag or hidden continuation.
#[cfg(feature = "defaults")]
#[proc_macro]
pub fn choose_relation_lowering(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::index_requirements::Module| {
        cq_ir::index_requirements::contract::check(source)?;
        Ok(
            if cq_relation_lowering::base_hash::check_domain(source).is_ok() {
                quote::quote!(mini_linq::lower_base_hash! { #source })
            } else if cq_relation_lowering::prefix_hash::check_domain(source).is_ok() {
                quote::quote!(mini_linq::lower_prefix_hash! { #source })
            } else {
                quote::quote!(mini_linq::materialized_hash_program! { #source })
            },
        )
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
///     R(x) => for (x,) in ([(1,)]),
///     let x = 2 => for (x,) in ([(2,)]);
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

/// Consume an Access plan into an exact `HashSet` through the existing access_to_distinct_iter
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

/// Consume a named pipeline into its exact `HashSet`, folding terminal
/// distinctness into the set sink. The source language is IteratorPipeline.
#[cfg(feature = "pipeline-set")]
#[proc_macro]
pub fn pipeline_to_set(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_pipeline_lowering::collect_set::compile)
}

/// Lower an explicit named physical plan to its actual ordinary-Rust AST.
///
/// The compiler writes one standard iterator-adapter local per named operator.
/// This proc-macro performs no further code generation: quoting the returned
/// [`syn::Expr`] is its direct `ToTokens` representation.
#[cfg(feature = "pipeline-distinct-iter")]
#[proc_macro]
pub fn pipeline_to_distinct_iter(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_pipeline_lowering::distinct_iter::compile)
}

#[cfg(any(
    feature = "scan-candidates",
    feature = "prune-block-candidates",
    feature = "exact-keys",
    feature = "summarized-exact-keys",
    feature = "hash-witnesses",
    feature = "nested-witnesses",
    feature = "hash-pairs",
    feature = "nested-pairs",
    feature = "sort-merge-pairs",
    feature = "array-hash-rows",
    feature = "tuple-hash-rows",
    feature = "tuple-sorted-rows",
    feature = "array-sorted-rows",
    feature = "array-block-rows",
    feature = "replay-resource"
))]
mod native_input;
#[cfg(feature = "configured")]
mod relation_input;

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

#[cfg(any(
    feature = "prune-block-candidates",
    feature = "exact-keys",
    feature = "summarized-exact-keys",
    feature = "hash-witnesses",
    feature = "nested-witnesses",
    feature = "hash-pairs",
    feature = "nested-pairs",
    feature = "sort-merge-pairs",
    feature = "array-hash-rows",
    feature = "tuple-hash-rows",
    feature = "tuple-sorted-rows",
    feature = "array-sorted-rows",
    feature = "array-block-rows",
    feature = "replay-resource"
))]
macro_rules! native_entry {
    ($name:ident, $compile:path, $options:ident, $doc:literal) => {
        #[doc = $doc]
        #[proc_macro]
        pub fn $name(input: TokenStream) -> TokenStream {
            entry::stage(
                input,
                None,
                |input: &native_input::Input<cq_ir::native::Operation>| {
                    $compile(input.source(), &input.$options()?)
                },
            )
        }
    };
}

/// Candidate scan requires no storage, collection or iterator configuration.
#[cfg(feature = "scan-candidates")]
#[proc_macro]
pub fn scan_candidates(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        None,
        |input: &native_input::Input<cq_ir::native::Operation>| {
            cq_native_lowering::candidates::scan::compile(input.scan_source()?)
        },
    )
}

#[cfg(feature = "prune-block-candidates")]
native_entry!(
    prune_block_candidates,
    cq_native_lowering::candidates::prune::compile,
    prune_options,
    "Prune through block metadata before opening selected payload."
);

#[cfg(feature = "exact-keys")]
native_entry!(
    exact_keys,
    cq_native_lowering::keys::exact::compile,
    exact_options,
    "Construct and probe an exact key resource."
);

#[cfg(feature = "summarized-exact-keys")]
native_entry!(
    summarized_exact_keys,
    cq_native_lowering::keys::zone::compile,
    zone_options,
    "Construct exact keys with a conservative range summary."
);

#[cfg(feature = "hash-witnesses")]
native_entry!(
    hash_witnesses,
    cq_native_lowering::witnesses::hash::compile,
    hash_witness_options,
    "Construct and probe hash witness buckets."
);

#[cfg(feature = "nested-witnesses")]
native_entry!(
    nested_witnesses,
    cq_native_lowering::witnesses::nested::compile,
    nested_witness_options,
    "Defer witness traversal until each probe."
);

#[cfg(feature = "array-hash-rows")]
native_entry!(
    array_hash_rows,
    cq_native_lowering::rows::hash::compile,
    hash_row_options,
    "Native hash-set storage operations."
);

#[cfg(feature = "hash-pairs")]
native_entry!(
    hash_pairs,
    cq_native_lowering::pairs::hash::compile,
    pair_options,
    "Build and probe equal-key pairs using hash buckets."
);

#[cfg(feature = "nested-pairs")]
native_entry!(
    nested_pairs,
    cq_native_lowering::pairs::nested::compile,
    pair_options,
    "Enumerate equal-key pairs using nested traversal."
);

#[cfg(feature = "sort-merge-pairs")]
native_entry!(
    sort_merge_pairs,
    cq_native_lowering::pairs::merge::compile,
    pair_options,
    "Sort both inputs and merge their equal-key groups."
);

#[cfg(feature = "array-sorted-rows")]
native_entry!(
    array_sorted_rows,
    cq_native_lowering::rows::sorted::compile,
    sorted_row_options,
    "Native sorted-set storage operations."
);

#[cfg(feature = "tuple-hash-rows")]
native_entry!(
    tuple_hash_rows,
    cq_native_lowering::rows::tuple_hash::compile,
    tuple_hash_row_options,
    "Heterogeneous tuple rows in native hash-set storage."
);

#[cfg(feature = "tuple-sorted-rows")]
native_entry!(
    tuple_sorted_rows,
    cq_native_lowering::rows::tuple_sorted::compile,
    tuple_sorted_row_options,
    "Heterogeneous tuple rows in native sorted-set storage."
);

#[cfg(feature = "array-block-rows")]
native_entry!(
    array_block_rows,
    cq_native_lowering::rows::block::compile,
    block_options,
    "Borrowed block storage and selection through operation traits."
);

/// Compile configured RA into a plain query program.
#[cfg(feature = "configured")]
#[proc_macro]
pub fn configured_program(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &relation_input::RelationInput| {
        cq_relation_lowering::configured::compile_program(&input.source, &input.options()?)
    })
}

/// Lower an empty-index plan through borrowed Access and Native boundaries.
/// Input R binds mini_linq::inputs::R; each execution/resource boundary remains
/// an independently resolved macro invocation in the generated Rust.
#[cfg(feature = "borrowed-nested")]
#[proc_macro]
pub fn lower_borrowed_nested(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::index_requirements::Module| {
        cq_relation_lowering::borrowed_nested::compile(
            source,
            &cq_relation_lowering::borrowed_nested::Options {
                inputs: entry::relation_macros(&source.relational_plan),
                stream: syn::parse_quote!(mini_linq::access_to_support_iter),
                output: syn::parse_quote!(mini_linq::access_to_distinct_iter),
                exists: syn::parse_quote!(mini_linq::access_to_exists),
                replay: syn::parse_quote!(mini_linq::replay_resource),
            },
        )
    })
}

/// Compile configured RA into a callable query with ordinary Rust captures.
#[cfg(feature = "configured")]
#[proc_macro]
pub fn configured_callable(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &relation_input::RelationInput| {
        cq_relation_lowering::configured::compile_query(&input.source, &input.options()?)
    })
}

/// Compile configured RA into an explicitly observed query program.
#[cfg(feature = "configured")]
#[proc_macro]
pub fn observed_configured_program(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &relation_input::RelationInput| {
        cq_relation_lowering::configured::compile_observed_program(
            &input.source,
            &input.observed_options()?,
        )
    })
}

/// Compile configured RA into an explicitly observed callable query.
#[cfg(feature = "configured")]
#[proc_macro]
pub fn observed_configured_callable(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &relation_input::RelationInput| {
        cq_relation_lowering::configured::compile_observed_query(
            &input.source,
            &input.observed_options()?,
        )
    })
}

/// Request no equality indexes while retaining the IndexRequirements boundary.
#[cfg(feature = "no-index-demands")]
#[proc_macro]
pub fn no_index_demands(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_relation_lowering)),
        cq_index_policy::no_demands::compile,
    )
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

#[cfg(feature = "replay-resource")]
native_entry!(
    replay_resource,
    cq_native_lowering::rows::replay::compile,
    replay_options,
    "Retain demanded Access rows in a replay resource; scan with owning cursors."
);
