//! User-facing macros for the MiniLinq teaching compiler.
//!
//! Each macro parses its exact source IR and routes that typed value to the
//! corresponding compiler pass. Syntax errors become Rust compile errors;
//! typed compilation entries check semantic well-formedness in every build.
//!
//! Language entries parse their own source language. Generated continuations use
//! `mini_linq::stage!`, resolved in the invocation scope. By default that names
//! this crate; `use wiring as mini_linq` can select a module of macro re-exports.
//! `materialized!` also accepts `access macro_path;` for its set-valued Access holes.
//! Replacements owe the same observation and binding contracts as the defaults;
//! a resolving alias and compatible tokens alone do not establish those laws.

use proc_macro::TokenStream;
#[cfg(feature = "materialized")]
mod access_input;
#[cfg(any(
    feature = "defaults",
    feature = "canonical",
    feature = "prefix",
    feature = "graph",
    feature = "input-indexes",
    feature = "result-indexes",
    feature = "input-indexed",
    feature = "lazy",
    feature = "materialized",
    feature = "physical",
    feature = "scan-rows",
    feature = "prune-rows",
    feature = "exact-keys",
    feature = "zone-keys",
    feature = "hash-witnesses",
    feature = "nested-witnesses",
    feature = "hash-rows",
    feature = "sorted-rows",
    feature = "block-rows",
    feature = "pull",
    feature = "iterator-pipeline",
    feature = "collect-pipeline",
    feature = "stream-pipeline",
))]
mod entry;
mod workload;
#[cfg(feature = "materialized")]
use access_input::AccessInput;

/// Own one workload query as real Rust macro input.
///
/// The default expansion exposes only the typed-CQ factory used by the catalog,
/// so the workload crate remains usable before students complete R1. The
/// `compiled-workloads` feature additionally emits the complete generated query
/// program through the same typed passes as [`mini_linq`].
#[proc_macro]
pub fn workload_query(input: TokenStream) -> TokenStream {
    workload::expand(input)
}

/// Parse CQ syntax and route it through the typed CQ compiler pass.
#[cfg(feature = "defaults")]
#[proc_macro]
pub fn mini_linq(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::relational_plan)),
        cq_compose::compile_cq,
    )
}

/// Canonical CQ-to-RA rules for variable-only conjunctive queries.
#[cfg(feature = "canonical")]
#[proc_macro]
pub fn canonical_query(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::relational_plan)),
        cq_plan::canonical::compile,
    )
}

/// Source-order clause-prefix CQ lowering, selected independently of source shape.
#[cfg(feature = "prefix")]
#[proc_macro]
pub fn prefix_query(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::relational_plan)),
        cq_plan::prefix::compile,
    )
}

/// Graph planning, including YA reduction when the query is acyclic.
#[cfg(feature = "graph")]
#[proc_macro]
pub fn graph_query(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::relational_plan)),
        cq_plan::graph::compile,
    )
}

/// Alternative CQ-to-RA pass; the target remains RelationalPlan.
/// Compiler wiring belongs to Rust imports, not a prefix in the CQ language:
///
/// ```compile_fail
/// macro_rules! accept_ra { ($($tokens:tt)*) => {} }
/// mini_linq::planned_query! {
///     then accept_ra;
///     struct Query;
///     relation R(a:i64);
///     answer(x) :- R(x);
/// }
/// ```
#[cfg(feature = "defaults")]
#[proc_macro]
pub fn planned_query(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::planned_relational_plan)),
        cq_compose::compile_cq_planned,
    )
}

/// Select indexes on named RA results, preserving the written algebra.
#[cfg(feature = "result-indexes")]
#[proc_macro]
pub fn planned_relational_plan(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::materialized)),
        cq_index::result::compile,
    )
}

/// Indexed RA-to-staged-Rust pass; local holes return exact sets.
/// The default `collect_access` continuation combines terminal distinctness and
/// collection. An `access path;` override must satisfy that same set contract.
#[cfg(feature = "materialized")]
#[proc_macro]
pub fn materialized(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_physical::materialized::compile_materialized(&input.source, &input.access)
        },
    )
}

/// Lower indexed RA to an anonymous Rust closure returning an exact set.
/// Call it with one iterator per input declaration, in order. Each closure value
/// has Rust-inferred input types and ordinary capture semantics; no generated
/// item name enters the query expressions' scope.
#[cfg(feature = "materialized")]
#[proc_macro]
pub fn materialized_query(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_physical::materialized::compile_materialized_callable(&input.source, &input.access)
        },
    )
}

/// Select base-input indexes for the admitted linear RA domain.
#[cfg(feature = "input-indexes")]
#[proc_macro]
pub fn input_indexes(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::index_requirements)),
        cq_index::input::compile,
    )
}

