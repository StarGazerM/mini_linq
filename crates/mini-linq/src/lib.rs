//! User-facing macros for the MiniLinq teaching compiler.
//!
//! Each macro parses its exact source IR and routes that typed value to the
//! corresponding compiler pass. Syntax errors become Rust compile errors;
//! typed compilation entries check semantic well-formedness in every build.
//!
//! Language entries parse their own source language. Generated continuations use
//! `mini_linq::stage!`, resolved in the invocation scope. By default that names
//! this crate; `use wiring as mini_linq` can select a module of macro re-exports.
//! `hash_materialized!` also accepts `access macro_path;` for its set-valued Access holes.
//! Replacements owe the same observation and binding contracts as the defaults;
//! a resolving alias and compatible tokens alone do not establish those laws.

use proc_macro::TokenStream;
#[cfg(feature = "hash-materialized")]
mod access_input;
#[cfg(any(
    feature = "defaults",
    feature = "canonical",
    feature = "prefix",
    feature = "graph",
    feature = "input-indexes",
    feature = "result-indexes",
    feature = "hash-inputs",
    feature = "hash-prefix",
    feature = "hash-materialized",
    feature = "relations",
    feature = "scan-rows",
    feature = "prune-rows",
    feature = "exact-keys",
    feature = "zone-keys",
    feature = "hash-witnesses",
    feature = "nested-witnesses",
    feature = "hash-pairs",
    feature = "nested-pairs",
    feature = "sort-merge-pairs",
    feature = "hash-rows",
    feature = "tuple-hash-rows",
    feature = "tuple-sorted-rows",
    feature = "sorted-rows",
    feature = "block-rows",
    feature = "pull",
    feature = "push-set",
    feature = "iterator-pipeline",
    feature = "collect-pipeline",
    feature = "stream-pipeline",
))]
mod entry;
mod workload;
#[cfg(feature = "hash-materialized")]
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
            quote::quote!(mini_linq::canonical_query! { #source })
        } else {
            quote::quote!(mini_linq::prefix_query! { #source })
        })
    })
}

/// Canonical CQ-to-RA rules for variable-only conjunctive queries.
#[cfg(feature = "canonical")]
#[proc_macro]
pub fn canonical_query(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_indexes)),
        cq_plan::canonical::compile,
    )
}

/// Source-order clause-prefix CQ lowering, selected independently of source shape.
#[cfg(feature = "prefix")]
#[proc_macro]
pub fn prefix_query(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_indexes)),
        cq_plan::prefix::compile,
    )
}

/// Graph planning, including YA reduction when the query is acyclic.
#[cfg(feature = "graph")]
#[proc_macro]
pub fn graph_query(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_indexes)),
        cq_plan::graph::compile,
    )
}

/// Select graph or expression-preserving prefix planning through caller bindings.
/// Evaluation demand belongs to the downstream composition, not this selector.
#[cfg(feature = "defaults")]
#[proc_macro]
pub fn optimized_query(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::cq::Module| {
        cq_ir::cq::contract::check(source)?;
        let facts = cq_plan::dependencies::ClauseDependencies::analyze(source)?;
        Ok(if facts.supports_graph_rules {
            quote::quote!(mini_linq::graph_query! { #source })
        } else {
            quote::quote!(mini_linq::prefix_query! { #source })
        })
    })
}

/// Indexed RA-to-staged-Rust pass; local holes return exact sets.
/// The default `pull_set` continuation combines terminal distinctness and
/// collection. An `access path;` override must satisfy that same set contract.
#[cfg(feature = "hash-materialized")]
#[proc_macro]
pub fn hash_materialized(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_realize::hash_materialized::compile_materialized(&input.source, &input.access)
        },
    )
}

/// Lower indexed RA to an anonymous Rust closure returning an exact set.
/// Call it with one iterator per input declaration, in order. Each closure value
/// has Rust-inferred input types and ordinary capture semantics; no generated
/// item name enters the query expressions' scope.
#[cfg(feature = "hash-materialized")]
#[proc_macro]
pub fn hash_materialized_query(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_realize::hash_materialized::compile_materialized_callable(
                &input.source,
                &input.access,
            )
        },
    )
}

/// Select base-input indexes for the admitted linear RA domain.
#[cfg(feature = "input-indexes")]
#[proc_macro]
pub fn input_indexes(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_realization)),
        cq_index::input::compile,
    )
}

/// Select indexes on named RA results without selecting a physical implementation.
#[cfg(feature = "result-indexes")]
#[proc_macro]
pub fn result_indexes(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_realization)),
        cq_index::result::compile,
    )
}

