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

use cq_ir::cq;
use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};

/// Own one workload query as real Rust macro input.
///
/// The default expansion exposes only the typed-CQ factory used by the catalog,
/// so the workload crate remains usable before students complete R1. The
/// `compiled-workloads` feature additionally emits the complete generated query
/// program through the same typed passes as [`mini_linq`].
#[proc_macro]
pub fn workload_query(input: TokenStream) -> TokenStream {
    let source_text = input.to_string();
    let source: cq::Module = match syn::parse(input) {
        Ok(source) => source,
        Err(error) => return error.into_compile_error().into(),
    };
    let catalog = quote! {
        pub(crate) const RUST_PATH: &str = file!();
        pub(crate) const SOURCE: &str = #source_text;

        pub(crate) fn module() -> ::syn::Result<::cq_ir::cq::Module> {
            ::syn::parse_str(SOURCE)
        }
    };

    #[cfg(feature = "compiled-workloads")]
    {
        let program = match cq_compiler::compile_program(&source) {
            Ok(program) => program,
            Err(error) => return error.into_compile_error().into(),
        };

        quote!(#catalog #program).into()
    }

    #[cfg(not(feature = "compiled-workloads"))]
    {
        let _ = source;
        catalog.into()
    }
}

/// Parse CQ syntax and route it through the typed CQ compiler pass.
#[proc_macro]
pub fn mini_linq(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::relational_plan)),
        cq_compiler::compile_cq,
    )
}

/// Canonical CQ-to-RA rules for variable-only conjunctive queries.
#[proc_macro]
pub fn canonical_query(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::relational_plan)),
        cq_compiler::compile_cq_canonical,
    )
}

/// Source-order clause-prefix CQ lowering, selected independently of source shape.
#[proc_macro]
pub fn prefix_query(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::relational_plan)),
        cq_compiler::compile_cq_prefix,
    )
}

/// Graph planning, including YA reduction when the query is acyclic.
#[proc_macro]
pub fn graph_query(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::relational_plan)),
        cq_compiler::compile_cq_graph,
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
#[proc_macro]
pub fn planned_query(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::planned_relational_plan)),
        cq_compiler::compile_cq_planned,
    )
}

/// Select indexes on named RA results, preserving the written algebra.
#[proc_macro]
pub fn planned_relational_plan(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::materialized)),
        cq_compiler::compile_result_indexes,
    )
}

/// Indexed RA-to-staged-Rust pass; local holes return exact sets.
/// The default `collect_access` continuation combines terminal distinctness and
/// collection. An `access path;` override must satisfy that same set contract.
#[proc_macro]
pub fn materialized(input: TokenStream) -> TokenStream {
    stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_compiler::compile_materialized_with(&input.source, &input.access)
        },
    )
}

/// Lower indexed RA to an anonymous Rust closure returning an exact set.
/// Call it with one iterator per input declaration, in order. Each closure value
/// has Rust-inferred input types and ordinary capture semantics; no generated
/// item name enters the query expressions' scope.
#[proc_macro]
pub fn materialized_query(input: TokenStream) -> TokenStream {
    stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_compiler::compile_materialized_callable(&input.source, &input.access)
        },
    )
}

/// Select base-input indexes for the admitted linear RA domain.
#[proc_macro]
pub fn input_indexes(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::index_requirements)),
        cq_compiler::compile_input_indexes,
    )
}

/// Select indexes on named RA results without selecting a physical implementation.
#[proc_macro]
pub fn result_indexes(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::index_requirements)),
        cq_compiler::compile_result_indexes,
    )
}

/// Explicit base-input indexed physical implementation.
#[proc_macro]
pub fn input_indexed(input: TokenStream) -> TokenStream {
    stage(input, None, cq_compiler::compile_input_indexed)
}

/// Explicit lazy physical implementation for a linear relational prefix.
#[proc_macro]
pub fn lazy_indexed(input: TokenStream) -> TokenStream {
    stage(input, None, cq_compiler::compile_lazy_indexed)
}

/// Parse RelationalPlan syntax and add its exact base-relation index overlay.
#[proc_macro]
pub fn relational_plan(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::index_requirements)),
        cq_compiler::compile_relational_plan,
    )
}

/// Parse IndexRequirements syntax and route it through the typed physical pass.
#[proc_macro]
pub fn index_requirements(input: TokenStream) -> TokenStream {
    stage(input, None, cq_compiler::compile_index_requirements)
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
#[proc_macro]
pub fn pull(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::iterator_pipeline)),
        cq_compiler::compile_pull,
    )
}

