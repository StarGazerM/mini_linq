//! User-facing macros for the MiniLinq teaching compiler.
//!
//! Each macro parses its exact source IR and routes that typed value to the
//! corresponding compiler pass. Syntax errors become Rust compile errors;
//! semantic well-formedness belongs to the configurable pass contracts.

use cq_ir::{cq, index_requirements, iterator_pipeline, relational_plan, rust_access_plan};
use proc_macro::TokenStream;
use quote::quote;

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
        let logical = match cq_compiler::compile_cq(&source) {
            Ok(logical) => logical,
            Err(error) => return error.into_compile_error().into(),
        };
        let requirements = match cq_compiler::compile_relational_plan(&logical) {
            Ok(requirements) => requirements,
            Err(error) => return error.into_compile_error().into(),
        };
        let program = match cq_compiler::compile_index_requirements(&requirements) {
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
    let source: cq::Module = match syn::parse(input) {
        Ok(source) => source,
        Err(error) => return error.into_compile_error().into(),
    };
    let target = match cq_compiler::compile_cq(&source) {
        Ok(target) => target,
        Err(error) => return error.into_compile_error().into(),
    };

    quote!(::mini_linq::relational_plan! { #target }).into()
}

/// Alternative CQ-to-RA pass; the target remains RelationalPlan.
#[proc_macro]
pub fn planned_query(input: TokenStream) -> TokenStream {
    let source = syn::parse_macro_input!(input as cq::Module);
    match cq_compiler::compile_cq_planned(&source) {
        Ok(target) => quote!(::mini_linq::planned_relational_plan! { #target }).into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Select indexes on named RA results, preserving the written algebra.
#[proc_macro]
pub fn planned_relational_plan(input: TokenStream) -> TokenStream {
    let source = syn::parse_macro_input!(input as relational_plan::Module);
    match cq_compiler::compile_result_indexes(&source) {
        Ok(target) => quote!(::mini_linq::materialized! { #target }).into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Indexed RA-to-staged-Rust pass; local holes use the existing pull route.
#[proc_macro]
pub fn materialized(input: TokenStream) -> TokenStream {
    let source = syn::parse_macro_input!(input as index_requirements::Module);
    match cq_compiler::compile_materialized(&source) {
        Ok(target) => quote!(#target).into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Parse RelationalPlan syntax and add its exact base-relation index overlay.
#[proc_macro]
pub fn relational_plan(input: TokenStream) -> TokenStream {
    let source: relational_plan::Module = match syn::parse(input) {
        Ok(source) => source,
        Err(error) => return error.into_compile_error().into(),
    };
    let target = match cq_compiler::compile_relational_plan(&source) {
        Ok(target) => target,
        Err(error) => return error.into_compile_error().into(),
    };

    quote!(::mini_linq::index_requirements! { #target }).into()
}

/// Parse IndexRequirements syntax and route it through the typed physical pass.
#[proc_macro]
pub fn index_requirements(input: TokenStream) -> TokenStream {
    let source: index_requirements::Module = match syn::parse(input) {
        Ok(source) => source,
        Err(error) => return error.into_compile_error().into(),
    };
    let target = match cq_compiler::compile_index_requirements(&source) {
        Ok(target) => target,
        Err(error) => return error.into_compile_error().into(),
    };

    quote!(#target).into()
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
    let source: rust_access_plan::Plan = match syn::parse(input) {
        Ok(source) => source,
        Err(error) => return error.into_compile_error().into(),
    };
    if let Err(error) = rust_access_plan::contract::check(&source) {
        return error.into_compile_error().into();
    }
    let target = cq_compiler::compile_pull(&source);

    quote!(::mini_linq::iterator_pipeline! { #target }).into()
}

/// Lower an explicit named physical plan to its actual ordinary-Rust AST.
///
/// The compiler writes one standard iterator-adapter local per named operator.
/// This proc-macro performs no further code generation: quoting the returned
/// [`syn::Expr`] is its direct `ToTokens` representation.
#[proc_macro]
pub fn iterator_pipeline(input: TokenStream) -> TokenStream {
    let source: iterator_pipeline::Pipeline = match syn::parse(input) {
        Ok(source) => source,
        Err(error) => return error.into_compile_error().into(),
    };
    if let Err(error) = iterator_pipeline::contract::check(&source) {
        return error.into_compile_error().into();
    }
    let target = cq_compiler::compile_iterator_pipeline(&source);

    quote!(#target).into()
}