/// Explicit base-input indexed physical implementation.
#[cfg(feature = "hash-inputs")]
#[proc_macro]
pub fn hash_inputs(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source| {
        cq_realize::hash_inputs::compile_input_indexed(source, &syn::parse_quote!(mini_linq::pull))
    })
}

/// Explicit lazy physical implementation for a linear relational prefix.
#[cfg(feature = "hash-prefix")]
#[proc_macro]
pub fn hash_prefix(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source| {
        cq_realize::hash_prefix::compile_lazy_indexed(source, &syn::parse_quote!(mini_linq::pull))
    })
}

/// Select an index policy without bypassing its caller-resolved macro binding.
#[cfg(feature = "defaults")]
#[proc_macro]
pub fn choose_indexes(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source| {
        Ok(if cq_compose::use_input_indexes(source)? {
            quote::quote!(mini_linq::input_indexes! { #source })
        } else {
            quote::quote!(mini_linq::result_indexes! { #source })
        })
    })
}

/// Select a realization from the actual plan and written index requirements.
/// No earlier policy choice is encoded as a route tag or hidden continuation.
#[cfg(feature = "defaults")]
#[proc_macro]
pub fn choose_realization(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::index_requirements::Module| {
        cq_ir::index_requirements::contract::check(source)?;
        Ok(if cq_realize::hash_inputs::check_domain(source).is_ok() {
            quote::quote!(mini_linq::hash_inputs! { #source })
        } else if cq_realize::hash_prefix::check_domain(source).is_ok() {
            quote::quote!(mini_linq::hash_prefix! { #source })
        } else {
            quote::quote!(mini_linq::hash_materialized! { #source })
        })
    })
}

/// Execute an Access plan with eager nested loops and an exact-set sink.
/// This execution implementation does not pass through IteratorPipeline.
#[cfg(feature = "push-set")]
#[proc_macro]
pub fn push_set(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_execution::push::compile)
}

/// Expose the left-deep iterator operators of an annotated Rust access plan.
///
/// This stage produces an independently invocable [`iterator_pipeline!`]
/// macro rather than emitting Rust directly, so each binary join and its
/// intermediate binding remain visible in macro expansion.
/// Let accesses introduce fresh logical names, including in direct quotations:
///
/// ```compile_fail
/// let _rows = mini_linq::pull! {
///     out(x) => ((x,)) :-
///     R(x) => for (x,) in ([(1,)]),
///     let x = 2 => for (x,) in ([(2,)]);
/// };
/// ```
#[cfg(feature = "pull")]
#[proc_macro]
pub fn pull(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::iterator_pipeline)),
        cq_execution::pull::compile,
    )
}

/// Consume an Access plan into an exact `HashSet` through the existing pull
/// compiler and the independently selectable `collect_pipeline` continuation.
#[cfg(feature = "pull")]
#[proc_macro]
pub fn pull_set(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::collect_pipeline)),
        cq_execution::pull::compile,
    )
}

/// Consume a named pipeline into its exact `HashSet`, folding terminal
/// distinctness into the set sink. The source language is IteratorPipeline.
#[cfg(feature = "collect-pipeline")]
#[proc_macro]
pub fn collect_pipeline(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_pipeline::collect::compile)
}

/// Lower an explicit named physical plan to its actual ordinary-Rust AST.
///
/// The compiler writes one standard iterator-adapter local per named operator.
/// This proc-macro performs no further code generation: quoting the returned
/// [`syn::Expr`] is its direct `ToTokens` representation.
#[cfg(feature = "iterator-pipeline")]
#[proc_macro]
pub fn iterator_pipeline(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_pipeline::iterator::compile)
}

#[cfg(any(
    feature = "scan-rows",
    feature = "prune-rows",
    feature = "exact-keys",
    feature = "zone-keys",
    feature = "hash-witnesses",
    feature = "nested-witnesses",
    feature = "hash-pairs",
    feature = "nested-pairs",
    feature = "sort-merge-pairs",
    feature = "hash-rows",
    feature = "tuple-hash-rows",
    feature = "tuple-sorted-rows",
    feature = "sorted-rows",
    feature = "block-rows"
))]
mod native_input;
#[cfg(feature = "relations")]
mod relation_input;

/// The Access-to-Pipeline edge with a support-stream continuation.
#[cfg(feature = "pull")]
#[proc_macro]
pub fn pull_stream(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::stream_pipeline)),
        cq_execution::pull::compile,
    )
}

/// Enumerate Pipeline support; the enclosing consumer owns distinctness.
#[cfg(feature = "stream-pipeline")]
#[proc_macro]
pub fn stream_pipeline(input: TokenStream) -> TokenStream {
    entry::stage(input, None, cq_pipeline::stream::compile)
}