/// Select indexes on named RA results without selecting a physical implementation.
#[cfg(feature = "result-indexes")]
#[proc_macro]
pub fn result_indexes(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::index_requirements)),
        cq_index::result::compile,
    )
}

/// Explicit base-input indexed physical implementation.
#[cfg(feature = "input-indexed")]
#[proc_macro]
pub fn input_indexed(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source| {
        cq_physical::input::compile_input_indexed(source, &syn::parse_quote!(mini_linq::pull))
    })
}

/// Explicit lazy physical implementation for a linear relational prefix.
#[cfg(feature = "lazy")]
#[proc_macro]
pub fn lazy_indexed(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source| {
        cq_physical::lazy::compile_lazy_indexed(source, &syn::parse_quote!(mini_linq::pull))
    })
}

/// Choose the default index policy and physical continuation together.
#[cfg(feature = "defaults")]
#[proc_macro]
pub fn relational_plan(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source| {
        cq_compose::expand_indexed(
            source,
            &syn::parse_quote!(mini_linq::input_indexed),
            &syn::parse_quote!(mini_linq::lazy_indexed),
        )
    })
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
        cq_pull::compile,
    )
}

/// Consume an Access plan into an exact `HashSet` through the existing pull
/// compiler and the independently selectable `collect_pipeline` continuation.
#[cfg(feature = "pull")]
#[proc_macro]
pub fn collect_access(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::collect_pipeline)),
        cq_pull::compile,
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
    feature = "hash-rows",
    feature = "sorted-rows",
    feature = "block-rows"
))]
mod native_input;
#[cfg(feature = "physical")]
mod physical_input;

/// The Access-to-Pipeline edge with a support-stream continuation.
#[cfg(feature = "pull")]
#[proc_macro]
pub fn stream_access(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::stream_pipeline)),
        cq_pull::compile,
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
    feature = "hash-rows",
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
                |input: &native_input::Input<cq_ir::physical_provider::Operation>| {
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
        |input: &native_input::Input<cq_ir::physical_provider::Operation>| {
            cq_access::candidates::scan::compile(input.scan_source()?)
        },
    )
}

#[cfg(feature = "prune-rows")]
native_entry!(
    prune_rows,
    cq_access::candidates::prune::compile,
    prune_options,
    "Prune through block metadata before opening selected payload."
);

#[cfg(feature = "exact-keys")]
native_entry!(
    exact_keys,
    cq_access::keys::exact::compile,
    exact_options,
    "Construct and probe an exact key resource."
);

#[cfg(feature = "zone-keys")]
native_entry!(
    zone_keys,
    cq_access::keys::zone::compile,
    zone_options,
    "Construct exact keys with a conservative range summary."
);

#[cfg(feature = "hash-witnesses")]
native_entry!(
    hash_witnesses,
    cq_access::witnesses::hash::compile,
    hash_witness_options,
    "Construct and probe hash witness buckets."
);

#[cfg(feature = "nested-witnesses")]
native_entry!(
    nested_witnesses,
    cq_access::witnesses::nested::compile,
    nested_witness_options,
    "Defer witness traversal until each probe."
);

#[cfg(feature = "hash-rows")]
native_entry!(
    hash_rows,
    cq_providers::hash::compile,
    hash_row_options,
    "Native hash-set storage operations."
);

#[cfg(feature = "sorted-rows")]
native_entry!(
    sorted_rows,
    cq_providers::sorted::compile,
    sorted_row_options,
    "Native sorted-set storage operations."
);

#[cfg(feature = "block-rows")]
native_entry!(
    block_rows,
    cq_providers::block::compile,
    block_options,
    "Borrowed block storage and selection through operation traits."
);

/// Compile configured RA into a plain query program.
#[cfg(feature = "physical")]
#[proc_macro]
pub fn physical_plan(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &physical_input::PhysicalInput| {
        cq_physical::direct::compile_physical(&input.source, &input.options()?)
    })
}

/// Compile configured RA into a callable query with ordinary Rust captures.
#[cfg(feature = "physical")]
#[proc_macro]
pub fn physical_query(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &physical_input::PhysicalInput| {
        cq_physical::direct::compile_physical_query(&input.source, &input.options()?)
    })
}

/// Compile configured RA into an explicitly observed query program.
#[cfg(feature = "physical")]
#[proc_macro]
pub fn observed_physical_plan(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &physical_input::PhysicalInput| {
        cq_physical::direct::compile_observed_physical(&input.source, &input.observed_options()?)
    })
}

/// Compile configured RA into an explicitly observed callable query.
#[cfg(feature = "physical")]
#[proc_macro]
pub fn observed_physical_query(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |input: &physical_input::PhysicalInput| {
        cq_physical::direct::compile_observed_physical_query(
            &input.source,
            &input.observed_options()?,
        )
    })
}