/// Consume an Access plan into an exact `HashSet` through the existing pull
/// compiler and the independently selectable `collect_pipeline` continuation.
#[proc_macro]
pub fn collect_access(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::collect_pipeline)),
        cq_compiler::compile_pull,
    )
}

/// Consume a named pipeline into its exact `HashSet`, folding terminal
/// distinctness into the set sink. The source language is IteratorPipeline.
#[proc_macro]
pub fn collect_pipeline(input: TokenStream) -> TokenStream {
    stage(input, None, cq_compiler::compile_collect_pipeline)
}

/// Lower an explicit named physical plan to its actual ordinary-Rust AST.
///
/// The compiler writes one standard iterator-adapter local per named operator.
/// This proc-macro performs no further code generation: quoting the returned
/// [`syn::Expr`] is its direct `ToTokens` representation.
#[proc_macro]
pub fn iterator_pipeline(input: TokenStream) -> TokenStream {
    stage(input, None, cq_compiler::compile_iterator_pipeline)
}

mod native_input;
mod physical_input;

mod kw {
    syn::custom_keyword!(access);
}

fn stage<S: Parse, T: ToTokens>(
    input: TokenStream,
    next: Option<syn::Path>,
    lower: impl FnOnce(&S) -> syn::Result<T>,
) -> TokenStream {
    let source = syn::parse_macro_input!(input as S);
    match lower(&source) {
        Ok(target) => match next {
            Some(next) => quote!(#next! { #target }).into(),
            None => quote!(#target).into(),
        },
        Err(error) => error.into_compile_error().into(),
    }
}

struct AccessInput<S> {
    access: syn::Path,
    source: S,
}
impl<S: Parse> Parse for AccessInput<S> {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let access = if input.peek(kw::access) {
            input.parse::<kw::access>()?;
            let path = input.parse()?;
            input.parse::<syn::Token![;]>()?;
            path
        } else {
            syn::parse_quote!(mini_linq::collect_access)
        };
        Ok(Self {
            access,
            source: input.parse()?,
        })
    }
}
/// Configured RA -> Rust with independently selected native Access leaves.
#[proc_macro]
pub fn physical_plan(input: TokenStream) -> TokenStream {
    stage(input, None, |input: &physical_input::PhysicalInput| {
        cq_compiler::compile_physical(&input.source, &input.options()?)
    })
}

/// The same checked Access-to-Pipeline lowering with a support-stream endpoint.
#[proc_macro]
pub fn stream_access(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::stream_pipeline)),
        cq_compiler::compile_pull,
    )
}

/// Enumerate the support of a set pipeline; the consumer owns distinctness.
#[proc_macro]
pub fn stream_pipeline(input: TokenStream) -> TokenStream {
    stage(input, None, cq_compiler::compile_stream_pipeline)
}

macro_rules! physical_entry {
    ($name:ident, $compile:ident, $source:ty, $options:ident, $doc:literal) => {
        #[doc = $doc]
        #[proc_macro]
        pub fn $name(input: TokenStream) -> TokenStream {
            stage(input, None, |input: &native_input::Input<$source>| {
                cq_compiler::$compile(input.source(), &input.$options()?)
            })
        }
    };
}
physical_entry!(
    hash_rows,
    compile_hash_provider,
    cq_ir::physical_provider::Operation,
    provider_options,
    "Native HashSet physical operations."
);
physical_entry!(
    sorted_rows,
    compile_sorted_provider,
    cq_ir::physical_provider::Operation,
    provider_options,
    "Native BTreeSet physical operations."
);
physical_entry!(
    block_rows,
    compile_block_provider,
    cq_ir::physical_provider::Operation,
    provider_options,
    "Borrowed block access and independently selected masks through storage traits."
);
physical_entry!(
    scan_rows,
    compile_scan_rows,
    cq_ir::physical_provider::Operation,
    access_options,
    "Enumerate current candidate handles."
);
physical_entry!(
    prune_rows,
    compile_prune_rows,
    cq_ir::physical_provider::Operation,
    access_options,
    "Reject blocks through metadata before opening candidate payload."
);
physical_entry!(
    exact_keys,
    compile_exact_keys,
    cq_ir::physical_provider::Operation,
    access_options,
    "Build and test exact key membership."
);
physical_entry!(
    zone_keys,
    compile_zone_keys,
    cq_ir::physical_provider::Operation,
    access_options,
    "Build exact keys and their conservative coordinate summary."
);
physical_entry!(
    hash_witnesses,
    compile_hash_witnesses,
    cq_ir::physical_provider::Operation,
    access_options,
    "Build and probe exact witness buckets."
);
physical_entry!(
    nested_witnesses,
    compile_nested_witnesses,
    cq_ir::physical_provider::Operation,
    access_options,
    "Defer witness traversal until each probe."
);