#[cfg(any(
    feature = "prune-rows",
    feature = "exact-keys",
    feature = "zone-keys",
    feature = "hash-witnesses",
    feature = "nested-witnesses",
    feature = "hash-pairs",
    feature = "nested-pairs",
    feature = "sort-merge-pairs",
    feature = "hash-rows",
    feature = "tuple-hash-rows",
    feature = "tuple-sorted-rows",
    feature = "sorted-rows",
    feature = "block-rows"
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
#[cfg(feature = "scan-rows")]
#[proc_macro]
pub fn scan_rows(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        None,
        |input: &native_input::Input<cq_ir::native::Operation>| {
            cq_native::candidates::scan::compile(input.scan_source()?)
        },
    )
}

#[cfg(feature = "prune-rows")]
native_entry!(
    prune_rows,
    cq_native::candidates::prune::compile,
    prune_options,
    "Prune through block metadata before opening selected payload."
);

#[cfg(feature = "exact-keys")]
native_entry!(
    exact_keys,
    cq_native::keys::exact::compile,
    exact_options,
    "Construct and probe an exact key resource."
);

#[cfg(feature = "zone-keys")]
native_entry!(
    zone_keys,
    cq_native::keys::zone::compile,
    zone_options,
    "Construct exact keys with a conservative range summary."
);

#[cfg(feature = "hash-witnesses")]
native_entry!(
    hash_witnesses,
    cq_native::witnesses::hash::compile,
    hash_witness_options,
    "Construct and probe hash witness buckets."
);

#[cfg(feature = "nested-witnesses")]
native_entry!(
    nested_witnesses,
    cq_native::witnesses::nested::compile,
    nested_witness_options,
    "Defer witness traversal until each probe."
);

#[cfg(feature = "hash-rows")]
native_entry!(
    hash_rows,
    cq_native::rows::hash::compile,
    hash_row_options,
    "Native hash-set storage operations."
);

#[cfg(feature = "hash-pairs")]
native_entry!(
    hash_pairs,
    cq_native::pairs::hash::compile,
    pair_options,
    "Build and probe equal-key pairs using hash buckets."
);

#[cfg(feature = "nested-pairs")]
native_entry!(
    nested_pairs,
    cq_native::pairs::nested::compile,
    pair_options,
    "Enumerate equal-key pairs using nested traversal."
);

#[cfg(feature = "sort-merge-pairs")]
native_entry!(
    sort_merge_pairs,
    cq_native::pairs::merge::compile,
    pair_options,
    "Sort both inputs and merge their equal-key groups."
);

#[cfg(feature = "sorted-rows")]
native_entry!(
    sorted_rows,
    cq_native::rows::sorted::compile,
    sorted_row_options,
    "Native sorted-set storage operations."
);

#[cfg(feature = "tuple-hash-rows")]
native_entry!(
    tuple_hash_rows,
    cq_native::rows::tuple_hash::compile,
    tuple_hash_row_options,
    "Heterogeneous tuple rows in native hash-set storage."
);

#[cfg(feature = "tuple-sorted-rows")]
native_entry!(
    tuple_sorted_rows,
    cq_native::rows::tuple_sorted::compile,
    tuple_sorted_row_options,
    "Heterogeneous tuple rows in native sorted-set storage."
);

#[cfg(feature = "block-rows")]
native_entry!(
    block_rows,
    cq_native::rows::block::compile,
    block_options,
    "Borrowed block storage and selection through operation traits."
);

/// Compile configured RA into a plain query program.
#[cfg(feature = "relations")]
#[proc_macro]
pub fn relation_program(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &relation_input::RelationInput| {
        cq_realize::relations::compile_program(&input.source, &input.options()?)
    })
}

/// Compile configured RA into a callable query with ordinary Rust captures.
#[cfg(feature = "relations")]
#[proc_macro]
pub fn relation_query(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &relation_input::RelationInput| {
        cq_realize::relations::compile_query(&input.source, &input.options()?)
    })
}

/// Compile configured RA into an explicitly observed query program.
#[cfg(feature = "relations")]
#[proc_macro]
pub fn observed_relation_program(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &relation_input::RelationInput| {
        cq_realize::relations::compile_observed_program(&input.source, &input.observed_options()?)
    })
}

/// Compile configured RA into an explicitly observed callable query.
#[cfg(feature = "relations")]
#[proc_macro]
pub fn observed_relation_query(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &relation_input::RelationInput| {
        cq_realize::relations::compile_observed_query(&input.source, &input.observed_options()?)
    })
}
